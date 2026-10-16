//! The entries of one unit (DWARF 5 §7.5.2): each an abbreviation code (0
//! closes a list of siblings) and its attributes' values. [`UnitContext`]
//! steps from entry to entry without allocating, and reads an entry's
//! attribute by walking its abbreviation's specifications.

use super::abbrev::{parse_abbreviations, Abbreviations, Children};
use super::forms::{decode_value, Encoding, Value};
use super::unit::UnitHeader;
use super::{short, DwarfError};
use crate::read::Bytes;

/// An entry: where it is, what it is, where its attributes start, and
/// where the next entry starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub offset: usize,
    pub tag: u64,
    pub children: Children,
    /// Its abbreviation's index in the unit's table.
    pub abbreviation: usize,
    pub attributes_at: usize,
    pub next: usize,
}

/// One step through a unit's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The end of a list of siblings.
    Null { next: usize },
    Entry(Entry),
}

/// A unit, its section and its abbreviations.
#[derive(Clone, Copy)]
pub struct UnitContext<'a> {
    pub section: Bytes<'a>,
    pub header: UnitHeader,
    pub abbreviations: &'a Abbreviations,
    pub encoding: Encoding,
}

/// The abbreviation table `header` names, from `.debug_abbrev`.
pub fn unit_abbreviations(header: &UnitHeader, abbrev_section: Bytes<'_>) -> Result<Abbreviations, DwarfError> {
    let at = usize::try_from(header.abbrev_offset)
        .map_err(|_| DwarfError::TooMany { what: "abbreviation offset", count: header.abbrev_offset })?;
    parse_abbreviations(abbrev_section, at)
}

impl<'a> UnitContext<'a> {
    #[must_use]
    pub fn new(section: Bytes<'a>, header: UnitHeader, abbreviations: &'a Abbreviations) -> UnitContext<'a> {
        UnitContext {
            section,
            header,
            abbreviations,
            encoding: Encoding { version: header.version, format: header.format, address_size: header.address_size },
        }
    }

    /// The entry (or null) at `at`; every attribute is decoded to find
    /// where the next starts, so an entry that runs past its unit fails
    /// here.
    pub fn step(&self, at: usize) -> Result<Step, DwarfError> {
        let code = self.section.uleb128(at).map_err(|source| short("entry", source))?;
        if code.value == 0 {
            return Ok(Step::Null { next: code.next });
        }
        let index = self
            .abbreviations
            .index_of(code.value)
            .ok_or(DwarfError::UnknownAbbreviation { at, code: code.value })?;
        let abbreviation = self
            .abbreviations
            .by_index(index)
            .ok_or(DwarfError::UnknownAbbreviation { at, code: code.value })?;
        let mut cursor = code.next;
        for spec in &abbreviation.attributes {
            cursor = decode_value(self.section, cursor, spec.form, spec.implicit, &self.encoding)?.next;
        }
        if cursor > self.header.end {
            return Err(DwarfError::BadReference { offset: at as u64 });
        }
        Ok(Step::Entry(Entry {
            offset: at,
            tag: abbreviation.tag,
            children: abbreviation.children,
            abbreviation: index,
            attributes_at: code.next,
            next: cursor,
        }))
    }

    /// The entry at `at`, which must not be a null.
    pub fn entry_at(&self, at: usize) -> Result<Entry, DwarfError> {
        match self.step(at)? {
            Step::Entry(entry) => Ok(entry),
            Step::Null { .. } => Err(DwarfError::BadReference { offset: at as u64 }),
        }
    }

    /// The value of attribute `name` on `entry`.
    pub fn attribute(&self, entry: &Entry, name: u64) -> Result<Option<Value<'a>>, DwarfError> {
        let abbreviation = self
            .abbreviations
            .by_index(entry.abbreviation)
            .ok_or(DwarfError::BadReference { offset: entry.offset as u64 })?;
        let mut cursor = entry.attributes_at;
        for spec in &abbreviation.attributes {
            let decoded = decode_value(self.section, cursor, spec.form, spec.implicit, &self.encoding)?;
            if spec.name == name {
                return Ok(Some(decoded.value));
            }
            cursor = decoded.next;
        }
        Ok(None)
    }

    /// Whether `offset` (section-absolute) lies in this unit's entries.
    #[must_use]
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.header.entries_at && offset < self.header.end
    }
}
