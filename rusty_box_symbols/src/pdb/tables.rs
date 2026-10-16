//! A PDB's names, procedures and line ranges, read from its streams into
//! owned, sorted tables (tier A): the info stream gives the GUID, the DBI
//! stream the age, the module list and the section and OMAP streams; the
//! symbol-record stream gives public and data names; each module stream its
//! procedures and C13 line tables. Every loop walks records of a stream
//! already in memory and moves forward on each step.

use std::collections::BTreeMap;

use super::dbi::{self, OmapEntry};
use super::info;
use super::lines::{self, DEBUG_S_FILECHKSMS, DEBUG_S_LINES};
use super::symbols::{self, S_GDATA32, S_GPROC32, S_GPROC32_ID, S_LDATA32, S_LPROC32, S_LPROC32_ID, S_PUB32};
use super::{short, PdbError, PdbFile};
use crate::identity::{Guid, TablesIdentity};
use crate::read::Bytes;
use crate::tables::{push, LayoutError, MemberLocation, ParseError, SourceLine, SymbolHit, TypeLayout};
use crate::units::Rva;

/// Modules past this are not a real PDB's (a browser's has tens of
/// thousands).
const MAX_MODULES: usize = 1 << 20;
/// The PDB info stream and the DBI stream.
const INFO_STREAM: u32 = 1;
const DBI_STREAM: u32 = 3;
const NO_STREAM: u16 = 0xFFFF;

