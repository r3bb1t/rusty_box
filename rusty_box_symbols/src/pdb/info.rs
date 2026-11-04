//! The PDB info stream (stream 1; LLVM, "The PDB Info Stream"): version,
//! signature, age, GUID, then the named-stream map, a hash table from a
//! name (an offset into a string buffer) to a stream index.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use std::collections::BTreeMap;

use super::hash_table::{HashTable, TableLimits};
use super::{short, PdbError};
use crate::identity::Guid;
use crate::read::{Bytes, Cursor, ReadError};

/// A hash table's capacity past this is not a real PDB's (link.exe's
/// named-stream maps hold a handful).
const MAX_CAPACITY: u32 = 1 << 16;
const MAX_NAME_BYTES: usize = 1024;

/// The fixed head of the info stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoHead {
    pub version: u32,
    pub age: u32,
    pub guid: Guid,
}

/// Version, signature, age and GUID.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
pub fn decode_head(stream: Bytes<'_>) -> Result<InfoHead, ReadError> {
    Ok(InfoHead {
        version: stream.u32_le(0)?,
        age: stream.u32_le(8)?,
        guid: Guid::from_le_bytes(stream.array::<16>(12)?),
    })
}

const WHAT: &str = "named-stream map";

/// The named-stream map: a string buffer, then a hash table from a name's
/// offset in it to a stream index.
struct NamedStreamMap<'a> {
    names: Bytes<'a>,
    table: HashTable<'a>,
}

/// One name of the map, with its stream.
struct NamedStream<'a> {
    name: &'a [u8],
    index: u32,
}

impl<'a> NamedStreamMap<'a> {
    fn open(stream: &'a [u8]) -> Result<NamedStreamMap<'a>, PdbError> {
        let mut cursor = Cursor::new(Bytes::new(stream), 28);
        let names_size = cursor.u32_le().map_err(|source| short(WHAT, source))? as usize;
        let names = Bytes::new(cursor.bytes(names_size).map_err(|source| short(WHAT, source))?);
        let table = HashTable::open(cursor, TableLimits { what: WHAT, max_capacity: MAX_CAPACITY })?;
        Ok(NamedStreamMap { names, table })
    }

    /// The next name that reads, with its stream; `None` past the last.
    fn next_stream(&mut self) -> Result<Option<NamedStream<'a>>, PdbError> {
        while let Some(entry) = self.table.next_entry(4)? {
            let index = entry.value.u32_le(0).map_err(|source| short(WHAT, source))?;
            if let Ok(name) = self.names.cstr(entry.key as usize, MAX_NAME_BYTES) {
                return Ok(Some(NamedStream { name, index }));
            }
        }
        Ok(None)
    }
}

/// The stream index the named-stream map gives `wanted` (`/names`).
pub fn named_stream(stream: &[u8], wanted: &[u8]) -> Result<Option<u32>, PdbError> {
    let mut map = NamedStreamMap::open(stream)?;
    while let Some(named) = map.next_stream()? {
        if named.name == wanted {
            return Ok(Some(named.index));
        }
    }
    Ok(None)
}

/// Every name of the named-stream map with its stream index; a name met
/// again keeps its first stream.
pub fn named_streams(stream: &[u8]) -> Result<BTreeMap<Vec<u8>, u32>, PdbError> {
    let mut map = NamedStreamMap::open(stream)?;
    let mut by_name = BTreeMap::new();
    while let Some(named) = map.next_stream()? {
        let mut name = Vec::new();
        name.try_reserve_exact(named.name.len())
            .map_err(|_| PdbError::Allocation { what: "stream names", bytes: named.name.len() as u64 })?;
        name.extend_from_slice(named.name);
        by_name.entry(name).or_insert(named.index);
    }
    Ok(by_name)
}
