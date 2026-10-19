//! The symbol store: a folder of debug-information files, each kept under
//! the identity of the build it describes, laid out as WinDbg's symsrv lays
//! a downstream store out (Microsoft Learn, "Symbol Store Folder Tree"), so
//! one folder can be shared with WinDbg. Every file operation goes through
//! the [`FileStore`] port.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::identity::{BuildId, PdbId, PeImageId};

/// The file whose presence in a store's root makes it two-tier: files sit
/// under a folder named by the first two characters of their name.
pub const TWO_TIER_MARKER: &str = "index2.txt";

/// The largest file any [`FileStore`] reads, checked before a byte is read:
/// the largest PDBs Microsoft ships (browsers, Office) are around 2 GiB.
pub const MAX_FILE_BYTES: u64 = 1 << 32;

/// Whether a path names a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Presence {
    Present,
    Absent,
}

/// What a write of a new file came to. Files are keyed by identity, so a
/// file another writer placed first has the same content and is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Placed {
    Written,
    AlreadyPresent,
}

/// One file under a listed folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is {bytes} bytes, over the {cap} a symbol file may be", path.display())]
    TooLarge { path: PathBuf, bytes: u64, cap: u64 },
    #[error("failed to write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to rename {} to {}: {source}", from.display(), to.display())]
    Rename {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to look at {}: {source}", path.display())]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to list {}: {source}", path.display())]
    List {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A write that failed left its partial file behind, because removing
    /// it failed too.
    #[error("{cause}; the partial file {} could not be removed: {cleanup}", partial.display())]
    PartialLeft {
        partial: PathBuf,
        cause: Box<StoreError>,
        #[source]
        cleanup: io::Error,
    },
    /// A file name a guest supplied that would not name one file inside the
    /// store (`..`, a separator, a drive, a character no file system takes).
    #[error("{name:?} is not a file name a symbol store can hold")]
    UnsafeName { name: String },
}

/// Every side effect the engine has on files.
pub trait FileStore {
    /// The whole file; one over [`MAX_FILE_BYTES`] (or the adapter's own,
    /// lower cap) is [`StoreError::TooLarge`] before it is read.
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, StoreError>;
    /// The file's first `len` bytes, or all of it when it is shorter: what a
    /// file's headers are read from without reading the rest.
    fn read_prefix(&mut self, path: &Path, len: usize) -> Result<Vec<u8>, StoreError>;
    /// An I/O error is not "absent", so this is not a `bool`.
    fn presence(&mut self, path: &Path) -> Result<Presence, StoreError>;
    /// Writes `bytes` to a uniquely named file beside `path`, then renames it
    /// into place, unless a file is found at `path` first.
    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> Result<Placed, StoreError>;
    /// Renames `from` to `to`, replacing a file at `to`.
    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), StoreError>;
    /// Every file under `root`; none when `root` does not exist.
    fn list_recursive(&mut self, root: &Path) -> Result<Vec<ListedFile>, StoreError>;
}

/// Whether a store keeps files directly under its root or under a folder of
/// their name's first two characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiers {
    One,
    Two,
}

/// How much a store holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreUsage {
    pub files: u64,
    pub bytes: u64,
}

/// `name` when it names one file inside a folder and one path segment of a
/// server URL: not empty, not `.` or `..`, no separator, drive, wildcard,
/// URL delimiter or control character, and no trailing dot or space (which
/// Windows strips). Names come from the guest (an RSDS path, a loader
/// entry), which is not trusted.
pub fn checked_file_name(name: &str) -> Result<&str, StoreError> {
    let refused = name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| {
            c.is_control()
                || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '%')
        });
    if refused {
        Err(StoreError::UnsafeName {
            name: name.to_owned(),
        })
    } else {
        Ok(name)
    }
}

/// A build-id's hex split as `.build-id` folders split it: the first byte,
/// then the rest.
struct BuildIdPath {
    head: String,
    tail: String,
}

fn build_id_path(build_id: &BuildId) -> BuildIdPath {
    let hex = build_id.to_hex();
    BuildIdPath {
        head: hex.get(..2).unwrap_or(&hex).to_owned(),
        tail: hex.get(2..).unwrap_or("").to_owned(),
    }
}

