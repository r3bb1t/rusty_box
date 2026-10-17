//! Program databases. A PDB is a set of numbered streams in one of two
//! containers: MSF 7.00 ([`msf`]) or Microsoft's compressed MSFZ ([`msfz`]).
//! [`PdbFile`] opens either and hands out streams as owned bytes; the
//! streams' formats are read by the modules of Tasks 10 and 11.

use crate::decompress::DecompressError;
use crate::read::ReadError;

/// MSF 7.00's superblock magic.
pub const MSF_MAGIC: &[u8; 32] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";
/// MSFZ's file signature (`ms-pdb-msfz` `MSFZ_FILE_SIGNATURE`).
pub const MSFZ_MAGIC: &[u8; 32] = b"Microsoft MSFZ Container\r\n\x1aALD\0\0";
/// PDB 2.0's: no GUID, so no symbol-store key; refused by name.
pub const PDB2_MAGIC: &[u8] = b"Microsoft C/C++ program database 2.00\r\n";

/// Which container a PDB uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    Msf,
    Msfz,
}

/// Why a PDB, or one of its streams, did not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PdbError {
    #[error("the bytes are not a PDB")]
    NotPdb,
    #[error("a PDB 2.0 file has no GUID, so it cannot be matched to an image")]
    Pdb2,
    #[error("the {what} is cut short: {source}")]
    Short {
        what: &'static str,
        #[source]
        source: ReadError,
    },
    #[error("an MSF block size of {size} is not 512, 1024, 2048 or 4096")]
    BadBlockSize { size: u32 },
    #[error("block {block} lies outside the file's {count} blocks")]
    BadBlock { block: u32, count: u32 },
    #[error("block {block} belongs to two owners; an MSF block holds one stream's bytes")]
    SharedBlock { block: u32 },
    #[error("{count} {what} is more than a real PDB has")]
    TooMany { what: &'static str, count: u64 },
    #[error("MSFZ version {version} is not the one this reader knows (0)")]
    BadVersion { version: u64 },
    #[error("MSFZ compression {code} is not none, zstd or deflate")]
    UnknownCompression { code: u32 },
    #[error("an MSFZ chunk does not decompress: {0}")]
    Decompress(#[from] DecompressError),
    #[error("stream {stream}'s fragment {fragment} lies outside its chunk or the file")]
    BadFragment { stream: u32, fragment: u32 },
    #[error("stream {stream}'s fragment {fragment} reuses bytes another fragment holds")]
    OverlappingFragments { stream: u32, fragment: u32 },
    #[error("an uncompressed MSFZ chunk stores {stored} bytes and declares {declared}")]
    StoredChunkSize { stored: u32, declared: u32 },
    #[error("stream {index} is not in the directory")]
    NoStream { index: u32 },
    #[error("no memory for {bytes} bytes of {what}")]
    Allocation { what: &'static str, bytes: u64 },
    #[error("the PDB has no {what} stream")]
    MissingStream { what: &'static str },
    #[error("a {what} record at offset {at:#x} is malformed")]
    BadRecord { what: &'static str, at: usize },
    #[error("the OMAP stream is {bytes} bytes, not a whole number of entries")]
    Omap { bytes: usize },
}

/// `PdbError::Short` for the named part.
pub(crate) fn short(what: &'static str, source: ReadError) -> PdbError {
    PdbError::Short { what, source }
}

/// A `Vec` with room for exactly `len` bytes, or the error that says why
/// there is none.
pub(crate) fn reserved(len: u64, what: &'static str) -> Result<Vec<u8>, PdbError> {
    let count = usize::try_from(len).map_err(|_| PdbError::Allocation { what, bytes: len })?;
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| PdbError::Allocation { what, bytes: len })?;
    Ok(out)
}

/// An open PDB in either container.
pub struct PdbFile<'a> {
    container: Container<'a>,
}

enum Container<'a> {
    Msf(msf::MsfStreams<'a>),
    Msfz(msfz::MsfzStreams<'a>),
}

impl<'a> PdbFile<'a> {
    /// Opens `bytes`, reading the container's directory.
    pub fn open(bytes: &'a [u8]) -> Result<PdbFile<'a>, PdbError> {
        let container = if bytes.starts_with(MSF_MAGIC) {
            Container::Msf(msf::MsfStreams::open(bytes)?)
        } else if bytes.starts_with(MSFZ_MAGIC) {
            Container::Msfz(msfz::MsfzStreams::open(bytes)?)
        } else if bytes.starts_with(PDB2_MAGIC) {
            return Err(PdbError::Pdb2);
        } else {
            return Err(PdbError::NotPdb);
        };
        Ok(PdbFile { container })
    }

    #[must_use]
    pub fn kind(&self) -> ContainerKind {
        match self.container {
            Container::Msf(_) => ContainerKind::Msf,
            Container::Msfz(_) => ContainerKind::Msfz,
        }
    }

    #[must_use]
    pub fn stream_count(&self) -> u32 {
        match &self.container {
            Container::Msf(streams) => streams.count(),
            Container::Msfz(streams) => streams.count(),
        }
    }

    /// Stream `index`'s bytes; `None` for a nil stream.
    pub fn stream(&mut self, index: u32) -> Result<Option<Vec<u8>>, PdbError> {
        match &mut self.container {
            Container::Msf(streams) => streams.read(index),
            Container::Msfz(streams) => streams.read(index),
        }
    }
}

pub mod dbi;
pub mod hash_table;
pub mod info;
pub mod lines;
pub mod msf;
pub mod msfz;
pub mod source_block;
pub mod symbols;
pub mod tables;
pub mod tpi;

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols_fixtures::compress::sample_text;
    use rusty_box_symbols_fixtures::pdb::{
        build_pdb, fixture_streams, FixtureContainer, FixturePublic, PdbFixture, PublicKind,
    };

    /// A fixture whose streams are not all tiny: 400 publics make the
    /// symbol-record stream span many MSF blocks and MSFZ chunks.
    fn fixture(container: FixtureContainer) -> PdbFixture {
        let mut fixture = PdbFixture::new([7; 16], 3);
        fixture.container = container;
        let words = sample_text(8000);
        for index in 0..400u32 {
            let start = (index as usize * 17) % 7000;
            let stem = String::from_utf8_lossy(&words[start..start + 12]).replace(['\n', ' '], "_");
            fixture.publics.push(FixturePublic {
                name: format!("{stem}{index}"),
                section: 1,
                offset: index * 4,
                kind: PublicKind::Function,
            });
        }
        fixture
    }

    #[test]
    fn every_stream_reads_back_in_every_container() {
        let containers = [
            FixtureContainer::Msf,
            FixtureContainer::MsfzZstd,
            FixtureContainer::MsfzDeflate,
            FixtureContainer::MsfzSmallChunks,
            FixtureContainer::MsfzStored,
        ];
        for container in containers {
            let fixture = fixture(container);
            let bytes = build_pdb(&fixture);
            let mut file = PdbFile::open(&bytes).unwrap();
            let expected_kind = if container == FixtureContainer::Msf { ContainerKind::Msf } else { ContainerKind::Msfz };
            assert_eq!(file.kind(), expected_kind);
            for stream in fixture_streams(&fixture) {
                assert_eq!(
                    file.stream(stream.index).unwrap().as_deref(),
                    Some(stream.bytes.as_slice()),
                    "{container:?} stream {}",
                    stream.index
                );
            }
        }
    }

    #[test]
    fn a_stream_past_the_directory_is_refused() {
        let bytes = build_pdb(&fixture(FixtureContainer::Msf));
        let mut file = PdbFile::open(&bytes).unwrap();
        let count = file.stream_count();
        assert_eq!(file.stream(count), Err(PdbError::NoStream { index: count }));
    }

    #[test]
    fn a_pdb_2_file_is_named_not_parsed() {
        let mut bytes = PDB2_MAGIC.to_vec();
        bytes.extend_from_slice(&[0; 64]);
        assert_eq!(PdbFile::open(&bytes).map(|file| file.kind()), Err(PdbError::Pdb2));
        assert_eq!(PdbFile::open(b"not a pdb").map(|file| file.kind()), Err(PdbError::NotPdb));
    }

    #[test]
    fn a_damaged_msf_directory_is_refused() {
        let mut bytes = build_pdb(&fixture(FixtureContainer::Msf));
        // The block map's address (superblock +52) past the file.
        bytes[52..56].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        assert!(PdbFile::open(&bytes).is_err());
        let mut bytes = build_pdb(&fixture(FixtureContainer::Msf));
        // A block size MSF 7.00 does not use.
        bytes[32..36].copy_from_slice(&3000u32.to_le_bytes());
        assert_eq!(PdbFile::open(&bytes).map(|file| file.kind()), Err(PdbError::BadBlockSize { size: 3000 }));
    }

    #[test]
    fn a_damaged_msfz_header_is_refused() {
        let mut bytes = build_pdb(&fixture(FixtureContainer::MsfzZstd));
        // The version (header +32) must be 0.
        bytes[32] = 1;
        assert_eq!(PdbFile::open(&bytes).map(|file| file.kind()), Err(PdbError::BadVersion { version: 1 }));
        let mut bytes = build_pdb(&fixture(FixtureContainer::MsfzZstd));
        // The chunk table's offset (header +48) past the file.
        bytes[48..56].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(PdbFile::open(&bytes).is_err());
    }
}
