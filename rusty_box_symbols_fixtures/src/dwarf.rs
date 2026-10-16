//! DWARF test sections: one compilation unit written by gimli, and a byte
//! assembler for what gimli cannot write (the `strx`/`addrx`/`rnglistx`
//! forms, type units, the GNU alternate-file forms).

use gimli::write::{Address, AttributeValue, DwarfUnit, EndianVec, Expression, LineProgram, LineString, Sections};
use gimli::{Encoding, Format, LineEncoding};

use crate::elf::{build_elf, ElfSpec, Encoding as StoredAs, ExtraSection, Symtab, FUNC_LEN};

pub const COMP_DIR: &str = "/build/widget";

/// 32- or 64-bit DWARF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DwarfFormat {
    Dwarf32,
    Dwarf64,
}

/// What `build_dwarf` writes.
#[derive(Clone, Copy, Debug)]
pub struct DwarfSpec {
    pub version: u16,
    pub format: DwarfFormat,
    /// The link-time address of `.text`.
    pub text_va: u64,
}

/// The bytes of section `name` among `sections`.
pub fn section<'a>(sections: &'a [ExtraSection], name: &str) -> &'a [u8] {
    sections
        .iter()
        .find(|section| section.name == name)
        .map(|section| section.data.as_slice())
        .unwrap_or(&[])
}

