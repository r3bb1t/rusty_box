//! Abbreviation tables (DWARF 5 §7.5.3): per code, a tag, whether the
//! entry has children, and its attributes' names and forms (with the
//! constant of each `DW_FORM_implicit_const`).

use super::forms::DW_FORM_IMPLICIT_CONST;
use super::{short, DwarfError};
use crate::read::{Bytes, Cursor};

/// Abbreviations in one table past this are not a real file's (a large
/// unit has a few hundred).
const MAX_ABBREVIATIONS: usize = 1 << 16;
/// Attributes of one abbreviation; the standard defines about 130.
const MAX_ATTRIBUTES: usize = 512;

/// Whether an entry is followed by its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Children {
    Yes,
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttributeSpec {
    pub name: u64,
    pub form: u64,
    pub implicit: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Abbreviation {
    pub code: u64,
    pub tag: u64,
    pub children: Children,
    pub attributes: Vec<AttributeSpec>,
}

/// One table, sorted by code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Abbreviations {
    list: Vec<Abbreviation>,
}

impl Abbreviations {
    #[must_use]
    pub fn index_of(&self, code: u64) -> Option<usize> {
        self.list.binary_search_by_key(&code, |abbreviation| abbreviation.code).ok()
    }

    #[must_use]
    pub fn by_index(&self, index: usize) -> Option<&Abbreviation> {
        self.list.get(index)
    }

    #[must_use]
    pub fn get(&self, code: u64) -> Option<&Abbreviation> {
        self.index_of(code).and_then(|index| self.list.get(index))
    }
}

/// The table at `at` in `.debug_abbrev`; it ends at a code of 0.
pub fn parse_abbreviations(section: Bytes<'_>, at: usize) -> Result<Abbreviations, DwarfError> {
    let mut cursor = Cursor::new(section, at);
    let mut list: Vec<Abbreviation> = Vec::new();
    loop {
        let code = cursor.uleb128().map_err(|source| short("abbreviation", source))?;
        if code == 0 {
            break;
        }
        if list.len() >= MAX_ABBREVIATIONS {
            return Err(DwarfError::TooMany { what: "abbreviations", count: list.len() as u64 });
        }
        let tag = cursor.uleb128().map_err(|source| short("abbreviation", source))?;
        let children = match cursor.u8().map_err(|source| short("abbreviation", source))? {
            0 => Children::No,
            _ => Children::Yes,
        };
        let mut attributes = Vec::new();
        loop {
            let name = cursor.uleb128().map_err(|source| short("abbreviation", source))?;
            let form = cursor.uleb128().map_err(|source| short("abbreviation", source))?;
            if name == 0 && form == 0 {
                break;
            }
            if attributes.len() >= MAX_ATTRIBUTES {
                return Err(DwarfError::TooMany { what: "attributes", count: attributes.len() as u64 });
            }
            let implicit = if form == DW_FORM_IMPLICIT_CONST {
                cursor.sleb128().map_err(|source| short("abbreviation", source))?
            } else {
                0
            };
            attributes
                .try_reserve(1)
                .map_err(|_| DwarfError::Allocation { what: "abbreviations" })?;
            attributes.push(AttributeSpec { name, form, implicit });
        }
        list.try_reserve(1).map_err(|_| DwarfError::Allocation { what: "abbreviations" })?;
        list.push(Abbreviation { code, tag, children, attributes });
    }
    list.sort_unstable_by_key(|abbreviation| abbreviation.code);
    Ok(Abbreviations { list })
}
