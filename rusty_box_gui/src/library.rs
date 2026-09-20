//! The VM library: one TOML file per virtual machine, in one per-user folder.
//!
//! The folder is the only place the shell reads VM configurations from on its
//! own. A file dropped into the working directory, or anywhere else, is never
//! picked up (CWE-427, uncontrolled search path); outside the folder the
//! launcher reads only a file the user names with `--config`. A VM's file name
//! is derived from its name but cut down to a safe alphabet, so no name can
//! climb out of the folder or land on a Windows device name.

use crate::config::{load_toml_file, resolve_config_in, ResolvedConfig, DEFAULT_CONFIG_FILE};
pub use crate::error::LibraryError;
use crate::error::RunError;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The longest file stem the library writes, in bytes, a uniqueness suffix
/// (`-2`, `-3`, …) included.
const MAX_STEM_LEN: usize = 64;
/// The file recording which VM the shell showed last. Its name has no `.toml`
/// extension, so the listing never takes it for a VM.
const LAST_SELECTED_FILE: &str = ".last";
/// The extension of every VM file, matched exactly: the library writes no
/// other spelling, so a `.TOML` file is not one of its own.
const VM_EXTENSION: &str = "toml";
/// The name a VM gets when the file describing it names none.
pub const DEFAULT_VM_NAME: &str = "Rusty Box";

/// A VM file's stem: its file name in the library folder, without `.toml`.
///
/// Every stem is one plain file name that does not start with a dot, so
/// joining it to the folder never leaves the folder and never names one of
/// the library's own dot files. [`VmStem::parse`] is the only way to make one:
/// the listing, the last-selected record and the stems the library coins all
/// pass through it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VmStem(String);

