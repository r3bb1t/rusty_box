//! Line-number programs (DWARF 5 §6.2; §6.2.4 and Appendix F for the
//! version 2 to 4 header): the header's directories and files, then the
//! state machine's rows. A row's file is an index into `files`, whose
//! entries are full paths joined from their directory and the unit's
//! compilation directory.

use super::forms::{decode_value, Encoding, Value};
use super::{push, short, DwarfError, Format};
use crate::read::{Bytes, Cursor};

const DW_LNS_COPY: u8 = 1;
const DW_LNS_ADVANCE_PC: u8 = 2;
const DW_LNS_ADVANCE_LINE: u8 = 3;
const DW_LNS_SET_FILE: u8 = 4;
const DW_LNS_CONST_ADD_PC: u8 = 8;
const DW_LNS_FIXED_ADVANCE_PC: u8 = 9;
const DW_LNE_END_SEQUENCE: u8 = 1;
const DW_LNE_SET_ADDRESS: u8 = 2;
const DW_LNCT_PATH: u64 = 1;
const DW_LNCT_DIRECTORY_INDEX: u64 = 2;
/// Directories or files in one header past this are not a real program's.
const MAX_ENTRIES: u64 = 1 << 16;
/// Entry formats of one header (DWARF 5 defines five content types).
const MAX_FORMATS: u8 = 32;
/// Rows of one program; a large unit has a few hundred thousand.
const MAX_ROWS: usize = 1 << 24;
const MAX_NAME_BYTES: usize = 4096;

/// Whether a row ends its sequence (the address one past its last byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sequence {
    Continues,
    Ends,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub address: u64,
    pub file: u64,
    pub line: u32,
    pub sequence: Sequence,
}

/// A program's files (full paths, by index) and rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineProgram {
    pub files: Vec<String>,
    pub rows: Vec<Row>,
}

/// What a unit tells its line program.
pub struct LineContext {
    pub encoding: Encoding,
    pub comp_dir: String,
    pub unit_name: String,
}

/// Whether `path` is absolute on a Unix or a Windows build machine.
fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || path.starts_with('\\') || path.as_bytes().get(1) == Some(&b':')
}

/// `name` under `directory` unless `name` is already absolute.
fn joined(directory: &str, name: &str) -> String {
    if is_absolute(name) || directory.is_empty() {
        return name.to_owned();
    }
    let separator = if directory.ends_with('/') || directory.ends_with('\\') { "" } else { "/" };
    format!("{directory}{separator}{name}")
}

/// One entry format of a version 5 header.
#[derive(Clone, Copy)]
struct EntryFormat {
    content: u64,
    form: u64,
}

/// A version 5 header entry: its path and directory index.
#[derive(Clone, Default)]
struct HeaderEntry {
    path: String,
    directory: u64,
}

