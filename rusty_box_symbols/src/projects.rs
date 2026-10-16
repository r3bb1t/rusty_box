//! The owner's build folders, indexed by the identity each debug file
//! declares, so a module is matched to its own build's PDB or ELF debug
//! file and never to a namesake from another build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::identity::{BuildId, Guid, ModuleIdentity, TablesIdentity};
use crate::store::{FileStore, ListedFile, StoreError};
use crate::tables::{declared_identity, ParseError};

/// One project's folders of debug files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFolders {
    pub name: String,
    pub symbols: Vec<PathBuf>,
}

/// What a debug file is indexed under.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IndexKey {
    Pdb { guid: Guid, age: u32 },
    Build(BuildId),
}

/// A debug file found in a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedFile {
    pub project: String,
    pub path: PathBuf,
}

/// A candidate file that did not give an identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexProblem {
    pub path: PathBuf,
    pub reason: String,
}

/// Whether the folders changed since an index was built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Current,
    Stale,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectIndex {
    files: BTreeMap<IndexKey, IndexedFile>,
    /// The newest modification time under every folder when indexed.
    newest: Option<SystemTime>,
    pub problems: Vec<IndexProblem>,
}

/// Whether a file may be a debug file: a PDB by its extension, an ELF
/// executable, library, kernel module or separate debug file by its name.
fn is_candidate(path: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => {
            let extension = extension.to_ascii_lowercase();
            extension == "pdb"
                || extension == "debug"
                || extension == "ko"
                || extension == "so"
                || path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.contains(".so."))
        }
        None => true,
    }
}

/// A listed file and the project whose folder holds it.
struct ProjectFile {
    project: String,
    file: ListedFile,
}

fn listed<F: FileStore>(files: &mut F, projects: &[ProjectFolders]) -> Result<Vec<ProjectFile>, StoreError> {
    let mut out = Vec::new();
    for project in projects {
        for folder in &project.symbols {
            for file in files.list_recursive(folder)? {
                out.push(ProjectFile { project: project.name.clone(), file });
            }
        }
    }
    Ok(out)
}

impl ProjectIndex {
    /// Indexes every candidate file under every project's folders. A file
    /// that does not read or parse is listed in `problems`, not indexed;
    /// the first file of an identity wins.
    pub fn build<F: FileStore>(files: &mut F, projects: &[ProjectFolders]) -> Result<ProjectIndex, StoreError> {
        let mut index = ProjectIndex::default();
        for listed in listed(files, projects)? {
            index.newest = index.newest.max(Some(listed.file.modified));
            if !is_candidate(&listed.file.path) {
                continue;
            }
            let bytes = match files.read(&listed.file.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    index.problems.push(IndexProblem { path: listed.file.path, reason: error.to_string() });
                    continue;
                }
            };
            let key = match declared_identity(&bytes) {
                Ok(TablesIdentity::Pdb { guid, age }) => IndexKey::Pdb { guid, age },
                Ok(TablesIdentity::Elf { build_id: Some(build_id) }) => IndexKey::Build(build_id),
                // An ELF file without a build-id cannot be matched.
                Ok(TablesIdentity::Elf { build_id: None }) => continue,
                // A file of another kind with a candidate's name is no
                // problem; one that claims a format and breaks is.
                Err(ParseError::NotSymbols) => continue,
                Err(error) => {
                    index.problems.push(IndexProblem { path: listed.file.path, reason: error.to_string() });
                    continue;
                }
            };
            index.files.entry(key).or_insert(IndexedFile { project: listed.project, path: listed.file.path });
        }
        Ok(index)
    }

    /// The project file of `wanted`'s own build.
    #[must_use]
    pub fn find(&self, wanted: &ModuleIdentity) -> Option<&IndexedFile> {
        let key = match wanted {
            ModuleIdentity::Pe { pdb: Some(pdb), .. } => IndexKey::Pdb { guid: pdb.guid, age: pdb.age },
            ModuleIdentity::Pe { pdb: None, .. } => return None,
            ModuleIdentity::Elf { build_id, .. } => IndexKey::Build(build_id.clone()),
        };
        self.files.get(&key)
    }

    /// The project file of an ELF build-id: a dwz supplement is named so.
    #[must_use]
    pub fn find_build_id(&self, build_id: &BuildId) -> Option<&IndexedFile> {
        self.files.get(&IndexKey::Build(build_id.clone()))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Whether any file under the folders is newer than the index, or one
    /// was removed or added (the newest time moved either way).
    pub fn freshness<F: FileStore>(&self, files: &mut F, projects: &[ProjectFolders]) -> Result<Freshness, StoreError> {
        let newest = listed(files, projects)?.iter().map(|listed| listed.file.modified).max();
        Ok(if newest == self.newest { Freshness::Current } else { Freshness::Stale })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{PdbId, PeImageId};
    use crate::memory_store::MemoryStore;
    use rusty_box_symbols_fixtures::pdb::{build_pdb, PdbFixture};

    fn pdb_bytes(guid: [u8; 16], age: u32) -> Vec<u8> {
        build_pdb(&PdbFixture::new(guid, age))
    }

    fn project() -> ProjectFolders {
        ProjectFolders { name: "mydriver".to_owned(), symbols: vec![PathBuf::from("work").join("build")] }
    }

    fn module(guid: [u8; 16], age: u32) -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: "mydriver.sys".to_owned(),
            image: PeImageId { time_date_stamp: 1, size_of_image: 0x1000 },
            pdb: Some(PdbId { name: "mydriver.pdb".to_owned(), guid: Guid::from_le_bytes(guid), age }),
        }
    }

    #[test]
    fn a_module_is_matched_to_its_own_build_not_to_a_namesake() {
        let mut files = MemoryStore::default();
        let build = PathBuf::from("work").join("build");
        files.insert(build.join("x64").join("mydriver.pdb"), pdb_bytes([1; 16], 3));
        files.insert(build.join("old").join("mydriver.pdb"), pdb_bytes([2; 16], 1));
        files.insert(build.join("mydriver.obj"), b"not a debug file".to_vec());
        let index = ProjectIndex::build(&mut files, &[project()]).unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(
            index.find(&module([1; 16], 3)),
            Some(&IndexedFile { project: "mydriver".to_owned(), path: build.join("x64").join("mydriver.pdb") })
        );
        assert_eq!(index.find(&module([1; 16], 4)), None, "a rebuilt binary has a new age");
        assert!(index.problems.is_empty());
    }

    #[test]
    fn a_broken_debug_file_is_a_problem_not_an_entry() {
        let mut files = MemoryStore::default();
        let path = PathBuf::from("work").join("build").join("broken.pdb");
        files.insert(path.clone(), b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0garbage".to_vec());
        let index = ProjectIndex::build(&mut files, &[project()]).unwrap();
        assert!(index.is_empty());
        assert_eq!(index.problems.len(), 1);
        assert_eq!(index.problems[0].path, path);
    }

    #[test]
    fn a_new_file_makes_the_index_stale() {
        let mut files = MemoryStore::default();
        let build = PathBuf::from("work").join("build");
        files.insert(build.join("a.pdb"), pdb_bytes([1; 16], 1));
        let index = ProjectIndex::build(&mut files, &[project()]).unwrap();
        assert_eq!(index.freshness(&mut files, &[project()]).unwrap(), Freshness::Current);
        files.insert(build.join("b.pdb"), pdb_bytes([2; 16], 1));
        assert_eq!(index.freshness(&mut files, &[project()]).unwrap(), Freshness::Stale);
    }
}
