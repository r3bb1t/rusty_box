//! What a file of debug information answers, whatever its format: names
//! for addresses, addresses for names, and structure layouts; and why a
//! file did not parse.

use crate::dwarf::tables::{elf_identity, DwarfTables};
use crate::elf::{AltLink, ElfFile};
use crate::identity::TablesIdentity;
use crate::pdb::tables::{pdb_identity, PdbTables};
use crate::pdb::{PdbError, MSFZ_MAGIC, MSF_MAGIC, PDB2_MAGIC};
use crate::units::{BitRange, FieldOffset, Rva};

/// A source position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLine {
    pub file: String,
    pub line: u32,
}

/// The hash a compiler recorded of a source file's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumKind {
    Md5,
    Sha1,
    Sha256,
}

impl ChecksumKind {
    /// The digest's length in bytes.
    #[must_use]
    pub const fn digest_len(self) -> usize {
        match self {
            ChecksumKind::Md5 => 16,
            ChecksumKind::Sha1 => 20,
            ChecksumKind::Sha256 => 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChecksum {
    pub kind: ChecksumKind,
    /// `kind.digest_len()` bytes.
    pub bytes: Vec<u8>,
}

impl FileChecksum {
    /// A digest of kind `kind`, when `bytes` is that kind's length.
    #[must_use]
    pub fn of(kind: ChecksumKind, bytes: &[u8]) -> Option<FileChecksum> {
        (bytes.len() == kind.digest_len()).then(|| FileChecksum { kind, bytes: bytes.to_vec() })
    }
}

/// A source file the tables name, as the build recorded its path, and the
/// checksum recorded with it, when there is one of a kind this reader knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    pub path: String,
    pub checksum: Option<FileChecksum>,
}

/// A source file a PDB carries inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedSource {
    /// The file's bytes, as they are on the build machine.
    Plain(Vec<u8>),
    /// Stored compressed, by a scheme this reader does not undo: the
    /// entry's compression byte.
    Compressed { kind: u8 },
}

/// The code one source line compiled to, in one place: `start..end`, and
/// the line it is, in file `file` (an index into [`SymbolTables::files`]).
/// A line may have several rows (a loop header, inlined or reordered code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineRow {
    pub start: Rva,
    pub end: Rva,
    pub file: usize,
    pub line: u32,
}

/// A source line resolved to code in one file: the line that has the code
/// (the asked line, or the next line of its function when the asked one has
/// none), and every row of it, by start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLine {
    pub line: u32,
    pub ranges: Vec<LineRow>,
}

/// A function's code, `start..end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcedureSpan {
    pub name: String,
    pub start: Rva,
    pub end: Rva,
}

/// An address named: the function or symbol it lies in, how far into it,
/// and the source line, when the tables have line information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolHit {
    pub name: String,
    pub offset: u64,
    pub line: Option<SourceLine>,
}

/// One member of a structure or union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: String,
    pub offset: FieldOffset,
    /// Set for a bitfield: its bits inside the storage unit at `offset`.
    pub bits: Option<BitRange>,
}

/// A structure or union: its size and its members, in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeLayout {
    pub size: u64,
    pub fields: Vec<FieldLayout>,
}

/// Where a member reached through a path of member names sits, from the
/// start of the outermost structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberLocation {
    pub offset: FieldOffset,
    pub bits: Option<BitRange>,
    /// The member's size in bytes, when its type has one (a bitfield's is
    /// its storage unit's).
    pub size: Option<u64>,
}