/// Reads a version 5 header entry list (§6.2.4.1, items 14 to 20).
fn entries_v5<F>(cursor: &mut Cursor<'_>, section: Bytes<'_>, encoding: &Encoding, text: &F) -> Result<Vec<HeaderEntry>, DwarfError>
where
    F: Fn(Value<'_>) -> Option<String>,
{
    let what = "line program header";
    let format_count = cursor.u8().map_err(|source| short(what, source))?;
    if format_count > MAX_FORMATS {
        return Err(DwarfError::TooMany { what: "entry formats", count: u64::from(format_count) });
    }
    let mut formats = Vec::new();
    for _ in 0..format_count {
        let content = cursor.uleb128().map_err(|source| short(what, source))?;
        let form = cursor.uleb128().map_err(|source| short(what, source))?;
        push(&mut formats, EntryFormat { content, form }, "entry formats")?;
    }
    let count = cursor.uleb128().map_err(|source| short(what, source))?;
    if count > MAX_ENTRIES {
        return Err(DwarfError::TooMany { what: "line header entries", count });
    }
    let mut entries = Vec::new();
    entries.try_reserve_exact(count as usize).map_err(|_| DwarfError::Allocation { what: "line header entries" })?;
    for _ in 0..count {
        let mut entry = HeaderEntry::default();
        for format in &formats {
            let decoded = decode_value(section, cursor.position(), format.form, 0, encoding)?;
            cursor
                .skip(decoded.next.saturating_sub(cursor.position()))
                .map_err(|source| short(what, source))?;
            match format.content {
                DW_LNCT_PATH => entry.path = text(decoded.value).unwrap_or_default(),
                DW_LNCT_DIRECTORY_INDEX => {
                    if let Value::Unsigned(index) = decoded.value {
                        entry.directory = index;
                    }
                }
                _ => {}
            }
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// The program at `at` in `.debug_line`. `text` resolves a string form of
/// a version 5 header (inline, `.debug_str`, `.debug_line_str`).
pub fn parse_line_program<F>(section: Bytes<'_>, at: usize, unit: &LineContext, text: F) -> Result<LineProgram, DwarfError>
where
    F: Fn(Value<'_>) -> Option<String>,
{
    let what = "line program header";
    let mut cursor = Cursor::new(section, at);
    let first = cursor.u32_le().map_err(|source| short(what, source))?;
    let format = if first == 0xFFFF_FFFF { Format::Dwarf64 } else { Format::Dwarf32 };
    let length = match format {
        Format::Dwarf64 => cursor.u64_le().map_err(|source| short(what, source))?,
        Format::Dwarf32 => u64::from(first),
    };
    let program_end = cursor
        .position()
        .checked_add(usize::try_from(length).map_err(|_| DwarfError::TooMany { what: "line program bytes", count: length })?)
        .ok_or(DwarfError::TooMany { what: "line program bytes", count: length })?;
    section.slice(at, program_end.saturating_sub(at)).map_err(|source| short("line program", source))?;
    let version = cursor.u16_le().map_err(|source| short(what, source))?;
    if !(2..=5).contains(&version) {
        return Err(DwarfError::BadVersion { at, version });
    }
    let mut address_size = unit.encoding.address_size;
    if version >= 5 {
        address_size = cursor.u8().map_err(|source| short(what, source))?;
        cursor.u8().map_err(|source| short(what, source))?;
    }
    let header_length = match format {
        Format::Dwarf64 => cursor.u64_le().map_err(|source| short(what, source))?,
        Format::Dwarf32 => u64::from(cursor.u32_le().map_err(|source| short(what, source))?),
    };
    let program_at = cursor
        .position()
        .checked_add(usize::try_from(header_length).map_err(|_| DwarfError::TooMany { what: "line header bytes", count: header_length })?)
        .ok_or(DwarfError::TooMany { what: "line header bytes", count: header_length })?;
    let minimum_instruction_length = u64::from(cursor.u8().map_err(|source| short(what, source))?);
    if version >= 4 {
        cursor.u8().map_err(|source| short(what, source))?;
    }
    cursor.u8().map_err(|source| short(what, source))?;
    let line_base = i64::from(cursor.u8().map_err(|source| short(what, source))? as i8);
    let line_range = cursor.u8().map_err(|source| short(what, source))?;
    let opcode_base = cursor.u8().map_err(|source| short(what, source))?;
    if line_range == 0 {
        return Err(DwarfError::BadReference { offset: at as u64 });
    }
    let mut standard_lengths = [0u8; 256];
    for index in 1..opcode_base {
        let length = cursor.u8().map_err(|source| short(what, source))?;
        if let Some(slot) = standard_lengths.get_mut(usize::from(index)) {
            *slot = length;
        }
    }
    let encoding = Encoding { version, format, address_size };
    let mut program = LineProgram::default();
    if version >= 5 {
        let directories = entries_v5(&mut cursor, section, &encoding, &text)?;
        let files = entries_v5(&mut cursor, section, &encoding, &text)?;
        let comp_dir = directories.first().map_or(unit.comp_dir.as_str(), |directory| directory.path.as_str());
        for file in files {
            let directory = usize::try_from(file.directory)
                .ok()
                .and_then(|index| directories.get(index))
                .map_or("", |directory| directory.path.as_str());
            push(&mut program.files, joined(comp_dir, &joined(directory, &file.path)), "file names")?;
        }
    } else {
        let mut directories = Vec::new();
        push(&mut directories, unit.comp_dir.clone(), "include directories")?;
        loop {
            let directory = cursor.cstr(MAX_NAME_BYTES).map_err(|source| short(what, source))?;
            if directory.is_empty() {
                break;
            }
            if directories.len() as u64 > MAX_ENTRIES {
                return Err(DwarfError::TooMany { what: "include directories", count: directories.len() as u64 });
            }
            push(&mut directories, String::from_utf8_lossy(directory).into_owned(), "include directories")?;
        }
        push(&mut program.files, joined(&unit.comp_dir, &unit.unit_name), "file names")?;
        loop {
            let name = cursor.cstr(MAX_NAME_BYTES).map_err(|source| short(what, source))?;
            if name.is_empty() {
                break;
            }
            let directory = cursor.uleb128().map_err(|source| short(what, source))?;
            cursor.uleb128().map_err(|source| short(what, source))?;
            cursor.uleb128().map_err(|source| short(what, source))?;
            if program.files.len() as u64 > MAX_ENTRIES {
                return Err(DwarfError::TooMany { what: "file names", count: program.files.len() as u64 });
            }
            let directory = usize::try_from(directory)
                .ok()
                .and_then(|index| directories.get(index))
                .map_or("", String::as_str);
            let name = String::from_utf8_lossy(name);
            push(&mut program.files, joined(&unit.comp_dir, &joined(directory, &name)), "file names")?;
        }
    }

    // The state machine (§6.2.2); every opcode consumes at least one byte.
    let mut cursor = Cursor::new(section, program_at);
    let mut address = 0u64;
    let mut file = 1u64;
    let mut line = 1i64;
    let emit = |program: &mut LineProgram, address: u64, file: u64, line: i64, sequence: Sequence| -> Result<(), DwarfError> {
        if program.rows.len() >= MAX_ROWS {
            return Err(DwarfError::TooMany { what: "line rows", count: program.rows.len() as u64 });
        }
        program.rows.try_reserve(1).map_err(|_| DwarfError::Allocation { what: "line rows" })?;
        program.rows.push(Row { address, file, line: u32::try_from(line).unwrap_or(0), sequence });
        Ok(())
    };
    let advance = |address: u64, operations: u64| address.wrapping_add(minimum_instruction_length.wrapping_mul(operations));
    while cursor.position() < program_end {
        let opcode = cursor.u8().map_err(|source| short("line program", source))?;
        if opcode >= opcode_base {
            let adjusted = opcode.wrapping_sub(opcode_base);
            address = advance(address, u64::from(adjusted.checked_div(line_range).unwrap_or(0)));
            line = line.wrapping_add(line_base.wrapping_add(i64::from(adjusted.checked_rem(line_range).unwrap_or(0))));
            emit(&mut program, address, file, line, Sequence::Continues)?;
            continue;
        }
        match opcode {
            0 => {
                let length = cursor.uleb128().map_err(|source| short("line program", source))?;
                let start = cursor.position();
                let sub = cursor.u8().map_err(|source| short("line program", source))?;
                match sub {
                    DW_LNE_END_SEQUENCE => {
                        emit(&mut program, address, file, line, Sequence::Ends)?;
                        address = 0;
                        file = 1;
                        line = 1;
                    }
                    DW_LNE_SET_ADDRESS => {
                        address = match address_size {
                            4 => u64::from(cursor.u32_le().map_err(|source| short("line program", source))?),
                            _ => cursor.u64_le().map_err(|source| short("line program", source))?,
                        };
                    }
                    _ => {}
                }
                let end = start.checked_add(usize::try_from(length).unwrap_or(usize::MAX)).ok_or(DwarfError::TooMany { what: "line program bytes", count: length })?;
                cursor.skip(end.saturating_sub(cursor.position())).map_err(|source| short("line program", source))?;
            }
            DW_LNS_COPY => emit(&mut program, address, file, line, Sequence::Continues)?,
            DW_LNS_ADVANCE_PC => {
                let operations = cursor.uleb128().map_err(|source| short("line program", source))?;
                address = advance(address, operations);
            }
            DW_LNS_ADVANCE_LINE => {
                line = line.wrapping_add(cursor.sleb128().map_err(|source| short("line program", source))?);
            }
            DW_LNS_SET_FILE => file = cursor.uleb128().map_err(|source| short("line program", source))?,
            DW_LNS_CONST_ADD_PC => {
                let adjusted = 255u8.wrapping_sub(opcode_base);
                address = advance(address, u64::from(adjusted.checked_div(line_range).unwrap_or(0)));
            }
            DW_LNS_FIXED_ADVANCE_PC => {
                address = address.wrapping_add(u64::from(cursor.u16_le().map_err(|source| short("line program", source))?));
            }
            other => {
                // Every other standard opcode: skip its LEB128 operands.
                let operands = standard_lengths.get(usize::from(other)).copied().unwrap_or(0);
                for _ in 0..operands {
                    cursor.uleb128().map_err(|source| short("line program", source))?;
                }
            }
        }
    }
    Ok(program)
}
