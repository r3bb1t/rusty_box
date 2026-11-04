//! The `/src/headerblock` named stream (LLVM `InjectedSourceStream`):
//! source files embedded in the PDB. A 64-byte header (version, the
//! stream's size, a file time, an age, padding), then a hash table from a
//! file's `/names` offset to a 40-byte entry (LLVM `SrcHeaderBlockEntry`):
//! its size, version, CRC, file size, file, object and virtual names (each a
//! `/names` offset), compression, whether it is virtual, padding. Each
//! file's bytes are the named stream `/src/files/` + its virtual name.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use super::hash_table::{HashTable, TableLimits};
use super::{short, PdbError};
use crate::read::{Bytes, Cursor};

/// The version of the header and of each entry (LLVM `SrcVerOne`).
pub const SOURCE_BLOCK_VERSION: u32 = 19_980_827;
const HEADER_BYTES: usize = 64;
const ENTRY_BYTES: usize = 40;
/// Embedded files past this are not read.
pub const MAX_EMBEDDED_FILES: u32 = 4096;

/// What an entry says of its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceEntry {
    /// The file's name, a `/names` offset.
    pub file: u32,
    /// The name its stream is found by, a `/names` offset.
    pub virtual_file: u32,
    /// 0 when the bytes are stored as they are.
    pub compression: u8,
}

/// One entry, when it is whole, its size is 40 and its version is
/// `SOURCE_BLOCK_VERSION` (LLVM refuses any other).
#[must_use]
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_entry(entry: Bytes<'_>) -> Option<SourceEntry> {
    entry.slice(0, ENTRY_BYTES).ok()?;
    let size = entry.u32_le(0).ok()?;
    let version = entry.u32_le(4).ok()?;
    if size as usize != ENTRY_BYTES || version != SOURCE_BLOCK_VERSION {
        return None;
    }
    Some(SourceEntry {
        file: entry.u32_le(16).ok()?,
        virtual_file: entry.u32_le(24).ok()?,
        compression: entry.u8(28).ok()?,
    })
}

/// Every entry of the stream, in bucket order.
pub fn entries(stream: &[u8]) -> Result<Vec<SourceEntry>, PdbError> {
    let what = "source header block";
    let bytes = Bytes::new(stream);
    bytes.slice(0, HEADER_BYTES).map_err(|source| short(what, source))?;
    let version = bytes.u32_le(0).map_err(|source| short(what, source))?;
    if version != SOURCE_BLOCK_VERSION {
        return Err(PdbError::BadRecord { what, at: 0 });
    }
    let mut table =
        HashTable::open(Cursor::new(bytes, HEADER_BYTES), TableLimits { what, max_capacity: MAX_EMBEDDED_FILES })?;
    let mut found = Vec::new();
    while let Some(entry) = table.next_entry(ENTRY_BYTES)? {
        let at = found.len().saturating_mul(ENTRY_BYTES);
        let decoded = decode_entry(entry.value).ok_or(PdbError::BadRecord { what: "embedded file entry", at })?;
        crate::tables::push(&mut found, decoded, "embedded files")
            .map_err(|_| PdbError::Allocation { what: "embedded files", bytes: ENTRY_BYTES as u64 })?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::Bytes;

    /// A 40-byte entry: size 40, the version, CRC, file size 7, file name
    /// 0x10, object name 0, virtual name 0x20, compression 1, not virtual.
    fn entry() -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [40u32, SOURCE_BLOCK_VERSION, 0xABCD, 7, 0x10, 0, 0x20] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn an_entry_reads_its_names_and_compression() {
        assert_eq!(
            decode_entry(Bytes::new(&entry())),
            Some(SourceEntry { file: 0x10, virtual_file: 0x20, compression: 1 })
        );
    }

    #[test]
    fn an_entry_cut_short_or_of_another_shape_does_not_read() {
        let whole = entry();
        for len in [0, 4, 28, 29, 39] {
            assert_eq!(decode_entry(Bytes::new(&whole[..len])), None, "cut to {len}");
        }
        let mut sized = entry();
        sized[0] = 44;
        assert_eq!(decode_entry(Bytes::new(&sized)), None, "an entry size other than 40");
        let mut versioned = entry();
        versioned[4] = 0;
        assert_eq!(decode_entry(Bytes::new(&versioned)), None, "another version");
    }
}