impl VmStem {
    /// The stem `text` spells, when `text` is exactly its own file name —
    /// `Path::file_name` returns all of it, so it holds no folder, no `..`,
    /// no root and no drive — does not start with a dot, and is its own
    /// trim: the last-selected record is read trimmed, so a stem with
    /// whitespace at either end could never be recalled.
    pub fn parse(text: &str) -> Result<Self, LibraryError> {
        let is_own_file_name = Path::new(text).file_name() == Some(OsStr::new(text));
        if is_own_file_name && !text.starts_with('.') && text.trim() == text {
            Ok(Self(text.to_owned()))
        } else {
            Err(LibraryError::InvalidStem {
                text: text.to_owned(),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A VM the library holds.
#[derive(Clone, Debug)]
pub struct LibraryVm {
    pub stem: VmStem,
    pub name: String,
    pub config: ResolvedConfig,
}

/// A file in the library folder that does not load as a VM. It is listed so
/// the user sees it, never loaded and never silently dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokenVmFile {
    pub path: PathBuf,
    pub error: String,
}

/// What the library folder holds.
#[derive(Clone, Debug, Default)]
pub struct LibraryContents {
    /// The VMs that load, ordered by stem without case.
    pub vms: Vec<LibraryVm>,
    pub broken: Vec<BrokenVmFile>,
}

impl LibraryContents {
    pub fn is_empty(&self) -> bool {
        self.vms.is_empty() && self.broken.is_empty()
    }
}

/// The library folder.
#[derive(Clone, Debug)]
pub struct VmLibrary {
    dir: PathBuf,
}

impl VmLibrary {
    /// Opens the library kept in `dir`, creating the folder when it is missing.
    pub fn open(dir: PathBuf) -> Result<Self, LibraryError> {
        fs::create_dir_all(&dir).map_err(|source| LibraryError::CreateDir {
            path: dir.clone(),
            source,
        })?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file `stem` names.
    pub fn path_of(&self, stem: &VmStem) -> PathBuf {
        self.dir.join(format!("{}.{VM_EXTENSION}", stem.0))
    }

    /// The stem of the file `path` names, when it is one of this library's
    /// VM files: a regular file directly in the folder, its extension spelled
    /// `toml` exactly, its stem one. Both sides are compared canonical
    /// (`fs::canonicalize`), so a spelling that differs in case, an 8.3 short
    /// name or a link through another folder still names the file, and the
    /// stem returned is the file's own spelling, the one the listing shows.
    /// `None` for any other path, and for one that cannot be resolved.
    pub fn stem_of_file(&self, path: &Path) -> Option<VmStem> {
        let file = fs::canonicalize(path).ok()?;
        let dir = fs::canonicalize(&self.dir).ok()?;
        let in_folder = file.is_file()
            && file.parent() == Some(dir.as_path())
            && file.extension() == Some(OsStr::new(VM_EXTENSION));
        if !in_folder {
            return None;
        }
        stem_of(&file).ok()
    }

    /// Whether the folder holds no `*.toml` file at all, broken ones
    /// included — what [`LibraryContents::is_empty`] answers after a load,
    /// found without parsing any file.
    pub fn is_empty(&self) -> Result<bool, LibraryError> {
        Ok(self.vm_files()?.is_empty())
    }

    /// Every `*.toml` file directly in the folder, ordered by stem without
    /// case (see [`library_order`]). A file whose stem is not one — not
    /// UTF-8, starting with a dot, or with whitespace at either end — is
    /// listed as broken, like one whose contents do not load.
    pub fn load(&self) -> Result<LibraryContents, LibraryError> {
        let mut contents = LibraryContents::default();
        let mut files = Vec::new();
        for path in self.vm_files()? {
            match stem_of(&path) {
                Ok(stem) => files.push((stem, path)),
                Err(error) => contents.broken.push(BrokenVmFile {
                    path,
                    error: error.to_string(),
                }),
            }
        }
        files.sort_by(|a, b| library_order(&a.0, &b.0));
        for (stem, path) in files {
            match read_vm(&self.dir, &path) {
                Ok(described) => contents.vms.push(LibraryVm {
                    name: described.name.unwrap_or_else(|| stem.0.clone()),
                    stem,
                    config: described.config,
                }),
                Err(error) => contents.broken.push(BrokenVmFile {
                    path,
                    error: error.to_string(),
                }),
            }
        }
        Ok(contents)
    }

    /// Adds a VM called `name` in a new file and returns the file's stem.
    pub fn create(&self, name: &str, config: &ResolvedConfig) -> Result<VmStem, LibraryError> {
        let stem = self.unused_stem(name)?;
        self.save(&stem, name, config)?;
        Ok(stem)
    }

    /// Rewrites the file of `stem` to describe `config` under `name`, trimmed;
    /// a name that is blank after trimming is stored as none, so the VM shows
    /// under its stem. Relative paths in `config` are made absolute against
    /// the working directory first — the base the running machine opened them
    /// against — so the file stands on its own wherever it is read.
    pub fn save(
        &self,
        stem: &VmStem,
        name: &str,
        config: &ResolvedConfig,
    ) -> Result<(), LibraryError> {
        let name = name.trim();
        let mut file = with_absolute_paths(config)?.to_file_config();
        file.vm.name = (!name.is_empty()).then(|| name.to_owned());
        let shown = if name.is_empty() { stem.as_str() } else { name };
        let text = toml::to_string_pretty(&file).map_err(|source| LibraryError::Serialize {
            name: shown.to_owned(),
            source,
        })?;
        write_atomically(&self.path_of(stem), text.as_bytes())
    }

    /// Removes the file of `stem`. A file already gone counts as removed.
    pub fn delete(&self, stem: &VmStem) -> Result<(), LibraryError> {
        remove_if_present(&self.path_of(stem))
    }

    /// Removes `path`, a file the listing reported as broken. Refuses any path
    /// that is not directly in the library folder.
    pub fn delete_file(&self, path: &Path) -> Result<(), LibraryError> {
        if path.parent() != Some(self.dir.as_path()) {
            return Err(LibraryError::OutsideLibrary {
                path: path.to_owned(),
                dir: self.dir.clone(),
            });
        }
        remove_if_present(path)
    }

    /// The VM the shell showed last, when its file is still in the library.
    /// A record that is missing, unreadable or not a stem is no record.
    pub fn last_selected(&self) -> Option<VmStem> {
        let recorded = match fs::read_to_string(self.dir.join(LAST_SELECTED_FILE)) {
            Ok(recorded) => recorded,
            Err(_) => return None,
        };
        let stem = match VmStem::parse(recorded.trim()) {
            Ok(stem) => stem,
            Err(_) => return None,
        };
        self.path_of(&stem).is_file().then_some(stem)
    }

    /// Records `stem` as the VM the shell shows at its next launch.
    pub fn remember_selected(&self, stem: &VmStem) -> Result<(), LibraryError> {
        write_atomically(&self.dir.join(LAST_SELECTED_FILE), stem.0.as_bytes())
    }

    /// Adds the VM a `rusty_box.toml` beside the executable describes and
    /// returns its stem; `None` when there is no such file. That folder
    /// belongs to whoever installed the program — anyone able to write there
    /// could replace the program itself — unlike the working directory.
    /// Relative paths in the file resolve against that folder, so the copy in
    /// the library stands on its own.
    pub fn import_bundled_vm(&self, exe_dir: &Path) -> Result<Option<VmStem>, RunError> {
        let path = exe_dir.join(DEFAULT_CONFIG_FILE);
        if !path.is_file() {
            return Ok(None);
        }
        let described = read_vm(exe_dir, &path)?;
        let name = described.name.as_deref().unwrap_or(DEFAULT_VM_NAME);
        Ok(Some(self.create(name, &described.config)?))
    }

    /// Every entry directly in the folder, whatever it is.
    fn entries(&self) -> Result<Vec<PathBuf>, LibraryError> {
        let read_error = |source: io::Error| LibraryError::Read {
            path: self.dir.clone(),
            source,
        };
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(read_error)? {
            entries.push(entry.map_err(read_error)?.path());
        }
        Ok(entries)
    }

    /// The regular files directly in the folder.
    fn files(&self) -> Result<Vec<PathBuf>, LibraryError> {
        let mut files = self.entries()?;
        files.retain(|path| path.is_file());
        Ok(files)
    }

    /// The `*.toml` files directly in the folder, the extension spelled
    /// exactly.
    fn vm_files(&self) -> Result<Vec<PathBuf>, LibraryError> {
        let mut files = self.files()?;
        files.retain(|path| path.extension() == Some(OsStr::new(VM_EXTENSION)));
        Ok(files)
    }

    /// A stem for `name` that no entry in the folder has: a folder, or
    /// anything else, named `<stem>.toml` takes the stem as a file does,
    /// since the VM's file could not be written over it. Taken stems are
    /// compared without case, and under any spelling of the extension,
    /// because Windows and macOS folders ignore case. A suffix `-2`, `-3`, …
    /// makes the stem unique, and the base is cut so the whole stays within
    /// `MAX_STEM_LEN`.
    fn unused_stem(&self, name: &str) -> Result<VmStem, LibraryError> {
        let taken: HashSet<String> = self
            .entries()?
            .iter()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(VM_EXTENSION))
            })
            .filter_map(|path| path.file_stem().and_then(OsStr::to_str))
            .map(str::to_ascii_lowercase)
            .collect();
        let base = safe_stem(name);
        let mut candidate = base.clone();
        let mut suffix: u64 = 2;
        while taken.contains(&candidate) {
            candidate = suffixed(&base, suffix);
            suffix += 1;
        }
        VmStem::parse(&candidate)
    }
}

/// What one file describes.
struct DescribedVm {
    /// `None` when the file has no `[vm] name`, or an empty one.
    name: Option<String>,
    config: ResolvedConfig,
}

/// The VM the file at `path` describes; relative paths in it resolve against
/// `dir`.
fn read_vm(dir: &Path, path: &Path) -> Result<DescribedVm, RunError> {
    let file = load_toml_file(path)?;
    let name = file.vm.name.clone().filter(|name| !name.trim().is_empty());
    let config = resolve_config_in(file, dir)?;
    Ok(DescribedVm { name, config })
}

/// The order the listing shows stems in: without case, so `Beta` follows
/// `alpha`; two stems equal without case keep their byte order, so the order
/// is total.
fn library_order(a: &VmStem, b: &VmStem) -> Ordering {
    a.0.to_lowercase()
        .cmp(&b.0.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// The stem of the VM file at `path`. A file name that is not UTF-8 is not a
/// stem: a lossy rendering of it would name a different file.
fn stem_of(path: &Path) -> Result<VmStem, LibraryError> {
    let stem = path.file_stem().unwrap_or(OsStr::new(""));
    match stem.to_str() {
        Some(text) => VmStem::parse(text),
        None => Err(LibraryError::InvalidStem {
            text: stem.to_string_lossy().into_owned(),
        }),
    }
}

/// `config` with every relative path made absolute against the working
/// directory. An empty path names no file and stays empty; an absolute one is
/// kept as it is.
fn with_absolute_paths(config: &ResolvedConfig) -> Result<ResolvedConfig, LibraryError> {
    let mut anchored = config.clone();
    anchored.bios = absolute_path(&config.bios)?;
    if let Some(vga_bios) = &mut anchored.vga_bios {
        *vga_bios = absolute_path(vga_bios)?;
    }
    if let Some(disk) = &mut anchored.disk {
        disk.path = absolute_path(&disk.path)?;
        if let Some(creation) = &mut disk.creation {
            creation.path = absolute_path(&creation.path)?;
        }
    }
    if let Some(cdrom) = &mut anchored.cdrom {
        cdrom.path = absolute_path(&cdrom.path)?;
    }
    Ok(anchored)
}

fn absolute_path(path: &Path) -> Result<PathBuf, LibraryError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Ok(path.to_owned());
    }
    std::path::absolute(path).map_err(|source| LibraryError::AbsolutePath {
        path: path.to_owned(),
        source,
    })
}

/// The file stem the library gives a VM called `name`: its ASCII letters and
/// digits in lower case, each run of anything else one `-`, at most
/// `MAX_STEM_LEN` bytes, never empty, and never a name Windows reserves for a
/// device. No stem it returns can name a parent folder, a drive or a device.
pub fn safe_stem(name: &str) -> String {
    let mut stem = String::with_capacity(MAX_STEM_LEN);
    let mut separator_pending = false;
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            separator_pending = !stem.is_empty();
            continue;
        }
        if separator_pending {
            stem.push('-');
            separator_pending = false;
        }
        stem.push(ch.to_ascii_lowercase());
        if stem.len() >= MAX_STEM_LEN {
            break;
        }
    }
    stem.truncate(MAX_STEM_LEN);
    while stem.ends_with('-') {
        stem.pop();
    }
    if stem.is_empty() {
        stem.push_str("vm");
    }
    if is_windows_device_name(&stem) {
        stem.push_str("-vm");
    }
    stem
}

