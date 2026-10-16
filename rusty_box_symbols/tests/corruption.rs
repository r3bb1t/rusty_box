//! The corruption test (spec §12): every fixture, turned into thousands of
//! numbered variants (`rusty_box_symbols_fixtures::mutate`), goes through
//! every reader that accepts its kind. Each must return `Ok` or `Err`; none
//! may panic, and none may take a second, which would point at a loop
//! whose bound is missing (spec §6.3).
//!
//! A failure names its fixture and case. `CORRUPTION_CASE=<fixture>#<case>`
//! runs that case alone; `CORRUPTION_TRACE=1` prints each case before it
//! runs, which names a case that aborts the process (an allocation failure
//! aborts instead of panicking) by being the last line printed.

use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use rusty_box_symbols::cab::{extract_single, is_cabinet};
use rusty_box_symbols::decompress::{decompress_to_vec, Codec};
use rusty_box_symbols::elf::identity_from_image;
use rusty_box_symbols::identity::PeImageId;
use rusty_box_symbols::pe::{pdb_from_binary, read_pe_identity, ImageRead};
use rusty_box_symbols::tables::{declared_identity, supplement_of, SymbolTables};
use rusty_box_symbols::units::Rva;
use rusty_box_symbols_fixtures::cab::{cabinet, CabinetKind, LZX15_CABINET, LZX21_CABINET};
use rusty_box_symbols_fixtures::compress::{raw_deflate, sample_binary, sample_text, zlib, zstd_frame, Checksum, ZstdOptions};
use rusty_box_symbols_fixtures::dwarf::{
    build_dwarf, dwz_pair, looping_typedefs, split_forms_dwarf, DwarfFormat, DwarfSpec, DwzForms,
};
use rusty_box_symbols_fixtures::elf::{build_elf, load_image, ElfSpec, Encoding, ExtraSection, Symtab, Text, TEXT_RVA};
use rusty_box_symbols_fixtures::mutate::{variants, Budget, FieldValue, Mutation, SplitMix64, Width};
use rusty_box_symbols_fixtures::pdb::{
    build_pdb, FixtureContainer, FixtureField, FixtureLine, FixtureOmap, FixtureOmapEntry, FixtureProcedure,
    FixturePublic, FixtureSection, FixtureType, FixtureUdt, PdbFixture, PublicKind, RecordForm, UdtKind,
    CODE_SECTION, DATA_SECTION,
};
use rusty_box_symbols_fixtures::pe::{loaded_image, pe32_file, rsds};

/// A case that runs longer than this is named (spec §12).
const SLOW: Duration = Duration::from_secs(1);

/// The cap every decompressing reader is given here.
const CAP: u64 = 1 << 24;

/// What a reader made of one input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parsed {
    /// It read, and `answers` questions found something.
    Read { answers: usize },
    Refused,
}

/// A file every variant of which a test feeds its readers.
struct Fixture {
    name: String,
    bytes: Vec<u8>,
    budget: Budget,
}

impl Fixture {
    fn new(name: &str, bytes: Vec<u8>) -> Self {
        Fixture { name: name.to_owned(), bytes, budget: Budget::STANDARD }
    }

    fn large(name: &str, bytes: Vec<u8>) -> Self {
        Fixture { name: name.to_owned(), bytes, budget: Budget::LARGE_OUTPUT }
    }
}

enum FailureKind {
    Panicked { message: String },
    Slow { elapsed: Duration },
}

struct Failure {
    fixture: String,
    case: usize,
    mutation: Mutation,
    kind: FailureKind,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{} ({}): ", self.fixture, self.case, self.mutation)?;
        match &self.kind {
            FailureKind::Panicked { message } => write!(f, "panicked: {message}"),
            FailureKind::Slow { elapsed } => write!(f, "took {elapsed:?}"),
        }
    }
}

/// Which cases of a fixture `CORRUPTION_CASE` selects.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    All,
    One(usize),
    None,
}