/// The DWARF of one compilation unit, `src/main.c` built in `COMP_DIR`: the
/// subprogram `widget_init` over `.text[0..FUNC_LEN)` at `text_va`; a line
/// program (`text_va` is line 10, `text_va + 8` line 11); a
/// declaration-only `struct Widget` the reader must pass by, then its
/// definition `{ int a @0; long b @8 (as DW_OP_plus_uconst); unsigned c:3
/// at bit 37 (DW_AT_data_bit_offset); unsigned d:2 in the DWARF 2/3 form,
/// storage @4, bit 8 }`; `struct Outer { int x @0; struct Widget inner @8;
/// }`; and `typedef struct Widget WidgetAlias`.
pub fn build_dwarf(spec: DwarfSpec) -> Vec<ExtraSection> {
    let encoding = Encoding {
        format: match spec.format {
            DwarfFormat::Dwarf32 => Format::Dwarf32,
            DwarfFormat::Dwarf64 => Format::Dwarf64,
        },
        version: spec.version,
        address_size: 8,
    };
    let text_va = spec.text_va;
    let mut dwarf = DwarfUnit::new(encoding);
    let comp_dir = LineString::new(COMP_DIR, encoding, &mut dwarf.line_strings);
    let src_dir = LineString::new("src", encoding, &mut dwarf.line_strings);
    let main_c = LineString::new("main.c", encoding, &mut dwarf.line_strings);
    let mut program = LineProgram::new(encoding, LineEncoding::default(), comp_dir, Some(src_dir.clone()), main_c.clone(), None);
    let directory = program.add_directory(src_dir);
    let file = program.add_file(main_c, directory, None);
    program.begin_sequence(Some(Address::Constant(text_va)));
    program.row().file = file;
    program.row().address_offset = 0;
    program.row().line = 10;
    program.generate_row();
    program.row().address_offset = 8;
    program.row().line = 11;
    program.generate_row();
    program.end_sequence(FUNC_LEN);
    dwarf.unit.line_program = program;

    let producer = dwarf.strings.add("rusty_box_symbols fixture");
    let unit_name = dwarf.strings.add("src/main.c");
    let unit_dir = dwarf.strings.add(COMP_DIR);
    let root = dwarf.unit.root();
    {
        let unit = dwarf.unit.get_mut(root);
        unit.set(gimli::DW_AT_producer, AttributeValue::StringRef(producer));
        unit.set(gimli::DW_AT_name, AttributeValue::StringRef(unit_name));
        unit.set(gimli::DW_AT_comp_dir, AttributeValue::StringRef(unit_dir));
        unit.set(gimli::DW_AT_language, AttributeValue::Language(gimli::DW_LANG_C99));
        unit.set(gimli::DW_AT_low_pc, AttributeValue::Address(Address::Constant(text_va)));
        unit.set(gimli::DW_AT_high_pc, AttributeValue::Udata(FUNC_LEN));
    }
    let function_name = dwarf.strings.add("widget_init");
    let function = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
    {
        let entry = dwarf.unit.get_mut(function);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(function_name));
        entry.set(gimli::DW_AT_external, AttributeValue::FlagPresent);
        entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(Address::Constant(text_va)));
        entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(FUNC_LEN));
    }

    let mut base = |name: &str, size: u64, encoding: gimli::DwAte| {
        let id = dwarf.unit.add(root, gimli::DW_TAG_base_type);
        let name = dwarf.strings.add(name);
        let entry = dwarf.unit.get_mut(id);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(name));
        entry.set(gimli::DW_AT_byte_size, AttributeValue::Udata(size));
        entry.set(gimli::DW_AT_encoding, AttributeValue::Encoding(encoding));
        id
    };
    let int_type = base("int", 4, gimli::DW_ATE_signed);
    let long_type = base("long", 8, gimli::DW_ATE_signed);
    let unsigned_type = base("unsigned int", 4, gimli::DW_ATE_unsigned);

    let widget_name = dwarf.strings.add("Widget");
    let declaration = dwarf.unit.add(root, gimli::DW_TAG_structure_type);
    {
        let entry = dwarf.unit.get_mut(declaration);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(widget_name));
        entry.set(gimli::DW_AT_declaration, AttributeValue::FlagPresent);
    }
    let widget = dwarf.unit.add(root, gimli::DW_TAG_structure_type);
    {
        let entry = dwarf.unit.get_mut(widget);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(widget_name));
        entry.set(gimli::DW_AT_byte_size, AttributeValue::Udata(16));
    }
    let outer = dwarf.unit.add(root, gimli::DW_TAG_structure_type);
    let mut member = |parent, name: &str, ty| {
        let id = dwarf.unit.add(parent, gimli::DW_TAG_member);
        let name = dwarf.strings.add(name);
        let entry = dwarf.unit.get_mut(id);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(name));
        entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(ty));
        id
    };
    let a = member(widget, "a", int_type);
    let b = member(widget, "b", long_type);
    let c = member(widget, "c", unsigned_type);
    let d = member(widget, "d", unsigned_type);
    let x = member(outer, "x", int_type);
    let inner = member(outer, "inner", declaration);
    let outer_name = dwarf.strings.add("Outer");
    {
        let entry = dwarf.unit.get_mut(outer);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(outer_name));
        entry.set(gimli::DW_AT_byte_size, AttributeValue::Udata(24));
    }
    dwarf.unit.get_mut(a).set(gimli::DW_AT_data_member_location, AttributeValue::Udata(0));
    let mut at_eight = Expression::new();
    at_eight.op_plus_uconst(8);
    dwarf.unit.get_mut(b).set(gimli::DW_AT_data_member_location, AttributeValue::Exprloc(at_eight));
    {
        let entry = dwarf.unit.get_mut(c);
        entry.set(gimli::DW_AT_bit_size, AttributeValue::Udata(3));
        entry.set(gimli::DW_AT_data_bit_offset, AttributeValue::Udata(4 * 8 + 5));
    }
    {
        // Storage 4 bytes at 4; 2 bits whose top is 22 bits below the
        // unit's most significant bit: LSB-first position 32 - 22 - 2 = 8.
        let entry = dwarf.unit.get_mut(d);
        entry.set(gimli::DW_AT_data_member_location, AttributeValue::Udata(4));
        entry.set(gimli::DW_AT_byte_size, AttributeValue::Udata(4));
        entry.set(gimli::DW_AT_bit_size, AttributeValue::Udata(2));
        entry.set(gimli::DW_AT_bit_offset, AttributeValue::Udata(22));
    }
    dwarf.unit.get_mut(x).set(gimli::DW_AT_data_member_location, AttributeValue::Udata(0));
    dwarf.unit.get_mut(inner).set(gimli::DW_AT_data_member_location, AttributeValue::Udata(8));
    let alias = dwarf.unit.add(root, gimli::DW_TAG_typedef);
    let alias_name = dwarf.strings.add("WidgetAlias");
    {
        let entry = dwarf.unit.get_mut(alias);
        entry.set(gimli::DW_AT_name, AttributeValue::StringRef(alias_name));
        entry.set(gimli::DW_AT_type, AttributeValue::UnitRef(widget));
    }

    let mut sections = Sections::new(EndianVec::new(gimli::LittleEndian));
    dwarf.write(&mut sections).expect("DWARF");
    let mut out = Vec::new();
    sections
        .for_each(|id, data| {
            if !data.slice().is_empty() {
                out.push(ExtraSection { name: id.name().to_owned(), data: data.slice().to_vec(), encoding: StoredAs::Plain });
            }
            Ok::<(), gimli::write::Error>(())
        })
        .expect("DWARF sections");
    out
}

