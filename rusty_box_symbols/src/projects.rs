//! The owner's build folders, indexed by the identity each debug file
//! declares, so a module is matched to its own build's PDB or ELF debug
//! file and never to a namesake from another build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::identity::{BuildId, Guid, ModuleIdentity, PeImageId, TablesIdentity};
use crate::pe::{FileImage, HeaderError};
use crate::store::{FileStore, ListedFile, StoreError};
use crate::tables::{declared_identity, ParseError};

/// One project's folders of debug files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFolders {
    pub name: String,
    pub symbols: Vec<PathBuf>,
}

/// What a debug file or a binary is indexed under.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IndexKey {
    Pdb { guid: Guid, age: u32 },
    Build(BuildId),
    /// A PE binary: a loaded module's bytes the guest has not paged in are
    /// read from it.
    Image(PeImageId),
}

/// The extensions of PE binaries: executables, libraries, drivers, and the
/// other names Windows gives a loadable image.
const BINARY_EXTENSIONS: [&str; 8] = ["exe", "dll", "sys", "efi", "ocx", "cpl", "drv", "scr"];

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
    /// The binaries' file names, each once (any case).
    binary_names: Vec<String>,
    pub problems: Vec<IndexProblem>,
}

/// How much of a binary is read to index it: its headers and section table,
/// which link.exe keeps within the first page or two.
const BINARY_HEADER_BYTES: usize = 0x1_0000;

