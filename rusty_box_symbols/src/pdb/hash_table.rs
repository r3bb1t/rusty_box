//! A PDB's serialized hash table (LLVM `HashTable::load`): size, capacity,
//! the present and deleted bit vectors (each a word count, then
//! little-endian 32-bit words), then for each present bucket, by bucket, a
//! u32 key and a value of fixed size. The named-stream map and
//! `/src/headerblock` are written so.

#[cfg(all(test, not(debug_assertions)))]
use no_panic::no_panic;

use super::{short, PdbError};
use crate::read::{Bytes, Cursor};

/// Whether bit `slot` is set in a little-endian bit vector of 32-bit words.
#[cfg_attr(all(test, not(debug_assertions)), no_panic)]
fn bit_set(words: Bytes<'_>, slot: u32) -> bool {
    let word_at = (slot.wrapping_shr(5) as usize).wrapping_mul(4);
    let word = words.u32_le(word_at).unwrap_or(0);
    word.wrapping_shr(slot & 31) & 1 == 1
}

/// What a table is, for its errors, and the most buckets it may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableLimits {
    pub what: &'static str,
    pub max_capacity: u32,
}

/// One present bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashEntry<'a> {
    pub key: u32,
    pub value: Bytes<'a>,
}

/// A table being read, bucket by bucket.
pub struct HashTable<'a> {
    cursor: Cursor<'a>,
    present: Bytes<'a>,
    capacity: u32,
    /// The next bucket to look at.
    slot: u32,
    what: &'static str,
}

impl<'a> HashTable<'a> {
    /// The table at `cursor`. It holds as many entries as its present-bit
    /// vector has bits set below its capacity, and at most its capacity.
    pub fn open(mut cursor: Cursor<'a>, limits: TableLimits) -> Result<HashTable<'a>, PdbError> {
        let what = limits.what;
        let entries = cursor.u32_le().map_err(|source| short(what, source))?;
        let capacity = cursor.u32_le().map_err(|source| short(what, source))?;
        if capacity > limits.max_capacity || entries > capacity {
            return Err(PdbError::TooMany { what, count: u64::from(capacity) });
        }
        let present_words = cursor.u32_le().map_err(|source| short(what, source))?;
        if present_words > capacity.wrapping_shr(5).wrapping_add(1) {
            return Err(PdbError::TooMany { what, count: u64::from(present_words) });
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
        Ok(HashTable { cursor, present, capacity, slot: 0, what })
    }

    /// The next present bucket, its value `value_len` bytes; `None` past
    /// the last.
    pub fn next_entry(&mut self, value_len: usize) -> Result<Option<HashEntry<'a>>, PdbError> {
        while self.slot < self.capacity {
            let slot = self.slot;
            self.slot = slot.saturating_add(1);
            if !bit_set(self.present, slot) {
                continue;
            }
            let what = self.what;
            let key = self.cursor.u32_le().map_err(|source| short(what, source))?;
            let value = Bytes::new(self.cursor.bytes(value_len).map_err(|source| short(what, source))?);
            return Ok(Some(HashEntry { key, value }));
        }
        Ok(None)
    }
}