/// Little-endian bytes for hand-built DWARF.
#[derive(Default)]
pub struct Assembler {
    pub bytes: Vec<u8>,
}

impl Assembler {
    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn uleb(&mut self, mut value: u64) -> &mut Self {
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                self.bytes.push(byte);
                return self;
            }
            self.bytes.push(byte | 0x80);
        }
    }

    pub fn sleb(&mut self, mut value: i64) -> &mut Self {
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
            self.bytes.push(if done { byte } else { byte | 0x80 });
            if done {
                return self;
            }
        }
    }

    pub fn cstr(&mut self, text: &str) -> &mut Self {
        self.bytes.extend_from_slice(text.as_bytes());
        self.bytes.push(0);
        self
    }

    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Overwrites the 4 bytes at `at` with `value` (a length known later).
    pub fn patch_u32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// The signature of `split_forms_dwarf`'s type unit.
pub const POINT_SIGNATURE: u64 = 0x1122_3344_5566_7788;

/// Overwrites a 4-byte length at `at` with the bytes that follow it.
fn close_length(unit: &mut Assembler, at: usize) {
    let length = (unit.len() - at - 4) as u32;
    unit.patch_u32(at, length);
}

/// DWARF 5 that uses the forms gimli does not write: a compilation unit
/// whose names are `strx1` (through `.debug_str_offsets`), whose
/// `split_init` starts at an `addrx1` (through `.debug_addr`) over
/// `[text_va, text_va + 0x10)`, whose `split_ranges` covers
/// `[text_va + 0x10, text_va + 0x20)` through `rnglistx`, and whose
/// `struct Holder { struct Point held @8; }` names its member's type by
/// `ref_sig8`; and the type unit of signature `POINT_SIGNATURE` defining
/// `struct Point { long x @0; long y @8; }`.
pub fn split_forms_dwarf(text_va: u64) -> Vec<ExtraSection> {
    let mut abbrev = Assembler::default();
    // 1: compile_unit: name strx1, str_offsets_base, addr_base,
    // rnglists_base (sec_offset), low_pc addr.
    abbrev.uleb(1).uleb(0x11).u8(1);
    abbrev.uleb(0x03).uleb(0x25).uleb(0x72).uleb(0x17).uleb(0x73).uleb(0x17).uleb(0x74).uleb(0x17).uleb(0x11).uleb(0x01);
    abbrev.uleb(0).uleb(0);
    // 2: subprogram: name strx1, low_pc addrx1, high_pc data4.
    abbrev.uleb(2).uleb(0x2e).u8(0).uleb(0x03).uleb(0x25).uleb(0x11).uleb(0x29).uleb(0x12).uleb(0x06).uleb(0).uleb(0);
    // 3: subprogram: name strx1, ranges rnglistx.
    abbrev.uleb(3).uleb(0x2e).u8(0).uleb(0x03).uleb(0x25).uleb(0x55).uleb(0x23).uleb(0).uleb(0);
    // 4: structure_type: name strx1, byte_size data1.
    abbrev.uleb(4).uleb(0x13).u8(1).uleb(0x03).uleb(0x25).uleb(0x0b).uleb(0x0b).uleb(0).uleb(0);
    // 5: member: name strx1, type ref_sig8, data_member_location data1.
    abbrev.uleb(5).uleb(0x0d).u8(0).uleb(0x03).uleb(0x25).uleb(0x49).uleb(0x20).uleb(0x38).uleb(0x0b).uleb(0).uleb(0);
    // 6: type_unit root.
    abbrev.uleb(6).uleb(0x41).u8(1).uleb(0).uleb(0);
    // 7: structure_type: name string, byte_size data1.
    abbrev.uleb(7).uleb(0x13).u8(1).uleb(0x03).uleb(0x08).uleb(0x0b).uleb(0x0b).uleb(0).uleb(0);
    // 8: member: name string, type ref4, data_member_location data1.
    abbrev.uleb(8).uleb(0x0d).u8(0).uleb(0x03).uleb(0x08).uleb(0x49).uleb(0x13).uleb(0x38).uleb(0x0b).uleb(0).uleb(0);
    // 9: base_type: name string, byte_size data1.
    abbrev.uleb(9).uleb(0x24).u8(0).uleb(0x03).uleb(0x08).uleb(0x0b).uleb(0x0b).uleb(0).uleb(0);
    abbrev.uleb(0);

    let mut strings = Assembler::default();
    let mut offsets = Vec::new();
    for name in ["split.c", "split_init", "split_ranges", "Holder", "held"] {
        offsets.push(strings.len() as u32);
        strings.cstr(name);
    }
    let mut str_offsets = Assembler::default();
    str_offsets.u32(0).u16(5).u16(0);
    for offset in &offsets {
        str_offsets.u32(*offset);
    }
    close_length(&mut str_offsets, 0);

    let mut addresses = Assembler::default();
    addresses.u32(0).u16(5).u8(8).u8(0).u64(text_va);
    close_length(&mut addresses, 0);

    // Header (12 bytes), one offset (4 bytes, relative to the header's
    // end), then DW_RLE_start_length and DW_RLE_end_of_list.
    let mut rnglists = Assembler::default();
    rnglists.u32(0).u16(5).u8(8).u8(0).u32(1).u32(4).u8(7).u64(text_va + 0x10).uleb(0x10).u8(0);
    close_length(&mut rnglists, 0);

    let mut info = Assembler::default();
    // The type unit.
    info.u32(0).u16(5).u8(2).u8(8).u32(0).u64(POINT_SIGNATURE);
    let type_offset_at = info.len();
    info.u32(0);
    info.uleb(6);
    let point = info.len();
    info.uleb(7).cstr("Point").u8(16);
    info.uleb(8).cstr("x");
    let x_type = info.len();
    info.u32(0).u8(0);
    info.uleb(8).cstr("y");
    let y_type = info.len();
    info.u32(0).u8(8);
    info.u8(0);
    let long = info.len();
    info.uleb(9).cstr("long").u8(8);
    info.u8(0);
    info.patch_u32(type_offset_at, point as u32);
    info.patch_u32(x_type, long as u32);
    info.patch_u32(y_type, long as u32);
    close_length(&mut info, 0);
    // The compile unit.
    let compile = info.len();
    info.u32(0).u16(5).u8(1).u8(8).u32(0);
    info.uleb(1).u8(0).u32(8).u32(8).u32(12).u64(0);
    info.uleb(2).u8(1).u8(0).u32(0x10);
    info.uleb(3).u8(2).uleb(0);
    info.uleb(4).u8(3).u8(24);
    info.uleb(5).u8(4).u64(POINT_SIGNATURE).u8(8);
    info.u8(0);
    info.u8(0);
    close_length(&mut info, compile);

    let plain = |name: &str, data: Vec<u8>| ExtraSection { name: name.to_owned(), data, encoding: StoredAs::Plain };
    vec![
        plain(".debug_info", info.bytes),
        plain(".debug_abbrev", abbrev.bytes),
        plain(".debug_str", strings.bytes),
        plain(".debug_str_offsets", str_offsets.bytes),
        plain(".debug_addr", addresses.bytes),
        plain(".debug_rnglists", rnglists.bytes),
    ]
}

