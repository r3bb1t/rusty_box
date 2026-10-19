//! An in-memory [`FileStore`] for tests: a map of paths to bytes, with
//! modification times that move forward one second per write.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::store::{FileStore, ListedFile, Placed, Presence, StoreError};

struct StoredFile {
    bytes: Vec<u8>,
    modified: SystemTime,
}

#[derive(Default)]
pub struct MemoryStore {
    files: BTreeMap<PathBuf, StoredFile>,
    writes: u64,
    unreadable: BTreeSet<PathBuf>,
    /// How many recursive listings were asked for.
    listings: u64,
    /// How many whole files were read.
    whole_reads: u64,
}

impl MemoryStore {
    /// Puts `bytes` at `path`, replacing what was there.
    pub fn insert(&mut self, path: PathBuf, bytes: Vec<u8>) {
        self.writes = self.writes.saturating_add(1);
        let modified = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(self.writes))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        self.files.insert(path, StoredFile { bytes, modified });
    }

    #[must_use]
    pub fn bytes(&self, path: &Path) -> Option<&[u8]> {
        self.files.get(path).map(|file| file.bytes.as_slice())
    }

    #[must_use]
    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.keys().cloned().collect()
    }

    /// Makes reads of `path` fail as a disk error would.
    pub fn fail_reads_of(&mut self, path: PathBuf) {
        self.unreadable.insert(path);
    }

    /// How many recursive listings were asked for: a walk of a whole folder
    /// tree, which on a disk is the costly read.
    #[must_use]
    pub fn listings(&self) -> u64 {
        self.listings
    }

    /// How many whole files were read: the costly read a header-only one
    /// spares.
    #[must_use]
    pub fn whole_reads(&self) -> u64 {
        self.whole_reads
    }

    fn stored(&self, path: &Path) -> Result<&[u8], StoreError> {
        if self.unreadable.contains(path) {
            return Err(StoreError::Read {
                path: path.to_owned(),
                source: io::Error::other("an unreadable sector"),
            });
        }
        self.files
            .get(path)
            .map(|file| file.bytes.as_slice())
            .ok_or_else(|| StoreError::Read {
                path: path.to_owned(),
                source: io::Error::from(io::ErrorKind::NotFound),
            })
    }
}

impl FileStore for MemoryStore {
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, StoreError> {
        self.whole_reads = self.whole_reads.saturating_add(1);
        self.stored(path).map(<[u8]>::to_vec)
    }

    fn read_prefix(&mut self, path: &Path, len: usize) -> Result<Vec<u8>, StoreError> {
        let bytes = self.stored(path)?;
        Ok(bytes.get(..len).unwrap_or(bytes).to_vec())
    }

    fn presence(&mut self, path: &Path) -> Result<Presence, StoreError> {
        Ok(if self.files.contains_key(path) {
            Presence::Present
        } else {
            Presence::Absent
        })
    }

    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> Result<Placed, StoreError> {
        if self.files.contains_key(path) {
            return Ok(Placed::AlreadyPresent);
        }
        self.insert(path.to_owned(), bytes.to_vec());
        Ok(Placed::Written)
    }

    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), StoreError> {
        let file = self.files.remove(from).ok_or_else(|| StoreError::Rename {
            from: from.to_owned(),
            to: to.to_owned(),
            source: io::Error::from(io::ErrorKind::NotFound),
        })?;
        self.files.insert(to.to_owned(), file);
        Ok(())
    }

    fn list_recursive(&mut self, root: &Path) -> Result<Vec<ListedFile>, StoreError> {
        self.listings = self.listings.saturating_add(1);
        Ok(self
            .files
            .iter()
            .filter(|entry| entry.0.starts_with(root))
            .map(|entry| ListedFile {
                path: entry.0.clone(),
                bytes: u64::try_from(entry.1.bytes.len()).unwrap_or(u64::MAX),
                modified: entry.1.modified,
            })
            .collect())
    }
}
