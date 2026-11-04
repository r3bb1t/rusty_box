//! A project's source folders, walked once and kept by file name, so a
//! file the build recorded on another machine is found by its name and the
//! longest run of trailing folders it shares.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rusty_box_symbols::store::{FileStore, StoreError};

/// Every file under a project's source folders, by lower-cased file name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceFolders {
    by_name: BTreeMap<String, Vec<PathBuf>>,
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

impl SourceFolders {
    /// Walks each of `folders` once.
    pub(crate) fn index<F: FileStore>(files: &mut F, folders: &[PathBuf]) -> Result<SourceFolders, StoreError> {
        let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for folder in folders {
            for listed in files.list_recursive(folder)? {
                let Some(name) = listed.path.file_name().map(|name| key(&name.to_string_lossy())) else {
                    continue;
                };
                by_name.entry(name).or_default().push(listed.path);
            }
        }
        for paths in by_name.values_mut() {
            paths.sort();
        }
        Ok(SourceFolders { by_name })
    }

    /// The files named `name`, whatever its case, by path.
    pub(crate) fn named(&self, name: &str) -> &[PathBuf] {
        self.by_name.get(&key(name)).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols::memory_store::MemoryStore;
    use std::path::PathBuf;

    #[test]
    fn files_are_found_by_name_without_case() {
        let mut files = MemoryStore::default();
        let src = PathBuf::from("work").join("src");
        files.insert(src.join("io").join("read.c"), b"a".to_vec());
        files.insert(src.join("net").join("Read.c"), b"b".to_vec());
        files.insert(src.join("io").join("write.c"), b"c".to_vec());
        files.insert(PathBuf::from("elsewhere").join("read.c"), b"d".to_vec());
        let index = SourceFolders::index(&mut files, &[src.clone()]).unwrap();
        assert_eq!(index.named("READ.C"), [src.join("io").join("read.c"), src.join("net").join("Read.c")]);
        assert_eq!(index.named("write.c"), [src.join("io").join("write.c")]);
        assert!(index.named("missing.c").is_empty());
    }

    #[test]
    fn each_folder_is_walked_once() {
        let mut files = MemoryStore::default();
        let folders = [PathBuf::from("a"), PathBuf::from("b")];
        SourceFolders::index(&mut files, &folders).unwrap();
        assert_eq!(files.listings(), 2);
    }
}