/// DWARF 4: `typedef A B; typedef B A; struct Loop { A m @0; }` — a type
/// chain that never reaches a type.
pub fn looping_typedefs() -> Vec<ExtraSection> {
    let encoding = Encoding { format: Format::Dwarf32, version: 4, address_size: 8 };
    let mut dwarf = DwarfUnit::new(encoding);
    let root = dwarf.unit.root();
    let a = dwarf.unit.add(root, gimli::DW_TAG_typedef);
    let b = dwarf.unit.add(root, gimli::DW_TAG_typedef);
    let a_name = dwarf.strings.add("A");
    let b_name = dwarf.strings.add("B");
    dwarf.unit.get_mut(a).set(gimli::DW_AT_name, AttributeValue::StringRef(a_name));
    dwarf.unit.get_mut(a).set(gimli::DW_AT_type, AttributeValue::UnitRef(b));
    dwarf.unit.get_mut(b).set(gimli::DW_AT_name, AttributeValue::StringRef(b_name));
    dwarf.unit.get_mut(b).set(gimli::DW_AT_type, AttributeValue::UnitRef(a));
    let looping = dwarf.unit.add(root, gimli::DW_TAG_structure_type);
    let loop_name = dwarf.strings.add("Loop");
    dwarf.unit.get_mut(looping).set(gimli::DW_AT_name, AttributeValue::StringRef(loop_name));
    dwarf.unit.get_mut(looping).set(gimli::DW_AT_byte_size, AttributeValue::Udata(8));
    let member = dwarf.unit.add(looping, gimli::DW_TAG_member);
    let member_name = dwarf.strings.add("m");
    dwarf.unit.get_mut(member).set(gimli::DW_AT_name, AttributeValue::StringRef(member_name));
    dwarf.unit.get_mut(member).set(gimli::DW_AT_type, AttributeValue::UnitRef(a));
    dwarf.unit.get_mut(member).set(gimli::DW_AT_data_member_location, AttributeValue::Udata(0));
    let mut sections = Sections::new(EndianVec::new(gimli::LittleEndian));
    dwarf.write(&mut sections).expect("DWARF");
    let mut out = Vec::new();
    sections
        .for_each(|id, data| {
            if !data.slice().is_empty() {
                out.push(ExtraSection { name: id.name().to_owned(), data: data.slice().to_vec(), encoding: StoredAs::Plain });
            }
            Ok::<(), gimli::write::Error>(())
        })
        .expect("DWARF sections");
    out
}