fn selection(fixture: &str) -> Selection {
    let Some(wanted) = std::env::var_os("CORRUPTION_CASE") else {
        return Selection::All;
    };
    let wanted = wanted.to_string_lossy().into_owned();
    match wanted.rsplit_once('#') {
        Some((name, case)) if name == fixture => match case.parse::<usize>() {
            Ok(case) => Selection::One(case),
            Err(error) => panic!("CORRUPTION_CASE={wanted}: the case is not a number: {error}"),
        },
        Some(_) => Selection::None,
        None => panic!("CORRUPTION_CASE={wanted}: expected <fixture>#<case>"),
    }
}

/// Runs every variant of `fixture` through `run`, adding a failure for each
/// case that panics or is slow.
fn survive<R>(fixture: &Fixture, run: &mut R, failures: &mut Vec<Failure>)
where
    R: FnMut(&[u8]) -> Parsed,
{
    let selected = selection(&fixture.name);
    let trace = std::env::var_os("CORRUPTION_TRACE").is_some();
    for (case, mutation) in variants(fixture.bytes.len(), fixture.budget).into_iter().enumerate() {
        match selected {
            Selection::All => {}
            Selection::One(wanted) if wanted == case => {}
            Selection::One(_) | Selection::None => continue,
        }
        if trace {
            eprintln!("{}#{case}: {mutation}", fixture.name);
        }
        let bytes = mutation.apply(&fixture.bytes);
        let started = Instant::now();
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| run(&bytes)));
        let elapsed = started.elapsed();
        let kind = match outcome {
            Ok(_) if elapsed <= SLOW => continue,
            Ok(_) => FailureKind::Slow { elapsed },
            Err(payload) => FailureKind::Panicked {
                message: payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "a panic without a message".to_owned()),
            },
        };
        failures.push(Failure { fixture: fixture.name.clone(), case, mutation, kind });
    }
}

/// Every variant of every fixture through `run`. The unmutated fixture must
/// read and answer something first: a fixture the readers refuse as it is
/// would make every variant's refusal meaningless.
fn assert_survives<R>(fixtures: &[Fixture], mut run: R)
where
    R: FnMut(&[u8]) -> Parsed,
{
    let mut failures = Vec::new();
    for fixture in fixtures {
        match run(&fixture.bytes) {
            Parsed::Read { answers } => assert!(answers > 0, "{}: the unmutated fixture answers nothing", fixture.name),
            Parsed::Refused => panic!("{}: the unmutated fixture does not read", fixture.name),
        }
        survive(fixture, &mut run, &mut failures);
    }
    if !failures.is_empty() {
        let shown: Vec<String> = failures.iter().take(20).map(ToString::to_string).collect();
        panic!(
            "{} corrupted cases failed (reproduce one with CORRUPTION_CASE=<fixture>#<case>):\n{}",
            failures.len(),
            shown.join("\n")
        );
    }
}

/// Addresses the fixtures name, and addresses no fixture names.
const PROBE_RVAS: [u64; 10] = [0, 0x1000, 0x1040, 0x1100, 0x1110, 0x1014, 0x3010, 0x40_1000, 0xFFFF_FFFF, u64::MAX];
const PROBE_NAMES: [&str; 9] = [
    "KiSystemCall64",
    "PsActiveProcessHead",
    "KeBugCheckEx",
    "widget_init",
    "helper_no_dwarf",
    "split_init",
    "dwz_function",
    "own_function",
    "",
];
const PROBE_TYPES: [&str; 11] =
    ["_EPROCESS", "_LIST_ENTRY", "Widget", "WidgetAlias", "Outer", "Holder", "Point", "Loop", "Local", "Shared", ""];

struct MemberProbe {
    type_name: &'static str,
    path: &'static [&'static str],
}

