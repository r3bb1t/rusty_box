//! Where the source of a file a module's tables name is on this computer
//! (spec §5): the maps the owner made this run by locating files, the
//! project's path maps, the path the build recorded, the project's source
//! folders by the longest shared suffix, then the copy the
//! PDB embeds; and whether what was found is what the build compiled.

use std::path::{Component, Path, PathBuf};

use rusty_box_symbols::source_path::{components, same_component, trailing_components_in_common};
use rusty_box_symbols::store::{FileStore, Presence, StoreError};
use rusty_box_symbols::tables::{EmbeddedSource, FileChecksum};

pub(crate) use super::folders::SourceFolders;
pub(crate) use super::text::{check, decode, SourceCheck, SourceText};

/// A source file past this is not read.
pub(crate) const MAX_SOURCE_BYTES: u64 = 16 << 20;
const TOO_LARGE: &str = "it is larger than the 16 MiB a source file is read up to";

/// A rewrite of the build machine's paths (`from`, as recorded) to where
/// the sources are here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PathMap {
    pub(crate) from: String,
    pub(crate) to: PathBuf,
}

/// The project whose symbols loaded the module: its maps and its source
/// folders' index.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectLookup<'a> {
    pub(crate) maps: &'a [PathMap],
    pub(crate) folders: &'a SourceFolders,
}

/// What finding one file goes on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceRequest<'a> {
    /// The path the build recorded.
    pub(crate) build_path: &'a str,
    pub(crate) checksum: Option<&'a FileChecksum>,
    /// The maps the owner made this run by locating files, for every
    /// module.
    pub(crate) located: &'a [PathMap],
    pub(crate) project: Option<ProjectLookup<'a>>,
    pub(crate) embedded: Option<&'a EmbeddedSource>,
}

/// How a file on disk was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FoundBy {
    /// By a map the owner made this run, locating a file.
    Located,
    PathMap,
    RecordedPath,
    SourceFolder,
}

/// Where the source is, or why it is not shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceFound {
    Disk { path: PathBuf, by: FoundBy, check: SourceCheck },
    Embedded { check: SourceCheck },
    /// The PDB embeds the file compressed, in a form this reader does not
    /// undo.
    EmbeddedCompressed { kind: u8 },
    /// Several files in the source folders share the longest suffix.
    Ambiguous { candidates: Vec<PathBuf> },
    Missing { build_path: String },
    Unreadable { path: PathBuf, reason: String },
}

/// What was found, and its text when there is text to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoadedSource {
    pub(crate) found: SourceFound,
    pub(crate) text: Option<SourceText>,
}