/// Why a file of debug information did not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Pdb(#[from] PdbError),
    #[error("no memory for the {what}")]
    Allocation { what: &'static str },
    #[error(transparent)]
    Elf(#[from] crate::elf::ElfError),
    #[error(transparent)]
    Dwarf(#[from] crate::dwarf::DwarfError),
    #[error("the file is neither a PDB nor an ELF file")]
    NotSymbols,
}

/// Why a layout question has no answer that can be trusted.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("the type records do not read: {what}")]
    Unreadable { what: &'static str },
    /// The type was not found, and the file holds type records this reader
    /// could not read, so it may be among them: the caller must not take
    /// the miss as an answer.
    #[error("{name} was not found, and {count} structure records could not be read")]
    UnreadableTypeRecords { name: String, count: u32 },
    #[error("the field list of {name} does not end")]
    EndlessFieldList { name: String },
    /// `index` is a PDB type index, or a DWARF entry's section offset.
    #[error("type {index:#x} is reached through a chain that does not end")]
    EndlessTypeChain { index: u64 },
    #[error("{member} is reached through a member that is not a structure or union")]
    NotAStructure { member: String },
    #[error("{name}'s size or a member's offset is not a non-negative integer")]
    BadNumber { name: String },
    #[error("the field list of {name} holds a record of kind {leaf:#06x} this reader cannot step over")]
    UnknownFieldRecord { name: String, leaf: u16 },
    #[error("{member}'s location is an expression this reader does not evaluate")]
    UnsupportedMemberLocation { member: String },
    /// A dwz debug file names a type in its supplementary file, which is
    /// not at hand.
    #[error("the type at offset {offset:#x} of the dwz supplementary file is needed, and that file is not loaded")]
    MissingSupplement { offset: u64 },
}

/// Pushes `item` after reserving room for it, so the push cannot reach
/// `Vec`'s capacity-overflow panic.
pub(crate) fn push<T>(items: &mut Vec<T>, item: T, what: &'static str) -> Result<(), ParseError> {
    items.try_reserve(1).map_err(|_| ParseError::Allocation { what })?;
    items.push(item);
    Ok(())
}

const ELF_MAGIC: &[u8] = b"\x7FELF";

/// Which reader a file's first bytes call for.
enum FileFormat {
    Pdb,
    Elf,
}

fn sniff(bytes: &[u8]) -> Result<FileFormat, ParseError> {
    if bytes.starts_with(MSF_MAGIC) || bytes.starts_with(MSFZ_MAGIC) {
        Ok(FileFormat::Pdb)
    } else if bytes.starts_with(ELF_MAGIC) {
        Ok(FileFormat::Elf)
    } else if bytes.starts_with(PDB2_MAGIC) {
        Err(ParseError::Pdb(PdbError::Pdb2))
    } else {
        Err(ParseError::NotSymbols)
    }
}

/// The identity a file of debug information declares, read without
/// parsing its tables: what a project folder is indexed by.
pub fn declared_identity(bytes: &[u8]) -> Result<TablesIdentity, ParseError> {
    match sniff(bytes)? {
        FileFormat::Pdb => pdb_identity(bytes),
        FileFormat::Elf => elf_identity(bytes),
    }
}

/// The dwz supplementary file an ELF debug file names, if any.
pub fn supplement_of(bytes: &[u8]) -> Result<Option<AltLink>, ParseError> {
    match sniff(bytes)? {
        FileFormat::Pdb => Ok(None),
        FileFormat::Elf => Ok(ElfFile::parse(bytes)?.alt_link()?),
    }
}

/// A file of debug information, of either format. Both answer the same
/// questions, so a consumer never asks which it holds. Each format's
/// tables are hundreds of bytes of headers, of sizes that differ, so both
/// live behind a box and the enum is a pointer and a tag.
pub enum SymbolTables {
    Pdb(Box<PdbTables>),
    Dwarf(Box<DwarfTables>),
}

impl SymbolTables {
    pub fn parse(bytes: &[u8]) -> Result<SymbolTables, ParseError> {
        SymbolTables::parse_with_supplement(bytes, None)
    }

    /// An ELF file may come with the dwz supplement it names; a PDB never
    /// has one, so `supplement` is ignored for it.
    pub fn parse_with_supplement(bytes: &[u8], supplement: Option<&[u8]>) -> Result<SymbolTables, ParseError> {
        match sniff(bytes)? {
            FileFormat::Pdb => Ok(SymbolTables::Pdb(Box::new(PdbTables::parse(bytes)?))),
            FileFormat::Elf => Ok(SymbolTables::Dwarf(Box::new(DwarfTables::parse_with_supplement(bytes, supplement)?))),
        }
    }

    #[must_use]
    pub fn identity(&self) -> TablesIdentity {
        match self {
            SymbolTables::Pdb(tables) => tables.identity(),
            SymbolTables::Dwarf(tables) => tables.identity(),
        }
    }

    #[must_use]
    pub fn symbolize(&self, rva: Rva) -> Option<SymbolHit> {
        match self {
            SymbolTables::Pdb(tables) => tables.symbolize(rva),
            SymbolTables::Dwarf(tables) => tables.symbolize(rva),
        }
    }

    #[must_use]
    pub fn address_of(&self, name: &str) -> Option<Rva> {
        match self {
            SymbolTables::Pdb(tables) => tables.address_of(name),
            SymbolTables::Dwarf(tables) => tables.address_of(name),
        }
    }

    /// Every source file the line tables name, each path once; a line
    /// answer's file index is an index into this list.
    #[must_use]
    pub fn files(&self) -> &[SourceFile] {
        match self {
            SymbolTables::Pdb(tables) => tables.files(),
            SymbolTables::Dwarf(tables) => tables.files(),
        }
    }

    /// The source of file `file` (an index into [`SymbolTables::files`]),
    /// when the PDB embeds it; DWARF tables embed none.
    #[must_use]
    pub fn embedded_source(&self, file: usize) -> Option<&EmbeddedSource> {
        match self {
            SymbolTables::Pdb(tables) => tables.embedded_source(file),
            SymbolTables::Dwarf(_) => None,
        }
    }

    /// The line row whose code holds `rva`.
    #[must_use]
    pub fn line_at(&self, rva: Rva) -> Option<LineRow> {
        match self {
            SymbolTables::Pdb(tables) => tables.line_at(rva),
            SymbolTables::Dwarf(tables) => tables.line_at(rva),
        }
    }

    /// Line `line` of every file whose path ends with `file`'s components
    /// (see [`crate::source_path`]), one answer per such file that has code
    /// for it: the line's rows, or, for a line with no code, the rows of the
    /// next line of the function the line lies in. A line in no function's
    /// lines resolves to nothing.
    #[must_use]
    pub fn lines_of(&self, file: &str, line: u32) -> Vec<ResolvedLine> {
        match self {
            SymbolTables::Pdb(tables) => tables.lines_of(file, line),
            SymbolTables::Dwarf(tables) => tables.lines_of(file, line),
        }
    }

    /// The lines of file `file` (a place in [`SymbolTables::files`]) that
    /// have code, ascending.
    #[must_use]
    pub fn code_lines(&self, file: usize) -> Vec<u32> {
        match self {
            SymbolTables::Pdb(tables) => tables.code_lines(file),
            SymbolTables::Dwarf(tables) => tables.code_lines(file),
        }
    }

    /// The function whose code holds `rva`: a PDB procedure or a DWARF
    /// subprogram, never a public or `.symtab` symbol (they have no end).
    #[must_use]
    pub fn procedure_at(&self, rva: Rva) -> Option<ProcedureSpan> {
        match self {
            SymbolTables::Pdb(tables) => tables.procedure_at(rva),
            SymbolTables::Dwarf(tables) => tables.procedure_at(rva),
        }
    }

    /// Where every function starts, ascending, each once: the PDB's
    /// procedures, or the DWARF subprograms.
    #[must_use]
    pub fn procedure_starts(&self) -> Vec<Rva> {
        match self {
            SymbolTables::Pdb(tables) => tables.procedure_starts(),
            SymbolTables::Dwarf(tables) => tables.procedure_starts(),
        }
    }

    pub fn layout(&self, type_name: &str) -> Result<Option<TypeLayout>, LayoutError> {
        match self {
            SymbolTables::Pdb(tables) => tables.layout(type_name),
            SymbolTables::Dwarf(tables) => tables.layout(type_name),
        }
    }

    pub fn member_location(&self, type_name: &str, path: &[&str]) -> Result<Option<MemberLocation>, LayoutError> {
        match self {
            SymbolTables::Pdb(tables) => tables.member_location(type_name, path),
            SymbolTables::Dwarf(tables) => tables.member_location(type_name, path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{BuildId, Guid};
    use crate::units::Rva;
    use rusty_box_symbols_fixtures::dwarf::{build_dwarf, dwz_pair, DwarfFormat, DwarfSpec, DwzForms, SUPPLEMENT_BUILD_ID};
    use rusty_box_symbols_fixtures::elf::{build_elf, ElfSpec, BUILD_ID, TEXT_RVA};
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, FixtureContainer, FixtureLine, FixtureProcedure, FixturePublic, PdbFixture, PublicKind,
    };

    fn pdb(container: FixtureContainer) -> Vec<u8> {
        let mut fixture = PdbFixture::new([3; 16], 2);
        fixture.container = container;
        fixture.publics.push(FixturePublic { name: "KeBugCheckEx".into(), section: 1, offset: 0x40, kind: PublicKind::Function });
        build_pdb(&fixture)
    }

    fn elf() -> Vec<u8> {
        let spec = ElfSpec::pie();
        build_elf(spec, &build_dwarf(DwarfSpec { version: 5, format: DwarfFormat::Dwarf32, text_va: TEXT_RVA }))
    }

    #[test]
    fn either_format_is_recognised_by_its_bytes() {
        for container in [FixtureContainer::Msf, FixtureContainer::MsfzZstd] {
            let tables = SymbolTables::parse(&pdb(container)).unwrap();
            assert!(matches!(tables, SymbolTables::Pdb(_)), "{container:?}");
            assert_eq!(tables.address_of("KeBugCheckEx"), Some(Rva(0x1040)));
            assert_eq!(tables.identity(), TablesIdentity::Pdb { guid: Guid::from_le_bytes([3; 16]), age: 2 });
        }
        let tables = SymbolTables::parse(&elf()).unwrap();
        assert!(matches!(tables, SymbolTables::Dwarf(_)));
        assert_eq!(
            tables.files().iter().map(|file| file.path.as_str()).collect::<Vec<&str>>(),
            ["/build/widget/src/main.c"]
        );
        assert_eq!(tables.line_at(Rva(TEXT_RVA)).map(|row| row.line), Some(10));
        assert_eq!(tables.lines_of("main.c", 11).first().map(|resolved| resolved.line), Some(11));
        assert_eq!(tables.procedure_at(Rva(TEXT_RVA)).map(|span| span.name), Some("widget_init".to_owned()));
        assert_eq!(tables.symbolize(Rva(TEXT_RVA)).map(|hit| hit.name), Some("widget_init".to_owned()));
        assert_eq!(tables.layout("Widget").unwrap().map(|layout| layout.size), Some(16));
    }

    #[test]
    fn procedure_starts_list_every_function() {
        let procedure = |name: &str, offset: u32| FixtureProcedure {
            name: name.into(),
            section: 1,
            offset,
            len: 0x10,
            file: r"C:\src\a.c".into(),
            checksum: None,
            lines: vec![FixtureLine { offset: 0, line: 1 }],
        };
        let mut fixture = PdbFixture::new([4; 16], 1);
        fixture.procedures.push(procedure("second", 0x200));
        fixture.procedures.push(procedure("first", 0x100));
        let tables = SymbolTables::parse(&build_pdb(&fixture)).unwrap();
        assert_eq!(tables.procedure_starts(), [Rva(0x1100), Rva(0x1200)]);
        let dwarf = SymbolTables::parse(&elf()).unwrap().procedure_starts();
        assert_eq!(dwarf.first(), Some(&Rva(TEXT_RVA)));
        assert!(dwarf.windows(2).all(|pair| pair[0] < pair[1]), "ascending, each once");
    }

    #[test]
    fn a_pdb_2_file_and_other_bytes_are_refused_by_name() {
        let mut old = crate::pdb::PDB2_MAGIC.to_vec();
        old.extend_from_slice(&[0; 64]);
        assert!(matches!(SymbolTables::parse(&old), Err(ParseError::Pdb(PdbError::Pdb2))));
        assert!(matches!(SymbolTables::parse(b"MZ\x90\x00"), Err(ParseError::NotSymbols)));
        assert_eq!(declared_identity(&old), Err(ParseError::Pdb(PdbError::Pdb2)));
        assert_eq!(declared_identity(b""), Err(ParseError::NotSymbols));
    }

    #[test]
    fn a_file_declares_its_identity_without_its_tables_being_read() {
        assert_eq!(
            declared_identity(&pdb(FixtureContainer::MsfzDeflate)),
            Ok(TablesIdentity::Pdb { guid: Guid::from_le_bytes([3; 16]), age: 2 })
        );
        assert_eq!(declared_identity(&elf()), Ok(TablesIdentity::Elf { build_id: Some(BuildId(BUILD_ID.to_vec())) }));
    }

    #[test]
    fn a_dwz_file_names_its_supplement() {
        let pair = dwz_pair(DwzForms::Gnu, TEXT_RVA);
        let main = build_elf(ElfSpec::pie(), &pair.main);
        let link = supplement_of(&main).unwrap().expect("a .gnu_debugaltlink");
        assert_eq!(link.build_id, BuildId(SUPPLEMENT_BUILD_ID.to_vec()));
        assert_eq!(link.file_name, "fixture.debug");
        assert_eq!(supplement_of(&elf()).unwrap(), None);
        assert_eq!(supplement_of(&pdb(FixtureContainer::Msf)).unwrap(), None);
        let tables = SymbolTables::parse_with_supplement(&main, Some(&pair.supplement)).unwrap();
        assert_eq!(tables.layout("Shared").unwrap().map(|layout| layout.size), Some(8));
    }

    #[test]
    fn the_tables_are_shared_across_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<SymbolTables>();
    }
}