/// A named address.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Named {
    name: String,
    rva: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Procedure {
    name: String,
    rva: u64,
    len: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineRange {
    rva: u64,
    len: u64,
    file: usize,
    line: u32,
}

/// Maps a section and offset to an RVA: through the image's own section
/// headers, or, for a binary rewritten after linking, through the original
/// section headers and the OMAP table.
struct Addresses {
    /// Each section's RVA, by CodeView section number minus one.
    sections: Vec<u64>,
    omap: Option<Vec<OmapEntry>>,
}

impl Addresses {
    fn rva(&self, segment: u16, offset: u32) -> Option<u64> {
        let index = usize::from(segment).checked_sub(1)?;
        let source = self.sections.get(index)?.checked_add(u64::from(offset))?;
        match &self.omap {
            None => Some(source),
            Some(map) => dbi::omap_translate(map, source),
        }
    }
}

/// A PDB's identity, names, procedures and line ranges.
pub struct PdbTables {
    guid: Guid,
    age: u32,
    /// Sorted by RVA.
    publics: Vec<Named>,
    /// Publics then procedures, sorted by name.
    by_name: Vec<Named>,
    /// Sorted by RVA.
    procedures: Vec<Procedure>,
    /// Sorted by RVA.
    lines: Vec<LineRange>,
    files: Vec<String>,
    types: Result<super::tpi::TypeTables, LayoutError>,
}

/// The stream `index`, which the PDB must have.
fn required(file: &mut PdbFile<'_>, index: u32, what: &'static str) -> Result<Vec<u8>, PdbError> {
    file.stream(index)?.ok_or(PdbError::MissingStream { what })
}

/// A stream named by an optional index; absent when the index is.
fn optional(file: &mut PdbFile<'_>, index: Option<u16>) -> Result<Option<Vec<u8>>, PdbError> {
    match index {
        Some(index) => file.stream(u32::from(index)),
        None => Ok(None),
    }
}

/// The identity a PDB declares, read from its info and DBI streams alone.
pub fn pdb_identity(bytes: &[u8]) -> Result<TablesIdentity, ParseError> {
    let mut file = PdbFile::open(bytes)?;
    let info_stream = required(&mut file, INFO_STREAM, "PDB info")?;
    let head = info::decode_head(Bytes::new(&info_stream)).map_err(|source| short("PDB info", source))?;
    let dbi_stream = required(&mut file, DBI_STREAM, "DBI")?;
    let header = dbi::decode_header(Bytes::new(&dbi_stream)).map_err(|source| short("DBI header", source))?;
    Ok(TablesIdentity::Pdb { guid: head.guid, age: header.age })
}

fn text(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

impl PdbTables {
    /// Parses a whole PDB file held in memory.
    pub fn parse(bytes: &[u8]) -> Result<PdbTables, ParseError> {
        let mut file = PdbFile::open(bytes)?;
        let info_stream = required(&mut file, INFO_STREAM, "PDB info")?;
        let head = info::decode_head(Bytes::new(&info_stream)).map_err(|source| short("PDB info", source))?;
        let dbi_stream = required(&mut file, DBI_STREAM, "DBI")?;
        let dbi_bytes = Bytes::new(&dbi_stream);
        let header = dbi::decode_header(dbi_bytes).map_err(|source| short("DBI header", source))?;
        let streams = dbi::decode_optional_streams(dbi_bytes, &header);
        let addresses = match (streams.omap_from_source, streams.original_section_headers) {
            (Some(map), Some(original)) => {
                let sections = optional(&mut file, Some(original))?.unwrap_or_default();
                let map = optional(&mut file, Some(map))?.unwrap_or_default();
                Addresses { sections: dbi::section_rvas(&sections)?, omap: Some(dbi::omap(&map)?) }
            }
            (Some(_) | None, Some(_) | None) => {
                let sections = optional(&mut file, streams.section_headers)?.unwrap_or_default();
                Addresses { sections: dbi::section_rvas(&sections)?, omap: None }
            }
        };
        // TPI is stream 2. A PDB whose type records do not read still names
        // addresses; only its layouts answer with the error.
        let types = match file.stream(2) {
            Ok(Some(stream)) => super::tpi::TypeTables::parse(stream),
            Ok(None) => Err(LayoutError::Unreadable { what: "TPI stream (nil)" }),
            Err(_) => Err(LayoutError::Unreadable { what: "TPI stream" }),
        };
        let mut tables = PdbTables {
            guid: head.guid,
            age: header.age,
            publics: Vec::new(),
            by_name: Vec::new(),
            procedures: Vec::new(),
            lines: Vec::new(),
            files: Vec::new(),
            types,
        };
        if header.symbol_records != NO_STREAM {
            if let Some(records) = file.stream(u32::from(header.symbol_records))? {
                tables.read_publics(&records, &addresses)?;
            }
        }
        let names = match info::named_stream(&info_stream, b"/names")? {
            Some(index) => file.stream(index)?.unwrap_or_default(),
            None => Vec::new(),
        };
        let span = dbi::modules_span(&header);
        let modules = dbi_bytes.sub(span.at, span.len).map_err(|source| short("module list", source))?;
        let mut at = 0usize;
        let mut count = 0usize;
        let mut file_index = BTreeMap::new();
        while at < modules.len() {
            count = count.saturating_add(1);
            if count > MAX_MODULES {
                return Err(PdbError::TooMany { what: "modules", count: count as u64 }.into());
            }
            let entry = dbi::decode_module(modules, at).map_err(|source| short("module entry", source))?;
            at = entry.next;
            if entry.info.stream == NO_STREAM {
                continue;
            }
            let Some(stream) = file.stream(u32::from(entry.info.stream))? else {
                continue;
            };
            tables.read_module(&stream, &entry.info, &addresses, Bytes::new(&names), &mut file_index)?;
        }
        tables.publics.sort_unstable_by_key(|named| named.rva);
        tables.procedures.sort_unstable_by_key(|procedure| procedure.rva);
        tables.lines.sort_unstable_by_key(|range| range.rva);
        for procedure in &tables.procedures {
            push(&mut tables.by_name, Named { name: procedure.name.clone(), rva: procedure.rva }, "name index")?;
        }
        // Unstable sorts take no scratch buffer; the address breaks a tie
        // between namesakes as the procedures' own order would.
        tables.by_name.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.rva.cmp(&b.rva)));
        Ok(tables)
    }

    /// `S_PUB32`, `S_GDATA32` and `S_LDATA32` of the symbol-record stream.
    fn read_publics(&mut self, records: &[u8], addresses: &Addresses) -> Result<(), ParseError> {
        let records = Bytes::new(records);
        let mut at = 0usize;
        while at < records.len() {
            let record = symbols::decode_record_header(records, at)
                .map_err(|_| PdbError::BadRecord { what: "symbol", at })?;
            at = record.next;
            if !matches!(record.kind, S_PUB32 | S_GDATA32 | S_LDATA32) {
                continue;
            }
            let named = symbols::decode_named_address(record.data)
                .map_err(|_| PdbError::BadRecord { what: "public symbol", at })?;
            let Some(rva) = addresses.rva(named.segment, named.offset) else {
                continue;
            };
            let name = text(named.name);
            push(&mut self.by_name, Named { name: name.clone(), rva }, "name index")?;
            push(&mut self.publics, Named { name, rva }, "publics")?;
        }
        Ok(())
    }

    /// A module's procedures and C13 line ranges.
    fn read_module(
        &mut self,
        stream: &[u8],
        module: &dbi::ModuleInfo,
        addresses: &Addresses,
        names: Bytes<'_>,
        file_index: &mut BTreeMap<u32, usize>,
    ) -> Result<(), ParseError> {
        let stream = Bytes::new(stream);
        // Symbols follow the 4-byte CodeView signature.
        let symbols_len = (module.symbol_bytes as usize).saturating_sub(4);
        let records = stream.sub(4, symbols_len).map_err(|source| short("module symbols", source))?;
        let mut at = 0usize;
        while at < records.len() {
            let record = symbols::decode_record_header(records, at)
                .map_err(|_| PdbError::BadRecord { what: "module symbol", at })?;
            at = record.next;
            if !matches!(record.kind, S_GPROC32 | S_LPROC32 | S_GPROC32_ID | S_LPROC32_ID) {
                continue;
            }
            let procedure = symbols::decode_procedure(record.data)
                .map_err(|_| PdbError::BadRecord { what: "procedure", at })?;
            let Some(rva) = addresses.rva(procedure.segment, procedure.offset) else {
                continue;
            };
            push(
                &mut self.procedures,
                Procedure { name: text(procedure.name), rva, len: u64::from(procedure.len) },
                "procedures",
            )?;
        }
        let c13_at = (module.symbol_bytes as usize).saturating_add(module.c11_bytes as usize);
        let Ok(c13) = stream.sub(c13_at, module.c13_bytes as usize) else {
            return Err(PdbError::BadRecord { what: "C13 line data", at: c13_at }.into());
        };
        // The file checksums first: line blocks name files through them.
        let mut checksums = Bytes::new(&[]);
        let mut at = 0usize;
        while at < c13.len() {
            let subsection = lines::decode_subsection(c13, at).map_err(|source| short("C13 subsection", source))?;
            at = subsection.next;
            if subsection.kind == DEBUG_S_FILECHKSMS {
                checksums = subsection.data;
            }
        }
        let mut at = 0usize;
        while at < c13.len() {
            let subsection = lines::decode_subsection(c13, at).map_err(|source| short("C13 subsection", source))?;
            at = subsection.next;
            if subsection.kind == DEBUG_S_LINES {
                self.read_lines(subsection.data, checksums, names, addresses, file_index)?;
            }
        }
        Ok(())
    }

    /// One lines subsection: each entry covers the code up to the next
    /// entry's offset, the last up to the subsection's end.
    fn read_lines(
        &mut self,
        data: Bytes<'_>,
        checksums: Bytes<'_>,
        names: Bytes<'_>,
        addresses: &Addresses,
        file_index: &mut BTreeMap<u32, usize>,
    ) -> Result<(), ParseError> {
        let head = lines::decode_lines_head(data).map_err(|source| short("lines header", source))?;
        let mut at = 12usize;
        while at < data.len() {
            let block = lines::decode_block(data, at).map_err(|source| short("line block", source))?;
            at = block.next;
            let name_offset = lines::checksum_name(checksums, block.file).map_err(|source| short("file checksum", source))?;
            let file = match file_index.get(&name_offset) {
                Some(index) => *index,
                None => {
                    let name = lines::names_string(names, name_offset).map(text).unwrap_or_default();
                    let index = self.files.len();
                    push(&mut self.files, name, "source files")?;
                    file_index.insert(name_offset, index);
                    index
                }
            };
            for index in 0..block.count {
                let entry = lines::decode_line(block.entries, index).map_err(|source| short("line entry", source))?;
                let end = if index.saturating_add(1) < block.count {
                    lines::decode_line(block.entries, index.saturating_add(1))
                        .map_err(|source| short("line entry", source))?
                        .offset
                } else {
                    head.size
                };
                let start = head.offset.saturating_add(entry.offset);
                let Some(rva) = addresses.rva(head.segment, start) else {
                    continue;
                };
                let len = u64::from(end.saturating_sub(entry.offset));
                push(&mut self.lines, LineRange { rva, len, file, line: entry.line }, "line ranges")?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn identity(&self) -> TablesIdentity {
        TablesIdentity::Pdb { guid: self.guid, age: self.age }
    }

    /// The procedure `rva` lies in, else the nearest public symbol at or
    /// below it, with the source line covering it when there is one.
    #[must_use]
    pub fn symbolize(&self, rva: Rva) -> Option<SymbolHit> {
        let at = rva.0;
        let line = self
            .lines
            .partition_point(|range| range.rva <= at)
            .checked_sub(1)
            .and_then(|index| self.lines.get(index))
            .filter(|range| at < range.rva.saturating_add(range.len.max(1)))
            .and_then(|range| {
                Some(SourceLine { file: self.files.get(range.file)?.clone(), line: range.line })
            });
        let procedure = self
            .procedures
            .partition_point(|procedure| procedure.rva <= at)
            .checked_sub(1)
            .and_then(|index| self.procedures.get(index))
            .filter(|procedure| at < procedure.rva.saturating_add(procedure.len));
        if let Some(procedure) = procedure {
            return Some(SymbolHit {
                name: procedure.name.clone(),
                offset: at.saturating_sub(procedure.rva),
                line,
            });
        }
        let public = self
            .publics
            .partition_point(|public| public.rva <= at)
            .checked_sub(1)
            .and_then(|index| self.publics.get(index))?;
        Some(SymbolHit {
            name: public.name.clone(),
            offset: at.saturating_sub(public.rva),
            line,
        })
    }

    /// A public symbol's RVA, else a procedure's.
    #[must_use]
    pub fn address_of(&self, name: &str) -> Option<Rva> {
        let first = self.by_name.partition_point(|named| named.name.as_str() < name);
        self.by_name
            .get(first)
            .filter(|named| named.name == name)
            .map(|named| Rva(named.rva))
    }

    /// The layout of the structure or union named `type_name`.
    pub fn layout(&self, type_name: &str) -> Result<Option<TypeLayout>, LayoutError> {
        match &self.types {
            Ok(types) => types.layout(type_name),
            Err(error) => Err(error.clone()),
        }
    }

    /// Where the member reached through `path` sits in `type_name`.
    pub fn member_location(&self, type_name: &str, path: &[&str]) -> Result<Option<MemberLocation>, LayoutError> {
        match &self.types {
            Ok(types) => types.member_location(type_name, path),
            Err(error) => Err(error.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, FixtureContainer, FixtureLine, FixtureOmap, FixtureOmapEntry, FixtureProcedure,
        FixturePublic, FixtureSection, PdbFixture, PublicKind, CODE_SECTION,
    };

    const GUID: [u8; 16] = [
        0xB9, 0xDB, 0x44, 0x38, 0x17, 0x20, 0x67, 0x49, 0xBE, 0x7A, 0xA4, 0xA2, 0xC2, 0x04, 0x30, 0xFA,
    ];

    /// Two procedures with lines, their publics, and a data public.
    fn named(container: FixtureContainer) -> PdbFixture {
        let mut fixture = PdbFixture::new(GUID, 7);
        fixture.container = container;
        fixture.publics = vec![
            FixturePublic { name: "KiTestProc".into(), section: 1, offset: 0x100, kind: PublicKind::Function },
            FixturePublic { name: "KiOtherProc".into(), section: 1, offset: 0x180, kind: PublicKind::Function },
            FixturePublic { name: "PsActiveProcessHead".into(), section: 2, offset: 0x10, kind: PublicKind::Data },
        ];
        fixture.procedures = vec![
            FixtureProcedure {
                name: "KiTestProc".into(),
                section: 1,
                offset: 0x100,
                len: 0x40,
                file: r"d:\src\ktest.c".into(),
                lines: vec![FixtureLine { offset: 0, line: 10 }, FixtureLine { offset: 0x10, line: 12 }],
            },
            FixtureProcedure {
                name: "KiOtherProc".into(),
                section: 1,
                offset: 0x180,
                len: 0x20,
                file: r"d:\src\other.c".into(),
                lines: vec![FixtureLine { offset: 0, line: 30 }],
            },
        ];
        fixture
    }

    fn line(file: &str, line: u32) -> Option<SourceLine> {
        Some(SourceLine { file: file.to_owned(), line })
    }

    #[test]
    fn a_pdb_names_addresses_in_either_container() {
        for container in [FixtureContainer::Msf, FixtureContainer::MsfzZstd, FixtureContainer::MsfzSmallChunks] {
            let tables = PdbTables::parse(&build_pdb(&named(container))).unwrap();
            assert_eq!(tables.identity(), TablesIdentity::Pdb { guid: Guid::from_le_bytes(GUID), age: 7 });
            assert_eq!(
                tables.symbolize(Rva(0x1114)),
                Some(SymbolHit { name: "KiTestProc".into(), offset: 0x14, line: line(r"d:\src\ktest.c", 12) }),
                "{container:?}"
            );
            assert_eq!(
                tables.symbolize(Rva(0x1180)),
                Some(SymbolHit { name: "KiOtherProc".into(), offset: 0, line: line(r"d:\src\other.c", 30) })
            );
            // Past every procedure, the nearest public names it, with no line.
            assert_eq!(
                tables.symbolize(Rva(0x3018)),
                Some(SymbolHit { name: "PsActiveProcessHead".into(), offset: 8, line: None })
            );
            // Below every symbol, nothing.
            assert_eq!(tables.symbolize(Rva(0x1000)), None);
            assert_eq!(tables.address_of("PsActiveProcessHead"), Some(Rva(0x3010)));
            assert_eq!(tables.address_of("KiOtherProc"), Some(Rva(0x1180)));
            assert_eq!(tables.address_of("Nope"), None);
        }
    }

    /// A PDB whose info-stream age is above its DBI age is the DBI age's
    /// build: dbghelp indexes it so, and its image's RSDS carries it.
    #[test]
    fn the_identity_age_is_the_dbi_age() {
        let mut fixture = named(FixtureContainer::Msf);
        fixture.age = 0xD;
        fixture.pdbi_age = 0xC;
        let bytes = build_pdb(&fixture);
        let expected = TablesIdentity::Pdb { guid: Guid::from_le_bytes(GUID), age: 0xD };
        assert_eq!(PdbTables::parse(&bytes).unwrap().identity(), expected);
        assert_eq!(pdb_identity(&bytes), Ok(expected));
    }

    /// OMAP: the linker put `.text` at 0x1000; a rewriting tool moved the
    /// range 0x1100.. to 0x5100 and dropped 0x1180..
    #[test]
    fn an_omap_pdb_names_the_rewritten_image() {
        let mut fixture = named(FixtureContainer::Msf);
        fixture.omap = Some(FixtureOmap {
            original_sections: vec![
                FixtureSection { name: ".text", rva: 0x1000, size: 0x2000, characteristics: CODE_SECTION },
                FixtureSection { name: ".data", rva: 0x3000, size: 0x2000, characteristics: CODE_SECTION },
            ],
            from_source: vec![
                FixtureOmapEntry { from: 0x1000, to: 0x5000 },
                FixtureOmapEntry { from: 0x1180, to: 0 },
                FixtureOmapEntry { from: 0x3000, to: 0x7000 },
            ],
        });
        let tables = PdbTables::parse(&build_pdb(&fixture)).unwrap();
        assert_eq!(tables.address_of("KiTestProc"), Some(Rva(0x5100)));
        assert_eq!(tables.address_of("PsActiveProcessHead"), Some(Rva(0x7010)));
        assert_eq!(tables.symbolize(Rva(0x5104)).map(|hit| hit.name), Some("KiTestProc".to_owned()));
        // The dropped procedure has no address in the rewritten image.
        assert_eq!(tables.address_of("KiOtherProc"), None);
    }

    #[test]
    fn bytes_that_are_not_a_pdb_are_refused() {
        assert!(PdbTables::parse(b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0garbage").is_err());
        assert_eq!(PdbTables::parse(&[]).map(|tables| tables.identity()), Err(ParseError::Pdb(PdbError::NotPdb)));
    }

    #[test]
    fn symbol_records_decode_one_at_a_time() {
        // S_PUB32: length 16 (kind, flags, offset, segment, "ab", NUL, one
        // pad byte), kind 0x110E, flags 2, offset 0x10, segment 1.
        let record = [16, 0, 0x0E, 0x11, 2, 0, 0, 0, 0x10, 0, 0, 0, 1, 0, b'a', b'b', 0, 0];
        let header = crate::pdb::symbols::decode_record_header(Bytes::new(&record), 0).unwrap();
        assert_eq!(header.kind, crate::pdb::symbols::S_PUB32);
        assert_eq!(header.next, 18);
        let public = crate::pdb::symbols::decode_named_address(header.data).unwrap();
        assert_eq!(public.offset, 0x10);
        assert_eq!(public.segment, 1);
        assert_eq!(public.name, b"ab");
    }
}