const PROBE_MEMBERS: [MemberProbe; 8] = [
    MemberProbe { type_name: "_EPROCESS", path: &["ActiveProcessLinks", "Blink"] },
    MemberProbe { type_name: "_EPROCESS", path: &["u", "Low"] },
    MemberProbe { type_name: "_EPROCESS", path: &["Flags"] },
    MemberProbe { type_name: "Outer", path: &["inner", "c"] },
    MemberProbe { type_name: "Holder", path: &["held", "y"] },
    MemberProbe { type_name: "Loop", path: &["m"] },
    MemberProbe { type_name: "Local", path: &["shared", "value"] },
    MemberProbe { type_name: "Widget", path: &["missing"] },
];

/// Asks `tables` every question the debugger asks, of names and addresses
/// the fixtures hold and ones they do not; returns how many found something.
fn ask(tables: &SymbolTables) -> usize {
    let names = PROBE_RVAS.iter().filter(|rva| tables.symbolize(Rva(**rva)).is_some()).count();
    let addresses = PROBE_NAMES.iter().filter(|name| tables.address_of(name).is_some()).count();
    let layouts = PROBE_TYPES
        .iter()
        .filter(|name| tables.layout(name).is_ok_and(|layout| layout.is_some()))
        .count();
    let members = PROBE_MEMBERS
        .iter()
        .filter(|probe| tables.member_location(probe.type_name, probe.path).is_ok_and(|location| location.is_some()))
        .count();
    names + addresses + layouts + members
}

/// What the service does with a file: its declared identity, its dwz link,
/// its tables, and every question of them.
fn read_symbol_file(bytes: &[u8]) -> Parsed {
    let declared = declared_identity(bytes).is_ok();
    let linked = supplement_of(bytes).is_ok();
    match SymbolTables::parse(bytes) {
        Ok(tables) => {
            let answers = ask(&tables);
            if declared && linked {
                Parsed::Read { answers }
            } else {
                Parsed::Refused
            }
        }
        Err(_) => Parsed::Refused,
    }
}

fn read_with_supplement(main: &[u8], supplement: &[u8]) -> Parsed {
    match SymbolTables::parse_with_supplement(main, Some(supplement)) {
        Ok(tables) => Parsed::Read { answers: ask(&tables) },
        Err(_) => Parsed::Refused,
    }
}

/// A guest reader over a flat image: every byte inside it reads.
fn read_flat(image: &[u8], rva: Rva, buf: &mut [u8]) -> ImageRead {
    let Ok(start) = usize::try_from(rva.0) else {
        return ImageRead::Unreadable;
    };
    match start.checked_add(buf.len()).and_then(|end| image.get(start..end)) {
        Some(bytes) => {
            buf.copy_from_slice(bytes);
            ImageRead::Read
        }
        None => ImageRead::Unreadable,
    }
}

/// A guest reader over a loaded image: a byte reads only where the loader
/// mapped one.
fn read_mapped(image: &[u8], mapped: &[bool], rva: Rva, buf: &mut [u8]) -> ImageRead {
    let Ok(start) = usize::try_from(rva.0) else {
        return ImageRead::Unreadable;
    };
    let all_mapped = start
        .checked_add(buf.len())
        .and_then(|end| mapped.get(start..end))
        .is_some_and(|bytes| bytes.iter().all(|byte| *byte));
    if all_mapped {
        read_flat(image, rva, buf)
    } else {
        ImageRead::Unreadable
    }
}

fn field(name: &str, offset: u64, ty: FixtureType) -> FixtureField {
    FixtureField { name: name.to_owned(), offset, ty }
}