/// The file `build_path` names under `map.to`, when `build_path` begins
/// with every component of `map.from`. A remaining part that is not a plain
/// name (`..`, a drive) maps nothing.
fn mapped(build_path: &str, map: &PathMap) -> Option<PathBuf> {
    let mut rest = components(build_path);
    let mut prefix = components(&map.from).peekable();
    prefix.peek()?;
    for wanted in prefix {
        if !same_component(rest.next()?, wanted) {
            return None;
        }
    }
    let mut path = map.to.clone();
    for part in rest {
        let mut parts = Path::new(part).components();
        if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

/// Whether a path a debug file recorded may be opened as it is: not a UNC
/// path (`\\host\share`, `//host/share`) nor a device path (`\\?\`,
/// `\\.\`). Debug files come from servers and shared folders; on Windows,
/// asking whether `\\host\…` exists already connects to the host and offers
/// it the owner's credentials. Only the owner's own maps lead there.
fn on_this_machine(path: &str) -> bool {
    !(path.starts_with("\\\\") || path.starts_with("//"))
}

/// A path the file may be at, and the way it was named.
struct Candidate {
    path: PathBuf,
    by: FoundBy,
}

/// The PDB's copy of the file.
enum EmbeddedCopy<'a> {
    Plain { check: SourceCheck, bytes: &'a [u8] },
    Compressed { kind: u8 },
}

/// What the disk had.
enum OnDisk {
    Found { path: PathBuf, by: FoundBy, bytes: Vec<u8> },
    Ambiguous { candidates: Vec<PathBuf> },
    Unreadable { path: PathBuf, reason: String },
    Missing,
}

fn reason_of(error: &StoreError) -> String {
    match error {
        StoreError::TooLarge { .. } => TOO_LARGE.to_owned(),
        other => other.to_string(),
    }
}

/// The file at `path`, read whole.
fn read<F: FileStore>(files: &mut F, path: PathBuf, by: FoundBy) -> OnDisk {
    match files.read(&path) {
        Ok(bytes) if bytes.len() as u64 > MAX_SOURCE_BYTES => OnDisk::Unreadable { path, reason: TOO_LARGE.to_owned() },
        Ok(bytes) => OnDisk::Found { path, by, bytes },
        Err(error) => OnDisk::Unreadable { path, reason: reason_of(&error) },
    }
}

/// The first of the maps' paths and the recorded path that is there, else
/// the source folders' file sharing the longest suffix. A path whose
/// presence cannot be told is passed by; when nothing else is found, it is
/// what is reported.
fn on_disk<F: FileStore>(files: &mut F, request: &SourceRequest<'_>) -> OnDisk {
    let mut tries: Vec<Candidate> = request
        .located
        .iter()
        .filter_map(|map| mapped(request.build_path, map))
        .map(|path| Candidate { path, by: FoundBy::Located })
        .collect();
    if let Some(project) = request.project {
        tries.extend(
            project
                .maps
                .iter()
                .filter_map(|map| mapped(request.build_path, map))
                .map(|path| Candidate { path, by: FoundBy::PathMap }),
        );
    }
    if on_this_machine(request.build_path) {
        tries.push(Candidate { path: PathBuf::from(request.build_path), by: FoundBy::RecordedPath });
    }
    let mut unknown = None;
    for Candidate { path, by } in tries {
        match files.presence(&path) {
            Ok(Presence::Present) => return read(files, path, by),
            Ok(Presence::Absent) => {}
            Err(error) => {
                unknown.get_or_insert(OnDisk::Unreadable { reason: reason_of(&error), path });
            }
        }
    }
    if let Some(project) = request.project {
        let name = components(request.build_path).next_back().unwrap_or_default();
        let named = project.folders.named(name);
        let score = |path: &PathBuf| trailing_components_in_common(&path.to_string_lossy(), request.build_path);
        let best = named.iter().map(score).max().unwrap_or(0);
        let mut candidates: Vec<PathBuf> = named.iter().filter(|path| best > 0 && score(path) == best).cloned().collect();
        match candidates.len() {
            0 => {}
            1 => {
                if let Some(path) = candidates.pop() {
                    return read(files, path, FoundBy::SourceFolder);
                }
            }
            _ => return OnDisk::Ambiguous { candidates },
        }
    }
    unknown.unwrap_or(OnDisk::Missing)
}

/// Finds the source of `request`'s file and decodes it. The disk comes
/// first; the PDB's embedded copy is used when the disk has no file, or has
/// one the checksum says differs while the embedded copy matches it.
pub(crate) fn load_source<F: FileStore>(files: &mut F, request: &SourceRequest<'_>) -> LoadedSource {
    let disk = on_disk(files, request);
    let embedded = request.embedded.map(|embedded| match embedded {
        EmbeddedSource::Plain(bytes) => EmbeddedCopy::Plain { check: check(bytes, request.checksum), bytes },
        EmbeddedSource::Compressed { kind } => EmbeddedCopy::Compressed { kind: *kind },
    });
    let disk_check = match &disk {
        OnDisk::Found { bytes, .. } => Some(check(bytes, request.checksum)),
        OnDisk::Ambiguous { .. } | OnDisk::Unreadable { .. } | OnDisk::Missing => None,
    };
    let use_embedded = match &embedded {
        None => false,
        Some(EmbeddedCopy::Plain { check: embedded_check, .. }) => match disk_check {
            None => true,
            Some(SourceCheck::Matches) => false,
            Some(SourceCheck::Differs | SourceCheck::NoChecksum) => *embedded_check == SourceCheck::Matches,
        },
        // Nothing can be shown from it, so it is named only for want of
        // anything on disk.
        Some(EmbeddedCopy::Compressed { .. }) => matches!(disk, OnDisk::Missing),
    };
    match embedded {
        Some(EmbeddedCopy::Plain { check, bytes }) if use_embedded => {
            return LoadedSource { found: SourceFound::Embedded { check }, text: Some(decode(bytes)) };
        }
        Some(EmbeddedCopy::Compressed { kind }) if use_embedded => {
            return LoadedSource { found: SourceFound::EmbeddedCompressed { kind }, text: None };
        }
        Some(EmbeddedCopy::Plain { .. } | EmbeddedCopy::Compressed { .. }) | None => {}
    }
    match disk {
        OnDisk::Found { path, by, bytes } => LoadedSource {
            found: SourceFound::Disk { path, by, check: disk_check.unwrap_or(SourceCheck::NoChecksum) },
            text: Some(decode(&bytes)),
        },
        OnDisk::Ambiguous { candidates } => LoadedSource { found: SourceFound::Ambiguous { candidates }, text: None },
        OnDisk::Unreadable { path, reason } => LoadedSource { found: SourceFound::Unreadable { path, reason }, text: None },
        OnDisk::Missing => {
            LoadedSource { found: SourceFound::Missing { build_path: request.build_path.to_owned() }, text: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box_symbols::memory_store::MemoryStore;
    use rusty_box_symbols::tables::{ChecksumKind, EmbeddedSource, FileChecksum};
    use std::path::PathBuf;

    const BUILD: &str = r"C:\agent\src\io\read.c";

    fn md5_of(bytes: &[u8]) -> FileChecksum {
        use md5::Digest;
        FileChecksum { kind: ChecksumKind::Md5, bytes: md5::Md5::digest(bytes).to_vec() }
    }

    fn work() -> PathBuf {
        PathBuf::from("work")
    }

    fn request<'a>(project: Option<ProjectLookup<'a>>, checksum: Option<&'a FileChecksum>) -> SourceRequest<'a> {
        SourceRequest { build_path: BUILD, checksum, located: &[], project, embedded: None }
    }

    /// A file the owner located this run is found by its map, before the
    /// project's own maps, for a module of no project too.
    #[test]
    fn a_located_file_is_found_by_its_run_map_first() {
        let mut files = MemoryStore::default();
        let located = PathBuf::from("picked").join("src").join("io").join("read.c");
        files.insert(located.clone(), b"picked".to_vec());
        files.insert(work().join("src").join("io").join("read.c"), b"mapped".to_vec());
        let run_maps = [PathMap { from: r"C:\agent".to_owned(), to: PathBuf::from("picked") }];
        let maps = [PathMap { from: r"C:\agent".to_owned(), to: work() }];
        let folders = SourceFolders::default();
        let with_project = SourceRequest {
            located: &run_maps,
            ..request(Some(ProjectLookup { maps: &maps, folders: &folders }), None)
        };
        assert_eq!(load_source(&mut files, &with_project).found, disk(located.clone(), FoundBy::Located, SourceCheck::NoChecksum));
        let alone = SourceRequest { located: &run_maps, ..request(None, None) };
        assert_eq!(load_source(&mut files, &alone).found, disk(located, FoundBy::Located, SourceCheck::NoChecksum));
    }

    fn folders_over(files: &mut MemoryStore) -> SourceFolders {
        SourceFolders::index(files, &[work()]).unwrap()
    }

    fn disk(path: PathBuf, by: FoundBy, check: SourceCheck) -> SourceFound {
        SourceFound::Disk { path, by, check }
    }

    #[test]
    fn a_path_map_wins_over_the_recorded_path() {
        let mut files = MemoryStore::default();
        let mapped = work().join("src").join("io").join("read.c");
        files.insert(mapped.clone(), b"mapped".to_vec());
        files.insert(PathBuf::from(BUILD), b"recorded".to_vec());
        let maps = [PathMap { from: "c:/AGENT".to_owned(), to: work() }];
        let folders = SourceFolders::default();
        let loaded = load_source(&mut files, &request(Some(ProjectLookup { maps: &maps, folders: &folders }), None));
        assert_eq!(loaded.found, disk(mapped, FoundBy::PathMap, SourceCheck::NoChecksum));
        assert_eq!(loaded.text.map(|text| text.lines), Some(vec!["mapped".to_owned()]));
    }

    #[test]
    fn a_map_that_names_no_file_gives_way_to_the_recorded_path() {
        let mut files = MemoryStore::default();
        files.insert(PathBuf::from(BUILD), b"recorded".to_vec());
        let maps = [PathMap { from: r"C:\agent".to_owned(), to: work() }];
        let folders = SourceFolders::default();
        let loaded = load_source(&mut files, &request(Some(ProjectLookup { maps: &maps, folders: &folders }), None));
        assert_eq!(loaded.found, disk(PathBuf::from(BUILD), FoundBy::RecordedPath, SourceCheck::NoChecksum));
    }

    #[test]
    fn a_map_matches_whole_leading_components() {
        let mut files = MemoryStore::default();
        files.insert(work().join("ent").join("src").join("io").join("read.c"), b"wrong".to_vec());
        let maps = [PathMap { from: r"C:\ag".to_owned(), to: work() }];
        let folders = SourceFolders::default();
        let loaded = load_source(&mut files, &request(Some(ProjectLookup { maps: &maps, folders: &folders }), None));
        assert_eq!(loaded.found, SourceFound::Missing { build_path: BUILD.to_owned() });
    }

    #[test]
    fn the_folder_file_sharing_the_longest_suffix_is_found() {
        let mut files = MemoryStore::default();
        let best = work().join("src").join("io").join("read.c");
        files.insert(best.clone(), b"best".to_vec());
        files.insert(work().join("old").join("read.c"), b"old".to_vec());
        let folders = folders_over(&mut files);
        let loaded = load_source(&mut files, &request(Some(ProjectLookup { maps: &[], folders: &folders }), None));
        assert_eq!(loaded.found, disk(best, FoundBy::SourceFolder, SourceCheck::NoChecksum));
    }

    /// Review focus 3: never picked silently.
    #[test]
    fn two_files_sharing_the_longest_suffix_are_ambiguous() {
        let mut files = MemoryStore::default();
        let one = work().join("a").join("io").join("read.c");
        let two = work().join("b").join("io").join("read.c");
        files.insert(one.clone(), b"1".to_vec());
        files.insert(two.clone(), b"2".to_vec());
        files.insert(work().join("c").join("read.c"), b"3".to_vec());
        let folders = folders_over(&mut files);
        let loaded = load_source(&mut files, &request(Some(ProjectLookup { maps: &[], folders: &folders }), None));
        assert_eq!(loaded.found, SourceFound::Ambiguous { candidates: vec![one, two] });
        assert_eq!(loaded.text, None);
    }

    #[test]
    fn without_a_project_its_folders_are_not_searched() {
        let mut files = MemoryStore::default();
        files.insert(work().join("src").join("io").join("read.c"), b"x".to_vec());
        let loaded = load_source(&mut files, &request(None, None));
        assert_eq!(loaded.found, SourceFound::Missing { build_path: BUILD.to_owned() });
    }

    #[test]
    fn the_embedded_copy_is_used_last() {
        let mut files = MemoryStore::default();
        let embedded = EmbeddedSource::Plain(b"inside".to_vec());
        let checksum = md5_of(b"inside");
        let loaded = load_source(
            &mut files,
            &SourceRequest { checksum: Some(&checksum), embedded: Some(&embedded), ..request(None, None) },
        );
        assert_eq!(loaded.found, SourceFound::Embedded { check: SourceCheck::Matches });
        assert_eq!(loaded.text.map(|text| text.lines), Some(vec!["inside".to_owned()]));
        let compressed = EmbeddedSource::Compressed { kind: 2 };
        let loaded = load_source(&mut files, &SourceRequest { embedded: Some(&compressed), ..request(None, None) });
        assert_eq!(loaded.found, SourceFound::EmbeddedCompressed { kind: 2 });
        assert_eq!(loaded.text, None);
    }

    #[test]
    fn a_file_that_differs_is_shown_and_said_to_differ() {
        let mut files = MemoryStore::default();
        files.insert(PathBuf::from(BUILD), b"edited".to_vec());
        let checksum = md5_of(b"built");
        let loaded = load_source(&mut files, &request(None, Some(&checksum)));
        assert_eq!(loaded.found, disk(PathBuf::from(BUILD), FoundBy::RecordedPath, SourceCheck::Differs));
        assert_eq!(loaded.text.map(|text| text.lines), Some(vec!["edited".to_owned()]));
        let matching = md5_of(b"edited");
        let loaded = load_source(&mut files, &request(None, Some(&matching)));
        assert_eq!(loaded.found, disk(PathBuf::from(BUILD), FoundBy::RecordedPath, SourceCheck::Matches));
    }

    /// The build's own bytes beat an edited file on disk.
    #[test]
    fn an_embedded_copy_that_matches_beats_a_disk_file_that_differs() {
        let mut files = MemoryStore::default();
        files.insert(PathBuf::from(BUILD), b"edited".to_vec());
        let checksum = md5_of(b"built");
        let embedded = EmbeddedSource::Plain(b"built".to_vec());
        let loaded = load_source(
            &mut files,
            &SourceRequest { checksum: Some(&checksum), embedded: Some(&embedded), ..request(None, None) },
        );
        assert_eq!(loaded.found, SourceFound::Embedded { check: SourceCheck::Matches });
    }

    #[test]
    fn a_file_past_16_mib_is_unreadable() {
        let mut files = MemoryStore::default();
        files.insert(PathBuf::from(BUILD), vec![b'a'; MAX_SOURCE_BYTES as usize + 1]);
        let loaded = load_source(&mut files, &request(None, None));
        assert!(
            matches!(&loaded.found, SourceFound::Unreadable { path, reason } if *path == PathBuf::from(BUILD) && reason.contains("16 MiB")),
            "{:?}",
            loaded.found
        );
        assert_eq!(loaded.text, None);
    }

    #[test]
    fn a_file_that_does_not_read_is_unreadable() {
        let mut files = MemoryStore::default();
        files.insert(PathBuf::from(BUILD), b"x".to_vec());
        files.fail_reads_of(PathBuf::from(BUILD));
        let loaded = load_source(&mut files, &request(None, None));
        assert!(matches!(loaded.found, SourceFound::Unreadable { .. }), "{:?}", loaded.found);
    }

    /// A recorded path naming another machine (or a device) is never
    /// opened: on Windows, even asking whether `\\host\share\…` exists
    /// connects to the host and offers it the owner's credentials. The
    /// owner's own maps may still lead there.
    #[test]
    fn a_recorded_path_on_another_machine_is_not_touched() {
        for build in [r"\\evil.example\share\src\read.c", "//evil.example/share/src/read.c", r"\\?\C:\src\read.c", r"\\.\pipe\read.c"] {
            let mut files = MemoryStore::default();
            files.insert(PathBuf::from(build), b"bait".to_vec());
            let loaded = load_source(&mut files, &SourceRequest { build_path: build, ..request(None, None) });
            assert_eq!(loaded.found, SourceFound::Missing { build_path: build.to_owned() }, "{build}");
        }
        let mut files = MemoryStore::default();
        let mapped = work().join("src").join("read.c");
        files.insert(mapped.clone(), b"mine".to_vec());
        let maps = [PathMap { from: r"\\buildserver\share".to_owned(), to: work() }];
        let folders = SourceFolders::default();
        let request = SourceRequest {
            build_path: r"\\buildserver\share\src\read.c",
            ..request(Some(ProjectLookup { maps: &maps, folders: &folders }), None)
        };
        assert_eq!(load_source(&mut files, &request).found, disk(mapped, FoundBy::PathMap, SourceCheck::NoChecksum));
    }

    #[test]
    fn nothing_found_is_missing() {
        let loaded = load_source(&mut MemoryStore::default(), &request(None, None));
        assert_eq!(loaded.found, SourceFound::Missing { build_path: BUILD.to_owned() });
    }
}