/// Which forms a dwz pair refers to its supplement with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DwzForms {
    /// DWARF 4 with `DW_FORM_GNU_strp_alt` and `DW_FORM_GNU_ref_alt`.
    Gnu,
    /// DWARF 5 with `DW_FORM_strp_sup` and `DW_FORM_ref_sup4`.
    Standard,
}

/// The supplement's build-id, as the main file's `.gnu_debugaltlink` names it.
pub const SUPPLEMENT_BUILD_ID: [u8; 20] = [0x5A; 20];

/// A dwz-style pair: the main file's DWARF sections, and the supplement as
/// a whole ELF file.
pub struct DwzPair {
    pub main: Vec<ExtraSection>,
    pub supplement: Vec<u8>,
}

/// A unit header of `version` (4: plain; 5: of unit type `unit_type`).
fn unit_header(unit: &mut Assembler, version: u16, unit_type: u8) {
    match version {
        5 => unit.u32(0).u16(5).u8(unit_type).u8(8).u32(0),
        _ => unit.u32(0).u16(version).u32(0).u8(8),
    };
}

/// The supplement's partial unit defines `struct Shared { long value @0; }`
/// (8 bytes) and holds the string `dwz_function`. The main file's unit
/// names `dwz_function` over `[text_va, text_va + 0x10)` through the
/// supplement's strings, `own_function` over `[text_va + 0x10, text_va +
/// 0x20)` inline, and `struct Local { struct Shared shared @8; }` (16
/// bytes) through the supplement's entries; its `.gnu_debugaltlink` names
/// the supplement by `SUPPLEMENT_BUILD_ID`.
pub fn dwz_pair(forms: DwzForms, text_va: u64) -> DwzPair {
    let version: u16 = match forms {
        DwzForms::Gnu => 4,
        DwzForms::Standard => 5,
    };
    let string_form: u64 = match forms {
        DwzForms::Gnu => 0x1f21,
        DwzForms::Standard => 0x1d,
    };
    let reference_form: u64 = match forms {
        DwzForms::Gnu => 0x1f20,
        DwzForms::Standard => 0x1c,
    };

    let mut sup_strings = Assembler::default();
    sup_strings.u8(0);
    let dwz_function = sup_strings.len();
    sup_strings.cstr("dwz_function");
    let shared = sup_strings.len();
    sup_strings.cstr("Shared");
    let value = sup_strings.len();
    sup_strings.cstr("value");
    let long = sup_strings.len();
    sup_strings.cstr("long");
    let mut sup_abbrev = Assembler::default();
    // 1: partial_unit; 2: structure_type (name strp, byte_size data1);
    // 3: member (name strp, type ref4, location data1); 4: base_type.
    sup_abbrev.uleb(1).uleb(0x3c).u8(1).uleb(0).uleb(0);
    sup_abbrev.uleb(2).uleb(0x13).u8(1).uleb(0x03).uleb(0x0e).uleb(0x0b).uleb(0x0b).uleb(0).uleb(0);
    sup_abbrev.uleb(3).uleb(0x0d).u8(0).uleb(0x03).uleb(0x0e).uleb(0x49).uleb(0x13).uleb(0x38).uleb(0x0b).uleb(0).uleb(0);
    sup_abbrev.uleb(4).uleb(0x24).u8(0).uleb(0x03).uleb(0x0e).uleb(0x0b).uleb(0x0b).uleb(0).uleb(0);
    sup_abbrev.uleb(0);
    let mut sup_info = Assembler::default();
    unit_header(&mut sup_info, version, 3);
    sup_info.uleb(1);
    let shared_entry = sup_info.len();
    sup_info.uleb(2).u32(shared as u32).u8(8);
    sup_info.uleb(3).u32(value as u32);
    let value_type = sup_info.len();
    sup_info.u32(0).u8(0);
    sup_info.u8(0);
    let long_entry = sup_info.len();
    sup_info.uleb(4).u32(long as u32).u8(8);
    sup_info.u8(0);
    sup_info.patch_u32(value_type, long_entry as u32);
    close_length(&mut sup_info, 0);
    let plain = |name: &str, data: Vec<u8>| ExtraSection { name: name.to_owned(), data, encoding: StoredAs::Plain };
    let supplement_spec = ElfSpec { symtab: Symtab::Without, build_id: SUPPLEMENT_BUILD_ID, ..ElfSpec::pie() };
    let supplement = build_elf(
        supplement_spec,
        &[
            plain(".debug_info", sup_info.bytes),
            plain(".debug_abbrev", sup_abbrev.bytes),
            plain(".debug_str", sup_strings.bytes),
        ],
    );

    let mut abbrev = Assembler::default();
    // 1: compile_unit (name string, low_pc addr); 2: subprogram named in
    // the supplement; 3: subprogram named inline; 4: structure_type;
    // 5: member typed in the supplement.
    abbrev.uleb(1).uleb(0x11).u8(1).uleb(0x03).uleb(0x08).uleb(0x11).uleb(0x01).uleb(0).uleb(0);
    abbrev.uleb(2).uleb(0x2e).u8(0).uleb(0x03).uleb(string_form).uleb(0x11).uleb(0x01).uleb(0x12).uleb(0x06).uleb(0).uleb(0);
    abbrev.uleb(3).uleb(0x2e).u8(0).uleb(0x03).uleb(0x08).uleb(0x11).uleb(0x01).uleb(0x12).uleb(0x06).uleb(0).uleb(0);
    abbrev.uleb(4).uleb(0x13).u8(1).uleb(0x03).uleb(0x08).uleb(0x0b).uleb(0x0b).uleb(0).uleb(0);
    abbrev.uleb(5).uleb(0x0d).u8(0).uleb(0x03).uleb(0x08).uleb(0x49).uleb(reference_form).uleb(0x38).uleb(0x0b).uleb(0).uleb(0);
    abbrev.uleb(0);
    let mut info = Assembler::default();
    unit_header(&mut info, version, 1);
    info.uleb(1).cstr("dwz.c").u64(0);
    info.uleb(2).u32(dwz_function as u32).u64(text_va).u32(0x10);
    info.uleb(3).cstr("own_function").u64(text_va + 0x10).u32(0x10);
    info.uleb(4).cstr("Local").u8(16);
    info.uleb(5).cstr("shared").u32(shared_entry as u32).u8(8);
    info.u8(0);
    info.u8(0);
    close_length(&mut info, 0);
    let mut alt_link = Assembler::default();
    alt_link.cstr("/usr/lib/debug/.dwz/x86_64-redhat-linux-gnu/fixture.debug").bytes(&SUPPLEMENT_BUILD_ID);
    DwzPair {
        main: vec![
            plain(".debug_info", info.bytes),
            plain(".debug_abbrev", abbrev.bytes),
            plain(".gnu_debugaltlink", alt_link.bytes),
        ],
        supplement,
    }
}