/// A kernel-shaped PDB: a function and a data public, a procedure with
/// lines, and structures with every member kind (pointer, array, bitfield,
/// named structure, nested union).
fn rich_pdb(container: FixtureContainer, form: RecordForm) -> PdbFixture {
    let mut fixture = PdbFixture::new([0x11; 16], 3);
    fixture.container = container;
    fixture.form = form;
    fixture.publics.push(FixturePublic { name: "KiSystemCall64".into(), section: 1, offset: 0x40, kind: PublicKind::Function });
    fixture.publics.push(FixturePublic { name: "PsActiveProcessHead".into(), section: 2, offset: 0x10, kind: PublicKind::Data });
    fixture.procedures.push(FixtureProcedure {
        name: "KeBugCheckEx".into(),
        section: 1,
        offset: 0x100,
        len: 0x40,
        file: r"d:\os\ke\bugcheck.c".into(),
        lines: vec![FixtureLine { offset: 0, line: 10 }, FixtureLine { offset: 0x10, line: 12 }],
    });
    fixture.types.push(FixtureUdt {
        name: "_LIST_ENTRY".into(),
        kind: UdtKind::Struct,
        size: 16,
        fields: vec![field("Flink", 0, FixtureType::Pointer), field("Blink", 8, FixtureType::Pointer)],
    });
    fixture.types.push(FixtureUdt {
        name: "_EPROCESS".into(),
        kind: UdtKind::Struct,
        size: 0x40,
        fields: vec![
            field("Flags", 0, FixtureType::Bitfield { storage: Box::new(FixtureType::U32), position: 3, length: 2 }),
            field("ImageFileName", 8, FixtureType::Array { element: Box::new(FixtureType::U8), count: 15 }),
            field("ActiveProcessLinks", 0x18, FixtureType::Named("_LIST_ENTRY".into())),
            field(
                "u",
                0x28,
                FixtureType::Nested(Box::new(FixtureUdt {
                    name: "<unnamed-tag>".into(),
                    kind: UdtKind::Union,
                    size: 8,
                    fields: vec![field("Value", 0, FixtureType::U64), field("Low", 0, FixtureType::U16)],
                })),
            ),
            field("Count", 0x30, FixtureType::U32),
        ],
    });
    fixture
}

fn pdb_fixtures() -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    for container in [
        FixtureContainer::Msf,
        FixtureContainer::MsfzZstd,
        FixtureContainer::MsfzDeflate,
        FixtureContainer::MsfzSmallChunks,
        FixtureContainer::MsfzStored,
    ] {
        fixtures.push(Fixture::new(&format!("pdb {container:?}"), build_pdb(&rich_pdb(container, RecordForm::Classic))));
    }
    fixtures.push(Fixture::new("pdb Msf, *2 records", build_pdb(&rich_pdb(FixtureContainer::Msf, RecordForm::Wide))));
    let mut rewritten = rich_pdb(FixtureContainer::Msf, RecordForm::Classic);
    rewritten.omap = Some(FixtureOmap {
        original_sections: vec![
            FixtureSection { name: ".text", rva: 0x1000, size: 0x2000, characteristics: CODE_SECTION },
            FixtureSection { name: ".data", rva: 0x3000, size: 0x2000, characteristics: DATA_SECTION },
        ],
        from_source: vec![
            FixtureOmapEntry { from: 0x1000, to: 0x1800 },
            FixtureOmapEntry { from: 0x1100, to: 0 },
            FixtureOmapEntry { from: 0x3000, to: 0x3000 },
        ],
    });
    rewritten.broken_udt_records = 2;
    fixtures.push(Fixture::new("pdb Msf, OMAP and broken records", build_pdb(&rewritten)));
    fixtures
}

/// `sections` stored as `encoding` says.
fn stored_as(sections: Vec<ExtraSection>, encoding: Encoding) -> Vec<ExtraSection> {
    sections.into_iter().map(|section| ExtraSection { encoding, ..section }).collect()
}