/// A symbol store over the folder `root`.
pub struct SymbolStore<F: FileStore> {
    files: F,
    root: PathBuf,
    tiers: Tiers,
}

impl<F: FileStore> SymbolStore<F> {
    /// The store at `root`, two-tier when `index2.txt` is in it.
    pub fn open(mut files: F, root: PathBuf) -> Result<Self, StoreError> {
        let tiers = match files.presence(&root.join(TWO_TIER_MARKER))? {
            Presence::Present => Tiers::Two,
            Presence::Absent => Tiers::One,
        };
        Ok(SymbolStore { files, root, tiers })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn tiers(&self) -> Tiers {
        self.tiers
    }

    pub fn files(&mut self) -> &mut F {
        &mut self.files
    }

    /// `<root>[/<first two>]/<file>/<key>/<file>`.
    fn keyed(&self, file: &str, key: &str) -> Result<PathBuf, StoreError> {
        let file = checked_file_name(file)?;
        let base = match self.tiers {
            Tiers::One => self.root.clone(),
            Tiers::Two => self.root.join(file.chars().take(2).collect::<String>()),
        };
        Ok(base.join(file).join(key).join(file))
    }

    pub fn pdb_path(&self, pdb: &PdbId) -> Result<PathBuf, StoreError> {
        self.keyed(&pdb.name, &pdb.store_key())
    }

    pub fn binary_path(&self, file_name: &str, image: PeImageId) -> Result<PathBuf, StoreError> {
        self.keyed(file_name, &image.store_key())
    }

    /// `<root>/.build-id/<first byte>/<rest>.debug`, as debuginfod clients
    /// and distributions lay debug files out.
    #[must_use]
    pub fn debug_file_path(&self, build_id: &BuildId) -> PathBuf {
        let split = build_id_path(build_id);
        self.root.join(".build-id").join(split.head).join(format!("{}.debug", split.tail))
    }

    /// `<root>/.build-id/<first byte>/<rest>`: the executable itself.
    #[must_use]
    pub fn executable_path(&self, build_id: &BuildId) -> PathBuf {
        let split = build_id_path(build_id);
        self.root.join(".build-id").join(split.head).join(split.tail)
    }

    /// The file at `path`, when the store holds it.
    pub fn lookup(&mut self, path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
        match self.files.presence(path)? {
            Presence::Present => self.files.read(path).map(Some),
            Presence::Absent => Ok(None),
        }
    }

    pub fn place(&mut self, path: &Path, bytes: &[u8]) -> Result<Placed, StoreError> {
        self.files.write_new(path, bytes)
    }

    /// Moves a file that does not parse aside as `<name>.bad`, so it is not
    /// loaded again and the next lookup fetches it afresh; returns the path
    /// it moved to.
    pub fn quarantine(&mut self, path: &Path) -> Result<PathBuf, StoreError> {
        let mut bad = path.as_os_str().to_owned();
        bad.push(".bad");
        let bad = PathBuf::from(bad);
        self.files.rename(path, &bad)?;
        Ok(bad)
    }

    pub fn usage(&mut self) -> Result<StoreUsage, StoreError> {
        let listed = self.files.list_recursive(&self.root)?;
        Ok(StoreUsage {
            files: u64::try_from(listed.len()).unwrap_or(u64::MAX),
            bytes: listed
                .iter()
                .fold(0u64, |total, file| total.saturating_add(file.bytes)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Guid;
    use crate::memory_store::MemoryStore;

    fn kernel_pdb() -> PdbId {
        PdbId {
            name: "ntkrnlmp.pdb".to_owned(),
            guid: Guid::from_le_bytes([
                0xFE, 0xC6, 0x61, 0xB6, 0x99, 0x72, 0x16, 0x1D, 0x0F, 0xE3, 0x7C, 0x48, 0xB8, 0xBC,
                0x54, 0x70,
            ]),
            age: 1,
        }
    }

    fn root() -> PathBuf {
        PathBuf::from("store")
    }

    #[test]
    fn files_sit_where_windbg_puts_them() {
        let store = SymbolStore::open(MemoryStore::default(), root()).unwrap();
        assert_eq!(store.tiers(), Tiers::One);
        assert_eq!(
            store.pdb_path(&kernel_pdb()).unwrap(),
            root().join("ntkrnlmp.pdb").join("B661C6FE72991D160FE37C48B8BC54701").join("ntkrnlmp.pdb")
        );
        let image = PeImageId { time_date_stamp: 0xAFF6_531F, size_of_image: 0x145_0000 };
        assert_eq!(
            store.binary_path("ntoskrnl.exe", image).unwrap(),
            root().join("ntoskrnl.exe").join("AFF6531F1450000").join("ntoskrnl.exe")
        );
        let build_id = BuildId(vec![0xAB, 0xCD, 0xEF]);
        assert_eq!(store.debug_file_path(&build_id), root().join(".build-id").join("ab").join("cdef.debug"));
        assert_eq!(store.executable_path(&build_id), root().join(".build-id").join("ab").join("cdef"));
        let short = BuildId(vec![0x01]);
        assert_eq!(store.debug_file_path(&short), root().join(".build-id").join("01").join(".debug"));
    }

    #[test]
    fn an_index2_marker_makes_a_two_tier_store() {
        let mut files = MemoryStore::default();
        files.insert(root().join(TWO_TIER_MARKER), Vec::new());
        let store = SymbolStore::open(files, root()).unwrap();
        assert_eq!(store.tiers(), Tiers::Two);
        assert_eq!(
            store.pdb_path(&kernel_pdb()).unwrap(),
            root()
                .join("nt")
                .join("ntkrnlmp.pdb")
                .join("B661C6FE72991D160FE37C48B8BC54701")
                .join("ntkrnlmp.pdb")
        );
    }

    #[test]
    fn a_name_from_the_guest_cannot_leave_the_store() {
        let store = SymbolStore::open(MemoryStore::default(), root()).unwrap();
        for name in ["..", ".", "", "a/b.pdb", r"..\x.pdb", "c:x.pdb", "x.pdb.", "nul\0.pdb", "x#y.pdb", "x%2F.pdb"] {
            let pdb = PdbId { name: name.to_owned(), ..kernel_pdb() };
            assert!(matches!(store.pdb_path(&pdb), Err(StoreError::UnsafeName { .. })), "{name:?} was accepted");
        }
    }

    #[test]
    fn a_file_placed_twice_keeps_the_first() {
        let mut store = SymbolStore::open(MemoryStore::default(), root()).unwrap();
        let path = store.pdb_path(&kernel_pdb()).unwrap();
        assert_eq!(store.place(&path, b"first").unwrap(), Placed::Written);
        assert_eq!(store.place(&path, b"second").unwrap(), Placed::AlreadyPresent);
        assert_eq!(store.lookup(&path).unwrap(), Some(b"first".to_vec()));
    }

    #[test]
    fn a_quarantined_file_is_moved_aside_as_bad() {
        let mut store = SymbolStore::open(MemoryStore::default(), root()).unwrap();
        let path = store.pdb_path(&kernel_pdb()).unwrap();
        assert_eq!(store.place(&path, b"garbage").unwrap(), Placed::Written);
        let bad = store.quarantine(&path).unwrap();
        assert_eq!(bad, PathBuf::from(format!("{}.bad", path.display())));
        assert_eq!(store.lookup(&path).unwrap(), None);
        assert_eq!(store.lookup(&bad).unwrap(), Some(b"garbage".to_vec()));
    }

    #[test]
    fn the_usage_counts_every_file_and_byte() {
        let mut store = SymbolStore::open(MemoryStore::default(), root()).unwrap();
        assert_eq!(store.usage().unwrap(), StoreUsage::default());
        let path = store.pdb_path(&kernel_pdb()).unwrap();
        assert_eq!(store.place(&path, &[0; 100]).unwrap(), Placed::Written);
        let other = store.debug_file_path(&BuildId(vec![1, 2, 3]));
        assert_eq!(store.place(&other, &[0; 20]).unwrap(), Placed::Written);
        assert_eq!(store.usage().unwrap(), StoreUsage { files: 2, bytes: 120 });
    }
}