/// `base-suffix`, with `base` — a `safe_stem` result, so ASCII — cut so the
/// whole stays within `MAX_STEM_LEN` and does not end in a double dash.
fn suffixed(base: &str, suffix: u64) -> String {
    let suffix = format!("-{suffix}");
    let mut stem = base.to_owned();
    stem.truncate(MAX_STEM_LEN - suffix.len());
    while stem.ends_with('-') {
        stem.pop();
    }
    stem.push_str(&suffix);
    stem
}

/// Whether Windows reserves `stem` for a device (`con`, `prn`, `aux`, `nul`,
/// `com0`–`com9`, `lpt0`–`lpt9`): a file of that name opens the device.
fn is_windows_device_name(stem: &str) -> bool {
    if matches!(stem, "con" | "prn" | "aux" | "nul") {
        return true;
    }
    ["com", "lpt"].iter().any(|prefix| {
        stem.strip_prefix(prefix)
            .is_some_and(|rest| rest.len() == 1 && rest.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

/// The file a write to `path` from this process goes to first:
/// `<path>.<pid>.partial`. Each process has its own, so two processes saving
/// the same file never share a partial, and the last complete save wins.
fn partial_path(path: &Path) -> PathBuf {
    let mut partial = path.as_os_str().to_owned();
    partial.push(format!(".{}.partial", std::process::id()));
    PathBuf::from(partial)
}

/// Writes `bytes` to this process's partial file beside `path` (see
/// [`partial_path`]), flushes it to the disk and renames it over `path`, so
/// `path` is either what it was or the whole of `bytes`, never part of them.
/// A partial file whose write or rename failed is removed again, and the
/// failure that stopped the write is what the caller gets.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let partial = partial_path(path);
    let written = write_synced(&partial, bytes)
        .map_err(|source| LibraryError::Write {
            path: partial.clone(),
            source,
        })
        .and_then(|()| {
            fs::rename(&partial, path).map_err(|source| LibraryError::Write {
                path: path.to_owned(),
                source,
            })
        });
    if written.is_err() {
        match remove_if_present(&partial) {
            Ok(()) => {}
            Err(error) => tracing::warn!(%error, "the partial file of a failed write stays behind"),
        }
    }
    written
}

/// Creates `path` holding `bytes` and waits until they are on the disk, so a
/// rename that follows never installs an empty file.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn remove_if_present(path: &Path) -> Result<(), LibraryError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LibraryError::Delete {
            path: path.to_owned(),
            source,
        }),
    }
}

