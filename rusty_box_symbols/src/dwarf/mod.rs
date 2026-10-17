//! DWARF 2 to 5, read by this crate's own code: unit headers ([`unit`]),
//! abbreviation tables ([`abbrev`]), attribute forms ([`forms`]) and the
//! entries of a unit ([`entries`]). The DWARF 5 standard is the reference,
//! with its Appendix F for versions 2 to 4 and GCC's `dwarf2.def` for the
//! GNU forms dwz and split DWARF use.

use crate::read::ReadError;

/// 32- or 64-bit DWARF: the width of section offsets in a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Dwarf32,
    Dwarf64,
}

impl Format {
    /// The width of a section offset.
    #[must_use]
    pub fn offset_size(self) -> usize {
        match self {
            Format::Dwarf32 => 4,
            Format::Dwarf64 => 8,
        }
    }
}

/// Why DWARF did not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DwarfError {
    #[error("the {what} is cut short: {source}")]
    Short {
        what: &'static str,
        #[source]
        source: ReadError,
    },
    #[error("the unit length {value:#x} at offset {at:#x} is a reserved value")]
    ReservedLength { at: usize, value: u32 },
    #[error("the unit at offset {at:#x} is DWARF version {version}, not 2 to 5")]
    BadVersion { at: usize, version: u16 },
    #[error("the unit at offset {at:#x} has {size}-byte addresses, not 4 or 8")]
    BadAddressSize { at: usize, size: u8 },
    #[error("the unit at offset {at:#x} is of unit type {kind:#x}, which DWARF 5 does not define")]
    UnknownUnitType { at: usize, kind: u8 },
    #[error("form {form:#x} is not one this reader knows")]
    UnknownForm { form: u64 },
    #[error("abbreviation code {code} at offset {at:#x} is not in the unit's table")]
    UnknownAbbreviation { at: usize, code: u64 },
    #[error("{count} {what} is more than a real file has")]
    TooMany { what: &'static str, count: u64 },
    #[error("a reference to offset {offset:#x} does not land on an entry")]
    BadReference { offset: u64 },
    #[error("range-list entry kind {kind:#x} is not one DWARF 5 defines")]
    UnknownRangeEntry { kind: u8 },
    #[error("no memory for the {what}")]
    Allocation { what: &'static str },
}

/// `DwarfError::Short` for the named part.
pub(crate) fn short(what: &'static str, source: ReadError) -> DwarfError {
    DwarfError::Short { what, source }
}

/// Pushes `item` after reserving room for it, so the push cannot reach
/// `Vec`'s capacity-overflow panic.
pub(crate) fn push<T>(items: &mut Vec<T>, item: T, what: &'static str) -> Result<(), DwarfError> {
    items.try_reserve(1).map_err(|_| DwarfError::Allocation { what })?;
    items.push(item);
    Ok(())
}

pub mod abbrev;
pub mod entries;
pub mod forms;
pub mod line;
pub mod ranges;
pub mod tables;
pub mod unit;

#[cfg(test)]
mod tests {
    use super::abbrev::{parse_abbreviations, Children};
    use super::entries::{unit_abbreviations, Step, UnitContext};
    use super::forms::*;
    use super::unit::{decode_unit_header, UnitKind, UnitSection};
    use super::*;
    use crate::read::Bytes;
    use rusty_box_symbols_fixtures::dwarf::{build_dwarf, section, Assembler, DwarfFormat, DwarfSpec};

    fn encoding(version: u16, format: Format) -> Encoding {
        Encoding { version, format, address_size: 8 }
    }

    /// One form, its bytes, and the value they decode to.
    struct FormCase {
        form: u64,
        bytes: Vec<u8>,
        value: Value<'static>,
        used: usize,
    }

    fn case(form: u64, bytes: &[u8], value: Value<'static>, used: usize) -> FormCase {
        FormCase { form, bytes: bytes.to_vec(), value, used }
    }

    #[test]
    fn every_fixed_form_decodes_to_its_value() {
        let cases = [
            case(DW_FORM_ADDR, &[1, 2, 3, 4, 5, 6, 7, 8], Value::Address(0x0807_0605_0403_0201), 8),
            case(DW_FORM_DATA1, &[0x7F], Value::Unsigned(0x7F), 1),
            case(DW_FORM_DATA2, &[1, 2], Value::Unsigned(0x0201), 2),
            case(DW_FORM_DATA4, &[1, 2, 3, 4], Value::Unsigned(0x0403_0201), 4),
            case(DW_FORM_DATA8, &[1, 0, 0, 0, 0, 0, 0, 0], Value::Unsigned(1), 8),
            case(DW_FORM_DATA16, &[7; 16], Value::Data16(&[7; 16]), 16),
            case(DW_FORM_SDATA, &[0x7E], Value::Signed(-2), 1),
            case(DW_FORM_UDATA, &[0x80, 0x01], Value::Unsigned(128), 2),
            case(DW_FORM_FLAG, &[1], Value::Flag(1), 1),
            case(DW_FORM_FLAG_PRESENT, &[], Value::Flag(1), 0),
            case(DW_FORM_STRING, b"ab\0", Value::String(b"ab"), 3),
            case(DW_FORM_STRP, &[0x10, 0, 0, 0], Value::StrOffset(0x10), 4),
            case(DW_FORM_LINE_STRP, &[0x20, 0, 0, 0], Value::LineStrOffset(0x20), 4),
            case(DW_FORM_STRP_SUP, &[0x30, 0, 0, 0], Value::SupStrOffset(0x30), 4),
            case(DW_FORM_GNU_STRP_ALT, &[0x40, 0, 0, 0], Value::SupStrOffset(0x40), 4),
            case(DW_FORM_STRX, &[5], Value::StrIndex(5), 1),
            case(DW_FORM_STRX1, &[5], Value::StrIndex(5), 1),
            case(DW_FORM_STRX2, &[5, 1], Value::StrIndex(0x105), 2),
            case(DW_FORM_STRX3, &[5, 1, 2], Value::StrIndex(0x2_0105), 3),
            case(DW_FORM_STRX4, &[5, 1, 2, 3], Value::StrIndex(0x0302_0105), 4),
            case(DW_FORM_GNU_STR_INDEX, &[7], Value::StrIndex(7), 1),
            case(DW_FORM_ADDRX, &[3], Value::AddressIndex(3), 1),
            case(DW_FORM_ADDRX1, &[3], Value::AddressIndex(3), 1),
            case(DW_FORM_ADDRX2, &[3, 0], Value::AddressIndex(3), 2),
            case(DW_FORM_ADDRX3, &[3, 0, 0], Value::AddressIndex(3), 3),
            case(DW_FORM_ADDRX4, &[3, 0, 0, 0], Value::AddressIndex(3), 4),
            case(DW_FORM_GNU_ADDR_INDEX, &[9], Value::AddressIndex(9), 1),
            case(DW_FORM_REF1, &[0x20], Value::UnitRef(0x20), 1),
            case(DW_FORM_REF2, &[0x20, 0], Value::UnitRef(0x20), 2),
            case(DW_FORM_REF4, &[0x20, 0, 0, 0], Value::UnitRef(0x20), 4),
            case(DW_FORM_REF8, &[0x20, 0, 0, 0, 0, 0, 0, 0], Value::UnitRef(0x20), 8),
            case(DW_FORM_REF_UDATA, &[0x20], Value::UnitRef(0x20), 1),
            case(DW_FORM_REF_ADDR, &[0x50, 0, 0, 0], Value::InfoRef(0x50), 4),
            case(DW_FORM_REF_SUP4, &[0x60, 0, 0, 0], Value::SupInfoRef(0x60), 4),
            case(DW_FORM_REF_SUP8, &[0x60, 0, 0, 0, 0, 0, 0, 0], Value::SupInfoRef(0x60), 8),
            case(DW_FORM_GNU_REF_ALT, &[0x70, 0, 0, 0], Value::SupInfoRef(0x70), 4),
            case(DW_FORM_REF_SIG8, &[1, 0, 0, 0, 0, 0, 0, 0], Value::Signature(1), 8),
            case(DW_FORM_SEC_OFFSET, &[0x80, 0, 0, 0], Value::SecOffset(0x80), 4),
            case(DW_FORM_RNGLISTX, &[2], Value::RangeListIndex(2), 1),
            case(DW_FORM_LOCLISTX, &[2], Value::LocationListIndex(2), 1),
            case(DW_FORM_EXPRLOC, &[2, 0x23, 8], Value::Block(&[0x23, 8]), 3),
            case(DW_FORM_BLOCK, &[1, 9], Value::Block(&[9]), 2),
            case(DW_FORM_BLOCK1, &[1, 9], Value::Block(&[9]), 2),
            case(DW_FORM_BLOCK2, &[1, 0, 9], Value::Block(&[9]), 3),
            case(DW_FORM_BLOCK4, &[1, 0, 0, 0, 9], Value::Block(&[9]), 5),
            case(DW_FORM_INDIRECT, &[0x0b, 0x2A], Value::Unsigned(0x2A), 2),
        ];
        let unit = encoding(5, Format::Dwarf32);
        for case in cases {
            let decoded = decode_value(Bytes::new(&case.bytes), 0, case.form, 0, &unit).unwrap();
            assert_eq!(decoded.value, case.value, "form {:#x}", case.form);
            assert_eq!(decoded.next, case.used, "form {:#x}", case.form);
        }
        let implicit = decode_value(Bytes::new(&[]), 0, DW_FORM_IMPLICIT_CONST, -5, &unit).unwrap();
        assert_eq!(implicit.value, Value::Signed(-5));
    }

    #[test]
    fn offset_sized_forms_follow_the_format_and_ref_addr_the_version() {
        let eight = [0x10, 0, 0, 0, 0, 0, 0, 0];
        let wide = encoding(5, Format::Dwarf64);
        assert_eq!(decode_value(Bytes::new(&eight), 0, DW_FORM_STRP, 0, &wide).unwrap().next, 8);
        assert_eq!(decode_value(Bytes::new(&eight), 0, DW_FORM_SEC_OFFSET, 0, &wide).unwrap().next, 8);
        // DWARF 2: ref_addr is address-sized.
        let two = Encoding { version: 2, format: Format::Dwarf32, address_size: 8 };
        assert_eq!(decode_value(Bytes::new(&eight), 0, DW_FORM_REF_ADDR, 0, &two).unwrap().next, 8);
        let three = Encoding { version: 3, format: Format::Dwarf32, address_size: 8 };
        assert_eq!(decode_value(Bytes::new(&eight), 0, DW_FORM_REF_ADDR, 0, &three).unwrap().next, 4);
    }

    #[test]
    fn an_unknown_form_or_a_chain_of_indirect_forms_is_refused() {
        let unit = encoding(5, Format::Dwarf32);
        assert_eq!(decode_value(Bytes::new(&[0]), 0, 0x2D, 0, &unit), Err(DwarfError::UnknownForm { form: 0x2D }));
        // An indirect form naming indirect again.
        assert_eq!(
            decode_value(Bytes::new(&[0x16, 0x16, 0x0b, 1]), 0, DW_FORM_INDIRECT, 0, &unit),
            Err(DwarfError::UnknownForm { form: DW_FORM_INDIRECT })
        );
    }

    #[test]
    fn abbreviations_read_with_implicit_constants() {
        let mut table = Assembler::default();
        // Code 1: DW_TAG_member, no children, name (strp), byte_size (implicit 4).
        table.uleb(1).uleb(0x0d).u8(0).uleb(0x03).uleb(0x0e).uleb(0x0b).uleb(0x21).sleb(4).uleb(0).uleb(0);
        // Code 2: DW_TAG_structure_type, children.
        table.uleb(2).uleb(0x13).u8(1).uleb(0).uleb(0);
        table.uleb(0);
        let abbreviations = parse_abbreviations(Bytes::new(&table.bytes), 0).unwrap();
        let member = abbreviations.get(1).unwrap();
        assert_eq!(member.tag, 0x0d);
        assert_eq!(member.children, Children::No);
        assert_eq!(member.attributes.len(), 2);
        assert_eq!(member.attributes[1].implicit, 4);
        assert_eq!(abbreviations.get(2).unwrap().children, Children::Yes);
        assert!(abbreviations.get(3).is_none());
    }

    #[test]
    fn units_of_every_version_and_format_walk_to_their_names() {
        for version in [2u16, 3, 4, 5] {
            for format in [DwarfFormat::Dwarf32, DwarfFormat::Dwarf64] {
                let sections = build_dwarf(DwarfSpec { version, format, text_va: 0x1000 });
                let info = Bytes::new(section(&sections, ".debug_info"));
                let header = decode_unit_header(info, 0, UnitSection::Info).unwrap();
                assert_eq!(header.version, version);
                assert_eq!(header.kind, UnitKind::Compile);
                assert_eq!(header.end, info.len(), "{version} {format:?}");
                let abbreviations = unit_abbreviations(&header, Bytes::new(section(&sections, ".debug_abbrev"))).unwrap();
                let context = UnitContext::new(info, header, &abbreviations);
                let strings = Bytes::new(section(&sections, ".debug_str"));
                let mut names = Vec::new();
                let mut at = header.entries_at;
                while at < header.end {
                    match context.step(at).unwrap() {
                        Step::Null { next } => at = next,
                        Step::Entry(entry) => {
                            if let Some(Value::StrOffset(offset)) = context.attribute(&entry, 0x03).unwrap() {
                                names.push(String::from_utf8_lossy(strings.cstr(offset as usize, 64).unwrap()).into_owned());
                            }
                            at = entry.next;
                        }
                    }
                }
                assert!(names.contains(&"widget_init".to_owned()), "{version} {format:?}: {names:?}");
                assert!(names.contains(&"Outer".to_owned()));
            }
        }
    }

    #[test]
    fn a_unit_longer_than_its_section_or_of_an_unknown_version_is_refused() {
        let mut unit = Assembler::default();
        unit.u32(100).u16(5).u8(1).u8(8).u32(0);
        assert!(matches!(
            decode_unit_header(Bytes::new(&unit.bytes), 0, UnitSection::Info),
            Err(DwarfError::Short { .. })
        ));
        let mut unit = Assembler::default();
        unit.u32(8).u16(6).u8(1).u8(8).u32(0);
        assert_eq!(
            decode_unit_header(Bytes::new(&unit.bytes), 0, UnitSection::Info),
            Err(DwarfError::BadVersion { at: 0, version: 6 })
        );
        let mut unit = Assembler::default();
        unit.u32(0xFFFF_FFF0).u16(5);
        assert_eq!(
            decode_unit_header(Bytes::new(&unit.bytes), 0, UnitSection::Info),
            Err(DwarfError::ReservedLength { at: 0, value: 0xFFFF_FFF0 })
        );
    }

    #[test]
    fn a_version_5_type_unit_header_names_its_signature() {
        let mut unit = Assembler::default();
        // unit_length, version 5, DW_UT_type, address size 8, abbrev offset
        // 0, signature, type offset; then one null entry.
        unit.u32(0).u16(5).u8(2).u8(8).u32(0).u64(0xABCD).u32(24).u8(0);
        let length = (unit.len() - 4) as u32;
        unit.patch_u32(0, length);
        let header = decode_unit_header(Bytes::new(&unit.bytes), 0, UnitSection::Info).unwrap();
        assert_eq!(header.kind, UnitKind::Type { signature: 0xABCD, type_offset: 24 });
        assert_eq!(header.entries_at, 24);
    }
}