/// Whether a file is named as a PE binary is.
fn is_binary(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| BINARY_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// What reading a file as a binary came to.
enum BinaryRead {
    Image(PeImageId),
    /// It does not start as a PE file does: not a binary, whatever its name.
    NotBinary,
    Broken { reason: String },
}

/// The image identity of the binary at `path`, read from its headers alone;
/// the whole file only when they reach past [`BINARY_HEADER_BYTES`].
fn read_binary<F: FileStore>(files: &mut F, path: &Path) -> BinaryRead {
    let prefix = match files.read_prefix(path, BINARY_HEADER_BYTES) {
        Ok(prefix) => prefix,
        Err(error) => return BinaryRead::Broken { reason: error.to_string() },
    };
    if let Some(read) = image_identity(&prefix) {
        return read;
    }
    match files.read(path) {
        Ok(whole) => match FileImage::new(whole.as_slice()) {
            Ok(image) => BinaryRead::Image(image.headers().image_id()),
            Err(error) => BinaryRead::Broken { reason: error.to_string() },
        },
        Err(error) => BinaryRead::Broken { reason: error.to_string() },
    }
}

/// What `bytes` say of a binary, unless its headers reach past them.
fn image_identity(bytes: &[u8]) -> Option<BinaryRead> {
    if !bytes.starts_with(b"MZ") {
        return Some(BinaryRead::NotBinary);
    }
    match FileImage::new(bytes) {
        Ok(image) => Some(BinaryRead::Image(image.headers().image_id())),
        Err(HeaderError::Unreadable { .. }) if bytes.len() >= BINARY_HEADER_BYTES => None,
        Err(error) => Some(BinaryRead::Broken { reason: error.to_string() }),
    }
}

/// Whether a file may be a debug file or a binary: a PDB or a PE binary by
/// its extension, an ELF executable, library, kernel module or separate
/// debug file by its name.
fn is_candidate(path: &Path) -> bool {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => {
            let extension = extension.to_ascii_lowercase();
            BINARY_EXTENSIONS.contains(&extension.as_str())
                || extension == "pdb"
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
            if is_binary(&listed.file.path) {
                match read_binary(files, &listed.file.path) {
                    BinaryRead::Image(image) => index.add_binary(image, listed),
                    BinaryRead::NotBinary => {}
                    BinaryRead::Broken { reason } => index.problems.push(IndexProblem { path: listed.file.path, reason }),
                }
                continue;
            }
            let bytes = match files.read(&listed.file.path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    index.problems.push(IndexProblem { path: listed.file.path, reason: error.to_string() });
                    continue;
                }
            };
            if bytes.starts_with(b"MZ") {
                match FileImage::new(bytes.as_slice()) {
                    Ok(image) => index.add_binary(image.headers().image_id(), listed),
                    Err(error) => index.problems.push(IndexProblem { path: listed.file.path, reason: error.to_string() }),
                }
                continue;
            }
            let key = match declared_identity(&bytes) {
                Ok(TablesIdentity::Pdb { guid, age }) => IndexKey::Pdb { guid, age },
                Ok(TablesIdentity::Elf { build_id: Some(build_id) }) => IndexKey::Build(build_id),
                // An ELF file without a build-id cannot be matched.
                Ok(TablesIdentity::Elf { build_id: None }) => continue,
                // A file of another kind with a candidate's name is no
                // problem; one that claims a format and breaks is.
                Err(ParseError::NotSymbols | ParseError::CompilerTypes) => continue,
                Err(error) => {
                    index.problems.push(IndexProblem { path: listed.file.path, reason: error.to_string() });
                    continue;
                }
            };
            index.files.entry(key).or_insert(IndexedFile { project: listed.project, path: listed.file.path });
        }
        Ok(index)
    }

    /// Indexes `listed` as the binary of `image`, the first of an image
    /// winning, and its file name among the binaries' names.
    fn add_binary(&mut self, image: PeImageId, listed: ProjectFile) {
        if let Some(name) = listed.file.path.file_name().and_then(|name| name.to_str()) {
            if !self.binary_names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                self.binary_names.push(name.to_owned());
            }
        }
        self.files
            .entry(IndexKey::Image(image))
            .or_insert(IndexedFile { project: listed.project, path: listed.file.path });
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

    /// The project binary of the image `image`.
    #[must_use]
    pub fn find_image(&self, image: PeImageId) -> Option<&IndexedFile> {
        self.files.get(&IndexKey::Image(image))
    }

    /// The file names of the projects' binaries, each once, in the order
    /// indexed: what a guest's loaded modules are matched against to find
    /// the owner's own.
    #[must_use]
    pub fn binary_names(&self) -> &[String] {
        &self.binary_names
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
    use rusty_box_symbols_fixtures::pe::{pe64_file, Pe64Spec, SpecSection, PAGE_ALIGNED, SECTION_CODE};

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

    fn binary(time_date_stamp: u32) -> Vec<u8> {
        pe64_file(&Pe64Spec {
            time_date_stamp,
            size_of_image: 0x2000,
            image_base: 0x1_4000_0000,
            section_alignment: PAGE_ALIGNED,
            sections: vec![SpecSection { name: b".text", rva: 0x1000, bytes: vec![0xC3; 0x10], characteristics: SECTION_CODE }],
            exception: None,
            base_relocations: None,
            import_address_table: None,
        })
    }

    /// A build folder's binaries can be large: only their headers are read.
    #[test]
    fn a_binary_is_indexed_from_its_headers_alone() {
        let mut files = MemoryStore::default();
        let large = pe64_file(&Pe64Spec {
            time_date_stamp: 9,
            size_of_image: 0x5_1000,
            image_base: 0x1_4000_0000,
            section_alignment: PAGE_ALIGNED,
            sections: vec![SpecSection { name: b".text", rva: 0x1000, bytes: vec![0xC3; 0x5_0000], characteristics: SECTION_CODE }],
            exception: None,
            base_relocations: None,
            import_address_table: None,
        });
        assert!(large.len() > BINARY_HEADER_BYTES);
        files.insert(PathBuf::from("work").join("build").join("large.dll"), large);
        let index = ProjectIndex::build(&mut files, &[project()]).unwrap();
        assert!(index.find_image(PeImageId { time_date_stamp: 9, size_of_image: 0x5_1000 }).is_some());
        assert_eq!(files.whole_reads(), 0, "no binary is read whole");
    }

    #[test]
    fn a_binary_is_indexed_by_its_image_identity() {
        let mut files = MemoryStore::default();
        let build = PathBuf::from("work").join("build");
        files.insert(build.join("x64").join("mydriver.sys"), binary(7));
        files.insert(build.join("old").join("mydriver.sys"), binary(6));
        files.insert(build.join("readme.exe"), b"a text file with a binary's name".to_vec());
        let index = ProjectIndex::build(&mut files, &[project()]).unwrap();
        assert_eq!(
            index.find_image(PeImageId { time_date_stamp: 7, size_of_image: 0x2000 }),
            Some(&IndexedFile { project: "mydriver".to_owned(), path: build.join("x64").join("mydriver.sys") })
        );
        assert_eq!(index.find_image(PeImageId { time_date_stamp: 7, size_of_image: 0x3000 }), None);
        assert!(index.problems.is_empty(), "a file that is no image is passed by: {:?}", index.problems);
        assert_eq!(index.binary_names(), ["mydriver.sys"], "each name once");
    }

    #[test]
    fn a_binary_whose_headers_break_is_a_problem() {
        let mut files = MemoryStore::default();
        let mut broken = binary(7);
        broken.truncate(0x90);
        files.insert(PathBuf::from("work").join("build").join("mydriver.dll"), broken);
        let index = ProjectIndex::build(&mut files, &[project()]).unwrap();
        assert!(index.is_empty());
        assert_eq!(index.problems.len(), 1);
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