/// The per-user folder the desktop shell keeps its library in:
/// `%APPDATA%\rusty_box\vms` on Windows,
/// `~/Library/Application Support/rusty_box/vms` on macOS, and
/// `$XDG_DATA_HOME/rusty_box/vms` (by default `~/.local/share/rusty_box/vms`)
/// elsewhere. `None` when the platform names no such folder.
#[cfg(not(target_os = "android"))]
pub fn default_library_dir() -> Option<PathBuf> {
    let from_env = |variable: &str| absolute_dir(std::env::var_os(variable));
    let base = if cfg!(windows) {
        from_env("APPDATA")
    } else if cfg!(target_os = "macos") {
        from_env("HOME").map(|home| home.join("Library").join("Application Support"))
    } else {
        from_env("XDG_DATA_HOME")
            .or_else(|| from_env("HOME").map(|home| home.join(".local").join("share")))
    };
    base.map(|base| base.join("rusty_box").join("vms"))
}

/// The directory an environment value names, when it names an absolute one.
/// A relative value would resolve against the working directory — where a
/// planted file lives — so it counts as unset.
#[cfg(not(target_os = "android"))]
fn absolute_dir(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CdromToml, CpuCapabilities, Engine, FileConfig, RomToml};

    /// A file that resolves in every build: a BIOS and a CD to boot from.
    const SAMPLE_TOML: &str = "[rom]\nbios = \"bios.bin\"\n\n[cdrom]\npath = \"boot.iso\"\n";

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after the Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rusty-box-library-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn remove_dir(dir: &Path) {
        fs::remove_dir_all(dir).expect("remove scratch dir");
    }

    /// `SAMPLE_TOML` as a configuration, resolved with `dir` as its folder.
    fn sample_config(dir: &Path) -> ResolvedConfig {
        let file = FileConfig {
            rom: RomToml {
                bios: Some(PathBuf::from("bios.bin")),
                vga_bios: None,
            },
            cdrom: Some(CdromToml {
                path: Some(PathBuf::from("boot.iso")),
                channel: None,
                drive: None,
            }),
            ..FileConfig::default()
        };
        resolve_config_in(file, dir).expect("a BIOS and a CD are all a config needs")
    }

    fn toml_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("list dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn stem(text: &str) -> VmStem {
        VmStem::parse(text).expect("a stem")
    }

    #[test]
    fn a_stem_keeps_letters_and_digits_and_joins_the_rest_with_dashes() {
        assert_eq!(safe_stem("Alpine Linux 3.24"), "alpine-linux-3-24");
        assert_eq!(safe_stem("  Win7 (x64)  "), "win7-x64");
    }

    #[test]
    fn no_stem_climbs_out_of_the_folder_or_names_a_drive() {
        assert_eq!(safe_stem("../../evil"), "evil");
        assert_eq!(safe_stem("C:\\Windows\\system32"), "c-windows-system32");
        assert_eq!(safe_stem("/etc/passwd"), "etc-passwd");
    }

    #[test]
    fn no_stem_is_a_windows_device_name() {
        assert_eq!(safe_stem("CON"), "con-vm");
        assert_eq!(safe_stem("com1"), "com1-vm");
        assert_eq!(safe_stem("Lpt9"), "lpt9-vm");
        assert_eq!(safe_stem("console"), "console");
        assert_eq!(safe_stem("com10"), "com10");
    }

    #[test]
    fn a_name_with_nothing_usable_becomes_vm() {
        assert_eq!(safe_stem(""), "vm");
        assert_eq!(safe_stem("!!!"), "vm");
        assert_eq!(safe_stem("Мой ВМ"), "vm");
    }

    #[test]
    fn a_long_name_is_cut_without_a_trailing_dash() {
        let stem = safe_stem(&format!("{} tail", "a".repeat(63)));
        assert!(stem.len() <= MAX_STEM_LEN);
        assert!(!stem.ends_with('-'));
    }

    #[test]
    fn a_stem_is_exactly_one_plain_file_name() {
        for accepted in ["alpine", "ALPINE", "DOS Lab", "alpine-2", "a.b"] {
            assert_eq!(stem(accepted).as_str(), accepted);
        }
        for refused in [
            "", ".", "..", "../escape", "a/b", "/a", ".hidden", "a/", " alpine", "alpine ",
            "\talpine", "alpine\n",
        ] {
            assert!(
                matches!(VmStem::parse(refused), Err(LibraryError::InvalidStem { .. })),
                "{refused:?} was accepted"
            );
        }
        if cfg!(windows) {
            for refused in ["a\\b", "C:a", "\\a"] {
                assert!(VmStem::parse(refused).is_err(), "{refused:?} was accepted");
            }
        }
    }

    #[test]
    fn a_created_vm_is_there_at_the_next_load() {
        let dir = scratch_dir("create");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let mut config = sample_config(&dir);
        config.engine = Engine::Whp;
        config.cpu_capabilities = CpuCapabilities::HostShared;

        let stem = library.create("Alpine", &config).expect("create");
        let contents = library.load().expect("load");

        assert_eq!(stem.as_str(), "alpine");
        assert_eq!(contents.vms.len(), 1);
        assert_eq!(contents.vms[0].name, "Alpine");
        assert_eq!(contents.vms[0].config.engine, Engine::Whp);
        assert_eq!(contents.vms[0].config, config);
        assert!(contents.broken.is_empty());
        remove_dir(&dir);
    }

    /// A path names a library VM when it resolves to one of the folder's own
    /// VM files, however it is spelled; any other file names none.
    #[test]
    fn a_library_file_is_known_by_its_stem_however_its_path_is_spelled() {
        let dir = scratch_dir("stem-of-file");
        let elsewhere = scratch_dir("stem-of-file-elsewhere");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = library.create("Alpine", &sample_config(&dir)).expect("create");
        fs::write(elsewhere.join("alpine.toml"), SAMPLE_TOML).expect("write");
        fs::write(dir.join("notes.txt"), "notes").expect("write");
        fs::create_dir(dir.join("folder.toml")).expect("a folder named like a VM file");
        let roundabout = dir
            .join("..")
            .join(dir.file_name().expect("the scratch folder has a name"))
            .join("alpine.toml");

        assert_eq!(library.stem_of_file(&library.path_of(&stem)), Some(stem.clone()));
        assert_eq!(library.stem_of_file(&roundabout), Some(stem.clone()));
        if cfg!(windows) {
            assert_eq!(library.stem_of_file(&dir.join("ALPINE.TOML")), Some(stem.clone()));
        }
        assert_eq!(library.stem_of_file(&elsewhere.join("alpine.toml")), None);
        assert_eq!(library.stem_of_file(&dir.join("notes.txt")), None);
        assert_eq!(library.stem_of_file(&dir.join("folder.toml")), None);
        assert_eq!(library.stem_of_file(&dir.join("missing.toml")), None);
        remove_dir(&dir);
        remove_dir(&elsewhere);
    }

    #[test]
    fn a_second_vm_of_the_same_name_gets_its_own_file() {
        let dir = scratch_dir("unique");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("ALPINE.toml"), SAMPLE_TOML).expect("write");
        let config = sample_config(&dir);

        let first = library.create("alpine", &config).expect("first");
        let second = library.create("Alpine", &config).expect("second");

        assert_eq!(first.as_str(), "alpine-2");
        assert_eq!(second.as_str(), "alpine-3");
        remove_dir(&dir);
    }

    /// A folder named like a VM file takes its stem as a file does: the VM's
    /// file could not be written over it, so the new VM gets the next stem.
    #[test]
    fn a_folder_named_like_a_vm_file_takes_its_stem() {
        let dir = scratch_dir("folder-stem");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::create_dir(dir.join("alpine.toml")).expect("a folder named like a VM file");

        let stem = library.create("Alpine", &sample_config(&dir)).expect("create");

        assert_eq!(stem.as_str(), "alpine-2");
        assert!(dir.join("alpine-2.toml").is_file());
        remove_dir(&dir);
    }

    #[test]
    fn a_name_as_long_as_a_stem_created_twice_still_fits() {
        let dir = scratch_dir("long");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let config = sample_config(&dir);
        let name = "a".repeat(MAX_STEM_LEN);

        let first = library.create(&name, &config).expect("first");
        let second = library.create(&name, &config).expect("second");
        library.remember_selected(&second).expect("remember");

        assert_eq!(first.as_str().len(), MAX_STEM_LEN);
        assert_eq!(second.as_str().len(), MAX_STEM_LEN);
        assert!(second.as_str().ends_with("-2"));
        assert_eq!(library.last_selected(), Some(second));
        remove_dir(&dir);
    }

    #[test]
    fn load_orders_by_stem_and_lists_only_toml_files() {
        let dir = scratch_dir("order");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let config = sample_config(&dir);
        let b = library.create("b", &config).expect("b");
        library.create("a", &config).expect("a");
        library.remember_selected(&b).expect("remember");
        fs::write(dir.join("x.toml.partial"), "junk").expect("partial");
        fs::write(dir.join("notes.txt"), "junk").expect("notes");
        fs::write(dir.join("shout.TOML"), SAMPLE_TOML).expect("upper-case extension");

        let contents = library.load().expect("load");

        let names: Vec<&str> = contents.vms.iter().map(|vm| vm.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(contents.broken.is_empty());
        remove_dir(&dir);
    }

    /// Byte order puts every upper-case stem before every lower-case one;
    /// the listing orders them as a reader does.
    #[test]
    fn load_orders_stems_without_case() {
        let dir = scratch_dir("case-order");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("Zeta.toml"), SAMPLE_TOML).expect("write");
        fs::write(dir.join("MID.toml"), SAMPLE_TOML).expect("write");
        library.create("alpha", &sample_config(&dir)).expect("alpha");

        let contents = library.load().expect("load");

        let stems: Vec<&str> = contents.vms.iter().map(|vm| vm.stem.as_str()).collect();
        assert_eq!(stems, ["alpha", "MID", "Zeta"]);
        remove_dir(&dir);
    }

    /// Two stems equal without case — possible only on a case-sensitive
    /// file system — keep their byte order, so the listing's order is total.
    #[test]
    fn library_order_breaks_a_case_tie_by_byte_order() {
        use std::cmp::Ordering;
        assert_eq!(library_order(&stem("alpha"), &stem("Beta")), Ordering::Less);
        assert_eq!(library_order(&stem("Alpine"), &stem("alpine")), Ordering::Less);
        assert_eq!(library_order(&stem("alpine"), &stem("Alpine")), Ordering::Greater);
        assert_eq!(library_order(&stem("alpine"), &stem("alpine")), Ordering::Equal);
    }

    #[test]
    fn is_empty_is_true_only_while_the_folder_holds_no_toml_file() {
        let dir = scratch_dir("is-empty");
        let library = VmLibrary::open(dir.clone()).expect("open");
        assert!(library.is_empty().expect("empty folder"));

        fs::write(dir.join("bad.toml"), "memory_mib = [").expect("write");
        assert!(!library.is_empty().expect("broken file"));

        fs::remove_file(dir.join("bad.toml")).expect("remove");
        library.create("Alpine", &sample_config(&dir)).expect("create");
        assert!(!library.is_empty().expect("valid VM"));
        remove_dir(&dir);
    }

    #[test]
    fn a_file_that_does_not_load_is_reported_not_dropped() {
        let dir = scratch_dir("broken");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("bad.toml"), "memory_mib = [").expect("write");

        let contents = library.load().expect("load");

        assert!(contents.vms.is_empty());
        assert_eq!(contents.broken.len(), 1);
        assert_eq!(contents.broken[0].path, dir.join("bad.toml"));
        assert!(!contents.broken[0].error.is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn a_dot_file_with_the_extension_is_reported_not_dropped() {
        let dir = scratch_dir("dotfile");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join(".hidden.toml"), SAMPLE_TOML).expect("write");

        let contents = library.load().expect("load");

        assert!(contents.vms.is_empty());
        assert_eq!(contents.broken.len(), 1);
        assert_eq!(contents.broken[0].path, dir.join(".hidden.toml"));
        assert!(contents.broken[0].error.contains("not a VM file stem"));
        remove_dir(&dir);
    }

    /// A stem with a space at either end could not be recalled from the
    /// last-selected record, which is read trimmed, so such a file is not a
    /// VM: it is listed as broken, where the user can see and delete it.
    #[test]
    fn a_hand_placed_file_with_an_edge_space_is_listed_as_broken() {
        let dir = scratch_dir("edge-space");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join(" alpine.toml"), SAMPLE_TOML).expect("write");

        let contents = library.load().expect("load");

        assert!(contents.vms.is_empty());
        assert_eq!(contents.broken.len(), 1);
        assert_eq!(contents.broken[0].path, dir.join(" alpine.toml"));
        assert!(contents.broken[0].error.contains("not a VM file stem"));
        remove_dir(&dir);
    }

    #[test]
    fn a_file_without_a_vm_table_is_shown_under_its_file_name() {
        let dir = scratch_dir("unnamed");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("dos-lab.toml"), SAMPLE_TOML).expect("write");

        let contents = library.load().expect("load");

        assert_eq!(contents.vms[0].name, "dos-lab");
        assert_eq!(contents.vms[0].config.bios, dir.join("bios.bin"));
        remove_dir(&dir);
    }

    #[test]
    fn hand_placed_stems_survive_the_last_selected_record() {
        let dir = scratch_dir("hand-placed");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("ALPINE.toml"), SAMPLE_TOML).expect("write");
        fs::write(dir.join("DOS Lab.toml"), SAMPLE_TOML).expect("write");

        let contents = library.load().expect("load");

        assert_eq!(contents.vms.len(), 2);
        for vm in &contents.vms {
            library.remember_selected(&vm.stem).expect("remember");
            assert_eq!(library.last_selected(), Some(vm.stem.clone()));
        }
        remove_dir(&dir);
    }

    #[test]
    fn renaming_a_vm_keeps_its_file() {
        let dir = scratch_dir("rename");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let config = sample_config(&dir);
        let stem = library.create("Alpine", &config).expect("create");

        library.save(&stem, "Alpine edge", &config).expect("save");
        let contents = library.load().expect("load");

        assert_eq!(toml_names(&dir), ["alpine.toml"]);
        assert_eq!(contents.vms[0].stem, stem);
        assert_eq!(contents.vms[0].name, "Alpine edge");
        remove_dir(&dir);
    }

    #[test]
    fn a_name_is_trimmed_and_a_blank_one_shows_the_stem() {
        let dir = scratch_dir("trim");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let config = sample_config(&dir);

        let blank = library.create("   ", &config).expect("blank");
        let padded = library.create(" Alpine ", &config).expect("padded");
        let contents = library.load().expect("load");

        assert_eq!(blank.as_str(), "vm");
        assert_eq!(padded.as_str(), "alpine");
        let names: Vec<&str> = contents.vms.iter().map(|vm| vm.name.as_str()).collect();
        assert_eq!(names, ["Alpine", "vm"]);
        let blank_file = fs::read_to_string(library.path_of(&blank)).expect("read");
        assert!(!blank_file.contains("[vm]"));
        remove_dir(&dir);
    }

    #[test]
    fn delete_removes_the_file_and_tolerates_one_already_gone() {
        let dir = scratch_dir("delete");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = library.create("Alpine", &sample_config(&dir)).expect("create");

        library.delete(&stem).expect("delete");
        library.delete(&stem).expect("delete again");

        assert!(toml_names(&dir).is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn delete_file_refuses_a_path_outside_the_folder() {
        let dir = scratch_dir("inside");
        let other = scratch_dir("outside");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let victim = other.join("keep.toml");
        fs::write(&victim, "keep").expect("write");

        let refused = library.delete_file(&victim);

        assert!(matches!(refused, Err(LibraryError::OutsideLibrary { .. })));
        assert!(victim.exists());
        remove_dir(&dir);
        remove_dir(&other);
    }

    #[test]
    fn the_last_selected_vm_is_remembered_while_its_file_exists() {
        let root = scratch_dir("last");
        let dir = root.join("vms");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = library.create("Alpine", &sample_config(&dir)).expect("create");

        library.remember_selected(&stem).expect("remember");
        assert_eq!(library.last_selected(), Some(stem.clone()));

        library.delete(&stem).expect("delete");
        assert_eq!(library.last_selected(), None);

        // A file the record could reach only by leaving the folder.
        fs::write(root.join("escape.toml"), SAMPLE_TOML).expect("write");
        fs::write(dir.join(LAST_SELECTED_FILE), "../escape").expect("write");
        assert_eq!(library.last_selected(), None);
        remove_dir(&root);
    }

    #[test]
    fn a_save_leaves_no_partial_file_behind() {
        let dir = scratch_dir("partial");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = library.create("Alpine", &sample_config(&dir)).expect("create");
        library.remember_selected(&stem).expect("remember");

        assert_eq!(toml_names(&dir), [".last", "alpine.toml"]);
        remove_dir(&dir);
    }

    /// Each process writes its own partial file, so one left by another
    /// process — still writing, or crashed — is neither overwritten nor
    /// removed by a save here.
    #[test]
    fn a_partial_of_another_process_is_left_alone() {
        let dir = scratch_dir("foreign-partial");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = stem("alpine");
        let other_pid = std::process::id().wrapping_add(1);
        let foreign = dir.join(format!("alpine.toml.{other_pid}.partial"));
        fs::write(&foreign, "half-written by another process").expect("write");

        library.save(&stem, "Alpine", &sample_config(&dir)).expect("save");

        assert_eq!(
            fs::read_to_string(&foreign).expect("the other process's partial"),
            "half-written by another process"
        );
        assert_eq!(
            toml_names(&dir),
            ["alpine.toml".to_owned(), format!("alpine.toml.{other_pid}.partial")]
        );
        assert_eq!(library.load().expect("load").vms[0].name, "Alpine");
        remove_dir(&dir);
    }

    #[test]
    fn a_partial_file_is_named_after_its_process() {
        let expected = format!("alpine.toml.{}.partial", std::process::id());
        assert_eq!(
            partial_path(Path::new("alpine.toml")),
            PathBuf::from(expected.clone())
        );
        assert_eq!(
            partial_path(&Path::new("vms").join("alpine.toml")),
            Path::new("vms").join(expected)
        );
    }

    #[test]
    fn a_failed_save_leaves_no_partial_file_behind() {
        let dir = scratch_dir("failed-save");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = stem("alpine");
        fs::create_dir(library.path_of(&stem)).expect("a folder in the file's way");

        let failed = library.save(&stem, "Alpine", &sample_config(&dir));

        assert!(matches!(failed, Err(LibraryError::Write { .. })));
        assert_eq!(toml_names(&dir), ["alpine.toml"]);
        remove_dir(&dir);
    }

    #[test]
    fn open_reports_a_folder_it_cannot_create() {
        let dir = scratch_dir("cannot-create");
        let file = dir.join("file");
        fs::write(&file, "not a folder").expect("write");

        let refused = VmLibrary::open(file.join("vms"));

        assert!(matches!(refused, Err(LibraryError::CreateDir { .. })));
        remove_dir(&dir);
    }

    #[test]
    fn relative_paths_are_saved_absolute_against_the_working_directory() {
        let dir = scratch_dir("relative");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let mut config = sample_config(&dir);
        config.bios = PathBuf::from("roms/bios.bin");
        config.vga_bios = Some(PathBuf::from("roms/vgabios.bin"));
        config.cdrom = config.cdrom.map(|cdrom| crate::config::ResolvedCdrom {
            path: PathBuf::from("boot.iso"),
            ..cdrom
        });

        library.create("Alpine", &config).expect("create");
        let contents = library.load().expect("load");

        let cwd = std::env::current_dir().expect("working directory");
        let saved = &contents.vms[0].config;
        assert_eq!(saved.bios, cwd.join("roms/bios.bin"));
        assert_eq!(saved.vga_bios, Some(cwd.join("roms/vgabios.bin")));
        assert_eq!(
            saved.cdrom.as_ref().map(|cdrom| cdrom.path.as_path()),
            Some(cwd.join("boot.iso").as_path())
        );
        remove_dir(&dir);
    }

    #[test]
    fn the_bundled_vm_resolves_its_paths_beside_the_executable() {
        let library_dir = scratch_dir("bundled-library");
        let exe_dir = scratch_dir("bundled-exe");
        let library = VmLibrary::open(library_dir.clone()).expect("open");
        assert_eq!(library.import_bundled_vm(&exe_dir).expect("nothing to import"), None);
        fs::write(
            exe_dir.join(DEFAULT_CONFIG_FILE),
            "[rom]\nbios = \"roms/BIOS-bochs-latest\"\n\n[cdrom]\npath = \"boot.iso\"\n",
        )
        .expect("write");

        let stem = library.import_bundled_vm(&exe_dir).expect("import");
        let contents = library.load().expect("load");

        assert_eq!(stem.map(|stem| stem.as_str().to_owned()), Some("rusty-box".to_owned()));
        assert_eq!(contents.vms[0].name, DEFAULT_VM_NAME);
        assert_eq!(contents.vms[0].config.bios, exe_dir.join("roms/BIOS-bochs-latest"));
        remove_dir(&library_dir);
        remove_dir(&exe_dir);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_relative_environment_directory_counts_as_unset() {
        let absolute = std::env::temp_dir();
        assert_eq!(absolute_dir(Some("relative/dir".into())), None);
        assert_eq!(absolute_dir(Some(absolute.clone().into_os_string())), Some(absolute));
        assert_eq!(absolute_dir(None), None);
    }
}
