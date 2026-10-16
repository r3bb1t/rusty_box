//! The PDB info stream (stream 1; LLVM, "The PDB Info Stream"): version,
//! signature, age, GUID, then the named-stream map, a hash table from a
//! name (an offset into a string buffer) to a stream index.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

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

/// Whether bit `slot` is set in a little-endian bit vector of 32-bit words.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn bit_set(words: Bytes<'_>, slot: u32) -> bool {
    let word_at = (slot.wrapping_shr(5) as usize).wrapping_mul(4);
    let word = words.u32_le(word_at).unwrap_or(0);
    word.wrapping_shr(slot & 31) & 1 == 1
}

/// The stream index the named-stream map gives `wanted` (`/names`). The
/// map holds as many entries as its present-bit vector has set bits, and
/// at most its capacity.
pub fn named_stream(stream: &[u8], wanted: &[u8]) -> Result<Option<u32>, PdbError> {
    let bytes = Bytes::new(stream);
    let what = "named-stream map";
    let mut cursor = Cursor::new(bytes, 28);
    let names_size = cursor.u32_le().map_err(|source| short(what, source))? as usize;
    let names = Bytes::new(cursor.bytes(names_size).map_err(|source| short(what, source))?);
    let entries = cursor.u32_le().map_err(|source| short(what, source))?;
    let capacity = cursor.u32_le().map_err(|source| short(what, source))?;
    if capacity > MAX_CAPACITY || entries > capacity {
        return Err(PdbError::TooMany { what: "named streams", count: u64::from(capacity) });
    }
    let present_words = cursor.u32_le().map_err(|source| short(what, source))?;
    if present_words > capacity.wrapping_shr(5).wrapping_add(1) {
        return Err(PdbError::TooMany { what: "named-stream bit words", count: u64::from(present_words) });
    }
    let present = Bytes::new(
        cursor
            .bytes((present_words as usize).saturating_mul(4))
            .map_err(|source| short(what, source))?,
    );
    let deleted_words = cursor.u32_le().map_err(|source| short(what, source))?;
    cursor
        .skip((deleted_words as usize).saturating_mul(4))
        .map_err(|source| short(what, source))?;
    for slot in 0..capacity {
        if !bit_set(present, slot) {
            continue;
        }
        let key = cursor.u32_le().map_err(|source| short(what, source))?;
        let value = cursor.u32_le().map_err(|source| short(what, source))?;
        if names.cstr(key as usize, MAX_NAME_BYTES).is_ok_and(|name| name == wanted) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