fn elf_fixtures() -> Vec<Fixture> {
    let pie = ElfSpec::pie();
    let dwarf = |version, format| build_dwarf(DwarfSpec { version, format, text_va: TEXT_RVA });
    let exec_dwarf = build_dwarf(DwarfSpec { version: 4, format: DwarfFormat::Dwarf32, text_va: 0x40_1000 });
    vec![
        Fixture::new("elf: .symtab only", build_elf(pie, &[])),
        Fixture::new("elf32: .symtab only", build_elf(ElfSpec::pie32(), &[])),
        Fixture::new("elf: DWARF 4", build_elf(pie, &dwarf(4, DwarfFormat::Dwarf32))),
        Fixture::new("elf: DWARF 5", build_elf(pie, &dwarf(5, DwarfFormat::Dwarf32))),
        Fixture::new("elf: DWARF 5, 64-bit, zlib", build_elf(pie, &stored_as(dwarf(5, DwarfFormat::Dwarf64), Encoding::Zlib))),
        Fixture::new("elf: DWARF 4, zstd", build_elf(pie, &stored_as(dwarf(4, DwarfFormat::Dwarf32), Encoding::Zstd))),
        Fixture::new("elf: DWARF 4, .zdebug, executable", build_elf(ElfSpec::exec(), &stored_as(exec_dwarf, Encoding::Zdebug))),
        Fixture::new("elf: --only-keep-debug", build_elf(ElfSpec { text: Text::NoBits, ..pie }, &dwarf(4, DwarfFormat::Dwarf32))),
        Fixture::new("elf: DWARF 5 split forms", build_elf(pie, &split_forms_dwarf(TEXT_RVA))),
        Fixture::new("elf: looping typedefs", build_elf(pie, &looping_typedefs())),
    ]
}

#[test]
fn the_generator_and_the_variants_are_fixed() {
    // SplitMix64's published first output for seed 0.
    assert_eq!(SplitMix64::new(0).next_u64(), 0xE220_A839_7B1D_CDAF);
    assert_eq!(variants(4096, Budget::STANDARD), variants(4096, Budget::STANDARD));
}

#[test]
fn every_kind_of_variant_is_made_and_applies_in_bounds() {
    let original: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    let all = variants(original.len(), Budget::STANDARD);
    assert!(all.iter().any(|mutation| matches!(mutation, Mutation::FlipBit { .. })));
    assert!(all.contains(&Mutation::Truncate { len: 0 }));
    for width in Width::ALL {
        for value in FieldValue::ALL {
            assert!(
                all.iter().any(|mutation| matches!(mutation, Mutation::SetField { width: w, value: v, .. } if *w == width && *v == value)),
                "{width:?} {value:?}"
            );
        }
    }
    for mutation in &all {
        let mutated = mutation.apply(&original);
        match mutation {
            Mutation::Truncate { len } => assert_eq!(mutated.len(), *len),
            Mutation::FlipBit { .. } | Mutation::SetField { .. } => assert_eq!(mutated.len(), original.len()),
        }
    }
    // One past a field's own value wraps inside the field: byte 0xFF becomes 0.
    let past = Mutation::SetField { at: 255, width: Width::U8, value: FieldValue::PastOwn }.apply(&original);
    assert_eq!(past[255], 0);
    assert_eq!(past[256], original[256]);
}

#[test]
fn pdb_files_survive_corruption() {
    assert_survives(&pdb_fixtures(), read_symbol_file);
}

#[test]
fn elf_files_survive_corruption() {
    assert_survives(&elf_fixtures(), read_symbol_file);
}

#[test]
fn dwz_pairs_survive_corruption() {
    for forms in [DwzForms::Gnu, DwzForms::Standard] {
        let pair = dwz_pair(forms, TEXT_RVA);
        let main = build_elf(ElfSpec { symtab: Symtab::Without, ..ElfSpec::pie() }, &pair.main);
        let supplement = pair.supplement;
        assert_survives(&[Fixture::new(&format!("dwz {forms:?}: main"), main.clone())], |bytes| {
            read_with_supplement(bytes, &supplement)
        });
        assert_survives(&[Fixture::new(&format!("dwz {forms:?}: supplement"), supplement.clone())], |bytes| {
            read_with_supplement(&main, bytes)
        });
    }
}

