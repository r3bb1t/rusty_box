//! [`FileStore`] over `std::fs`. A new file is written under a name no other
//! writer uses, synced, and renamed into place only when nothing is there
//! yet: `std::fs::rename` replaces its target on every platform, so the check
//! comes first, and a writer that loses the race finds the same content
//! (files are keyed by identity). A read checks the file's size against the
//! cap before reading it, and reads no more than the cap even if the file
//! grows meanwhile.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::store::{FileStore, ListedFile, Placed, Presence, StoreError, MAX_FILE_BYTES};

pub struct FsStore {
    /// Distinguishes this writer's partial files from other workers' in the
    /// same process.
    writer: u32,
    written: u64,
    read_cap: u64,
}

impl FsStore {
    /// A store used by worker number `writer`.
    #[must_use]
    pub fn new(writer: u32) -> Self {
        FsStore {
            writer,
            written: 0,
            read_cap: MAX_FILE_BYTES,
        }
    }

    /// This store reading no file over `cap` bytes (never more than
    /// [`MAX_FILE_BYTES`]).
    #[must_use]
    pub fn with_read_cap(self, cap: u64) -> Self {
        FsStore {
            read_cap: cap.min(MAX_FILE_BYTES),
            ..self
        }
    }

    fn partial_path(&mut self, path: &Path) -> PathBuf {
        self.written = self.written.wrapping_add(1);
        let mut name = path.file_name().map(|name| name.to_owned()).unwrap_or_default();
        name.push(format!(
            ".{}.{}.{}.partial",
            std::process::id(),
            self.writer,
            self.written
        ));
        path.with_file_name(name)
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Removes a partial file; one that was never created is no failure.
fn remove_partial(partial: &Path) -> io::Result<()> {
    match fs::remove_file(partial) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

impl FileStore for FsStore {
    fn read(&mut self, path: &Path) -> Result<Vec<u8>, StoreError> {
        let read_error = |source| StoreError::Read {
            path: path.to_owned(),
            source,
        };
        let file = fs::File::open(path).map_err(read_error)?;
        let bytes = file.metadata().map_err(read_error)?.len();
        if bytes > self.read_cap {
            return Err(StoreError::TooLarge {
                path: path.to_owned(),
                bytes,
                cap: self.read_cap,
            });
        }
        let len = usize::try_from(bytes).map_err(|_| StoreError::TooLarge {
            path: path.to_owned(),
            bytes,
            cap: self.read_cap,
        })?;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|error| read_error(io::Error::new(io::ErrorKind::OutOfMemory, error)))?;
        file.take(self.read_cap.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(read_error)?;
        let read = u64::try_from(out.len()).unwrap_or(u64::MAX);
        if read > self.read_cap {
            return Err(StoreError::TooLarge {
                path: path.to_owned(),
                bytes: read,
                cap: self.read_cap,
            });
        }
        Ok(out)
    }

    fn presence(&mut self, path: &Path) -> Result<Presence, StoreError> {
        match fs::metadata(path) {
            Ok(_) => Ok(Presence::Present),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Presence::Absent),
            Err(source) => Err(StoreError::Inspect {
                path: path.to_owned(),
                source,
            }),
        }
    }

    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> Result<Placed, StoreError> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(|source| StoreError::Write {
                path: folder.to_owned(),
                source,
            })?;
        }
        let partial = self.partial_path(path);
        let placed = write_synced(&partial, bytes)
            .map_err(|source| StoreError::Write {
                path: partial.clone(),
                source,
            })
            .and_then(|()| self.presence(path))
            .and_then(|presence| match presence {
                Presence::Absent => fs::rename(&partial, path)
                    .map(|()| Placed::Written)
                    .map_err(|source| StoreError::Rename {
                        from: partial.clone(),
                        to: path.to_owned(),
                        source,
                    }),
                Presence::Present => Ok(Placed::AlreadyPresent),
            });
        match placed {
            // The partial file became the store's file.
            Ok(Placed::Written) => Ok(Placed::Written),
            Ok(Placed::AlreadyPresent) => match remove_partial(&partial) {
                Ok(()) => Ok(Placed::AlreadyPresent),
                Err(source) => Err(StoreError::Write {
                    path: partial,
                    source,
                }),
            },
            Err(cause) => Err(match remove_partial(&partial) {
                Ok(()) => cause,
                Err(cleanup) => StoreError::PartialLeft {
                    partial,
                    cause: Box::new(cause),
                    cleanup,
                },
            }),
        }
    }

    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), StoreError> {
        fs::rename(from, to).map_err(|source| StoreError::Rename {
            from: from.to_owned(),
            to: to.to_owned(),
            source,
        })
    }

    fn list_recursive(&mut self, root: &Path) -> Result<Vec<ListedFile>, StoreError> {
        let mut listed = Vec::new();
        let mut folders = vec![root.to_owned()];
        while let Some(folder) = folders.pop() {
            let entries = match fs::read_dir(&folder) {
                Ok(entries) => entries,
                Err(error) if error.kind() == io::ErrorKind::NotFound && folder == root => {
                    return Ok(listed);
                }
                Err(source) => return Err(StoreError::List { path: folder, source }),
            };
            for entry in entries {
                let entry = entry.map_err(|source| StoreError::List {
                    path: folder.clone(),
                    source,
                })?;
                let path = entry.path();
                let metadata = entry.metadata().map_err(|source| StoreError::Inspect {
                    path: path.clone(),
                    source,
                })?;
                if metadata.is_dir() {
                    folders.push(path);
                } else {
                    let modified = metadata.modified().map_err(|source| StoreError::Inspect {
                        path: path.clone(),
                        source,
                    })?;
                    listed.push(ListedFile {
                        path,
                        bytes: metadata.len(),
                        modified,
                    });
                }
            }
        }
        Ok(listed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        std::env::temp_dir().join(format!("rusty_box_symbols-{tag}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn a_new_file_lands_whole_and_a_second_writer_keeps_it() {
        let root = scratch("write");
        let path = root.join("a.pdb").join("KEY1").join("a.pdb");
        let mut first = FsStore::new(0);
        let mut second = FsStore::new(1);
        assert_eq!(first.write_new(&path, b"one").unwrap(), Placed::Written);
        assert_eq!(second.write_new(&path, b"two").unwrap(), Placed::AlreadyPresent);
        assert_eq!(first.read(&path).unwrap(), b"one");
        let listed = first.list_recursive(&root).unwrap();
        assert_eq!(listed.len(), 1, "no partial file is left behind: {listed:?}");
        assert_eq!(listed[0].bytes, 3);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_that_does_not_exist_is_empty_and_absent() {
        let root = scratch("missing");
        let mut files = FsStore::new(0);
        assert_eq!(files.list_recursive(&root).unwrap(), Vec::new());
        assert_eq!(files.presence(&root.join("x")).unwrap(), Presence::Absent);
    }

    #[test]
    fn a_rename_replaces_its_target() {
        let root = scratch("rename");
        let mut files = FsStore::new(0);
        let from = root.join("x.pdb");
        let to = root.join("x.pdb.bad");
        assert_eq!(files.write_new(&to, b"old").unwrap(), Placed::Written);
        assert_eq!(files.write_new(&from, b"new").unwrap(), Placed::Written);
        files.rename(&from, &to).unwrap();
        assert_eq!(files.read(&to).unwrap(), b"new");
        assert_eq!(files.presence(&from).unwrap(), Presence::Absent);
        fs::remove_dir_all(&root).unwrap();
    }

    /// A file larger than the cap is refused before its bytes are read, so a
    /// hostile file in a shared folder cannot exhaust memory.
    #[test]
    fn a_file_over_the_cap_is_refused_unread() {
        let root = scratch("cap");
        let path = root.join("big.pdb");
        let mut files = FsStore::new(0).with_read_cap(10);
        assert_eq!(files.write_new(&path, &[0; 100]).unwrap(), Placed::Written);
        assert!(matches!(
            files.read(&path),
            Err(StoreError::TooLarge { bytes: 100, cap: 10, .. })
        ));
        fs::remove_dir_all(&root).unwrap();
    }
}