#[test]
fn pe_files_and_images_survive_corruption() {
    let codeview = rsds([0x11; 16], 3, r"d:\os\obj\amd64\ntkrnlmp.pdb");
    let wanted = PeImageId { time_date_stamp: 0x0BAD_F00D, size_of_image: 0x3000 };
    assert_survives(&[Fixture::new("pe file", pe32_file(&codeview))], |bytes| match pdb_from_binary(bytes, wanted) {
        Ok(_) => Parsed::Read { answers: 1 },
        Err(_) => Parsed::Refused,
    });
    assert_survives(&[Fixture::new("pe image", loaded_image(&codeview))], |bytes| {
        let mut read = |rva: Rva, buf: &mut [u8]| read_flat(bytes, rva, buf);
        match read_pe_identity(&mut read) {
            Ok(_) => Parsed::Read { answers: 1 },
            Err(_) => Parsed::Refused,
        }
    });
}

/// An ELF image a guest loaded, by its fixture spec.
struct ImageCase {
    name: &'static str,
    spec: ElfSpec,
}

#[test]
fn elf_images_survive_corruption() {
    for case in [
        ImageCase { name: "elf image: PIE", spec: ElfSpec::pie() },
        ImageCase { name: "elf image: executable", spec: ElfSpec::exec() },
        ImageCase { name: "elf image: 32-bit PIE", spec: ElfSpec::pie32() },
    ] {
        let loaded = load_image(&build_elf(case.spec, &[]), case.spec.link_base);
        let mapped: Vec<bool> = loaded.iter().map(Option::is_some).collect();
        let flat: Vec<u8> = loaded.iter().map(|byte| byte.unwrap_or(0)).collect();
        assert_survives(&[Fixture::new(case.name, flat)], |bytes| {
            let mut read = |rva: Rva, buf: &mut [u8]| read_mapped(bytes, &mapped, rva, buf);
            match identity_from_image(&mut read) {
                Ok(_) => Parsed::Read { answers: 1 },
                Err(_) => Parsed::Refused,
            }
        });
    }
}

#[test]
fn cabinets_survive_corruption() {
    let pdb = build_pdb(&rich_pdb(FixtureContainer::Msf, RecordForm::Classic));
    let fixtures = [
        Fixture::new("cab MSZIP", cabinet("ntkrnlmp.pdb", &pdb, CabinetKind::MsZip)),
        Fixture::new("cab stored", cabinet("ntkrnlmp.pdb", &pdb, CabinetKind::Stored)),
        Fixture::large("cab LZX 15", LZX15_CABINET.to_vec()),
        Fixture::large("cab LZX 21", LZX21_CABINET.to_vec()),
    ];
    assert_survives(&fixtures, |bytes| {
        if !is_cabinet(bytes) {
            return Parsed::Refused;
        }
        match extract_single(bytes, CAP) {
            Ok(extracted) => Parsed::Read { answers: extracted.bytes.len() },
            Err(_) => Parsed::Refused,
        }
    });
}

/// A compressed stream and what it declares.
struct StreamCase {
    name: &'static str,
    codec: Codec,
    bytes: Vec<u8>,
    declared: u64,
}

#[test]
fn compressed_streams_survive_corruption() {
    let text = sample_text(8_192);
    let noise = sample_binary(8_192, 3);
    let declared = text.len() as u64;
    let cases = [
        StreamCase { name: "zlib", codec: Codec::Zlib, bytes: zlib(&text, 6), declared },
        StreamCase { name: "raw deflate", codec: Codec::RawDeflate, bytes: raw_deflate(&noise, 9), declared },
        StreamCase {
            name: "zstd with checksum",
            codec: Codec::Zstd,
            bytes: zstd_frame(&text, ZstdOptions { level: 19, checksum: Checksum::With, window_log: 0 }),
            declared,
        },
        StreamCase {
            name: "zstd without checksum",
            codec: Codec::Zstd,
            bytes: zstd_frame(&noise, ZstdOptions { level: 1, checksum: Checksum::Without, window_log: 0 }),
            declared,
        },
    ];
    for case in cases {
        assert_survives(&[Fixture::new(case.name, case.bytes)], |bytes| {
            match decompress_to_vec(case.codec, bytes, case.declared, CAP) {
                Ok(out) => Parsed::Read { answers: out.len() },
                Err(_) => Parsed::Refused,
            }
        });
    }
}
