//! The VM library: one TOML file per virtual machine, in one per-user folder.
//!
//! The folder is the only place the shell reads VM configurations from on its
//! own. A file dropped into the working directory, or anywhere else, is never
//! picked up (CWE-427, uncontrolled search path); outside the folder the
//! launcher reads only a file the user names with `--config`. A VM's file name
//! is derived from its name but cut down to a safe alphabet, so no name can
//! climb out of the folder or land on a Windows device name.

use crate::config::{load_toml_file, resolve_config_in, ResolvedConfig, DEFAULT_CONFIG_FILE};
use crate::error::RunError;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The longest file stem `safe_stem` returns, in bytes. A uniqueness suffix
/// (`-2`, `-3`, …) may follow it.
const MAX_STEM_LEN: usize = 64;
/// The file recording which VM the shell showed last. Its leading dot keeps
/// it out of the VM listing.
const LAST_SELECTED_FILE: &str = ".last";
/// The name a VM gets when the file describing it names none.
pub const DEFAULT_VM_NAME: &str = "Rusty Box";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("failed to read the VM library {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("failed to delete {}: {source}", path.display())]
    Delete { path: PathBuf, source: io::Error },
    #[error("{} is not a file of the VM library in {}", path.display(), dir.display())]
    OutsideLibrary { path: PathBuf, dir: PathBuf },
    #[error("failed to serialize VM {name}: {source}")]
    Serialize {
        name: String,
        source: toml::ser::Error,
    },
}

/// A VM file's stem: its file name in the library folder, without `.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VmStem(String);

impl VmStem {
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
    /// The VMs that load, in file-name order.
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
        fs::create_dir_all(&dir).map_err(|source| LibraryError::Write {
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
        self.dir.join(format!("{}.toml", stem.0))
    }

    /// Every `*.toml` file directly in the folder, in file-name order. A file
    /// whose name starts with a dot belongs to the library itself.
    pub fn load(&self) -> Result<LibraryContents, LibraryError> {
        let mut contents = LibraryContents::default();
        let mut files = Vec::new();
        for path in self.toml_files()? {
            match path.file_stem().and_then(|stem| stem.to_str()) {
                Some(stem) if stem.starts_with('.') => {}
                Some(stem) => files.push((stem.to_owned(), path)),
                None => contents.broken.push(BrokenVmFile {
                    error: "the file name is not valid UTF-8".to_owned(),
                    path,
                }),
            }
        }
        files.sort();
        for (stem, path) in files {
            match read_vm(&self.dir, &path) {
                Ok(described) => contents.vms.push(LibraryVm {
                    name: described.name.unwrap_or_else(|| stem.clone()),
                    stem: VmStem(stem),
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

    /// Rewrites the file of `stem` to describe `name` and `config`.
    pub fn save(
        &self,
        stem: &VmStem,
        name: &str,
        config: &ResolvedConfig,
    ) -> Result<(), LibraryError> {
        let mut file = config.to_file_config();
        file.vm.name = Some(name.to_owned());
        let text = toml::to_string_pretty(&file).map_err(|source| LibraryError::Serialize {
            name: name.to_owned(),
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
    /// A record that is missing or unreadable is no record.
    pub fn last_selected(&self) -> Option<VmStem> {
        let recorded = match fs::read_to_string(self.dir.join(LAST_SELECTED_FILE)) {
            Ok(recorded) => recorded,
            Err(_) => return None,
        };
        let stem = VmStem(recorded.trim().to_owned());
        let well_formed = safe_stem(stem.as_str()) == stem.0;
        (well_formed && self.path_of(&stem).is_file()).then_some(stem)
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

    /// The regular `*.toml` files directly in the folder.
    fn toml_files(&self) -> Result<Vec<PathBuf>, LibraryError> {
        let read_error = |source: io::Error| LibraryError::Read {
            path: self.dir.clone(),
            source,
        };
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(read_error)? {
            let path = entry.map_err(read_error)?.path();
            let is_toml = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"));
            if is_toml && path.is_file() {
                files.push(path);
            }
        }
        Ok(files)
    }

    /// A stem for `name` no file in the folder has, compared without case
    /// because Windows and macOS folders ignore case.
    fn unused_stem(&self, name: &str) -> Result<VmStem, LibraryError> {
        let taken: HashSet<String> = self
            .toml_files()?
            .iter()
            .filter_map(|path| path.file_stem().and_then(|stem| stem.to_str()))
            .map(str::to_ascii_lowercase)
            .collect();
        let base = safe_stem(name);
        let mut candidate = base.clone();
        let mut suffix: u64 = 2;
        while taken.contains(&candidate) {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        Ok(VmStem(candidate))
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

/// Writes `bytes` to a sibling of `path` and renames it into place, so `path`
/// is never a partial copy.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let mut partial_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    partial_name.push(".partial");
    let partial = path.with_file_name(partial_name);
    fs::write(&partial, bytes).map_err(|source| LibraryError::Write {
        path: partial.clone(),
        source,
    })?;
    fs::rename(&partial, path).map_err(|source| LibraryError::Write {
        path: path.to_owned(),
        source,
    })
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
    use crate::config::{FileConfig, RomToml};

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

    /// A configuration that resolves: a BIOS path is all a shell VM needs.
    fn sample_config(dir: &Path) -> ResolvedConfig {
        let file = FileConfig {
            rom: RomToml {
                bios: Some(PathBuf::from("bios.bin")),
                vga_bios: None,
            },
            ..FileConfig::default()
        };
        resolve_config_in(file, dir).expect("a BIOS path is all a config needs")
    }

    fn toml_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("list dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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
    fn a_created_vm_is_there_at_the_next_load() {
        let dir = scratch_dir("create");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let config = sample_config(&dir);

        let stem = library.create("Alpine", &config).expect("create");
        let contents = library.load().expect("load");

        assert_eq!(stem.as_str(), "alpine");
        assert_eq!(contents.vms.len(), 1);
        assert_eq!(contents.vms[0].name, "Alpine");
        assert_eq!(contents.vms[0].config, config);
        assert!(contents.broken.is_empty());
        remove_dir(&dir);
    }

    #[test]
    fn a_second_vm_of_the_same_name_gets_its_own_file() {
        let dir = scratch_dir("unique");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("ALPINE.toml"), "[rom]\nbios = \"bios.bin\"\n").expect("write");
        let config = sample_config(&dir);

        let first = library.create("alpine", &config).expect("first");
        let second = library.create("Alpine", &config).expect("second");

        assert_eq!(first.as_str(), "alpine-2");
        assert_eq!(second.as_str(), "alpine-3");
        remove_dir(&dir);
    }

    #[test]
    fn load_orders_by_file_name_and_skips_the_library_s_own_files() {
        let dir = scratch_dir("order");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let config = sample_config(&dir);
        let b = library.create("b", &config).expect("b");
        library.create("a", &config).expect("a");
        library.remember_selected(&b).expect("remember");
        fs::write(dir.join("x.toml.partial"), "junk").expect("partial");
        fs::write(dir.join("notes.txt"), "junk").expect("notes");

        let contents = library.load().expect("load");

        let names: Vec<&str> = contents.vms.iter().map(|vm| vm.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(contents.broken.is_empty());
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
    fn a_file_without_a_vm_table_is_shown_under_its_file_name() {
        let dir = scratch_dir("unnamed");
        let library = VmLibrary::open(dir.clone()).expect("open");
        fs::write(dir.join("dos-lab.toml"), "[rom]\nbios = \"bios.bin\"\n").expect("write");

        let contents = library.load().expect("load");

        assert_eq!(contents.vms[0].name, "dos-lab");
        assert_eq!(contents.vms[0].config.bios, dir.join("bios.bin"));
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
        let dir = scratch_dir("last");
        let library = VmLibrary::open(dir.clone()).expect("open");
        let stem = library.create("Alpine", &sample_config(&dir)).expect("create");

        library.remember_selected(&stem).expect("remember");
        assert_eq!(library.last_selected(), Some(stem.clone()));

        library.delete(&stem).expect("delete");
        assert_eq!(library.last_selected(), None);

        fs::write(dir.join(LAST_SELECTED_FILE), "../escape").expect("write");
        assert_eq!(library.last_selected(), None);
        remove_dir(&dir);
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

    #[test]
    fn the_bundled_vm_resolves_its_paths_beside_the_executable() {
        let library_dir = scratch_dir("bundled-library");
        let exe_dir = scratch_dir("bundled-exe");
        let library = VmLibrary::open(library_dir.clone()).expect("open");
        assert_eq!(library.import_bundled_vm(&exe_dir).expect("nothing to import"), None);
        fs::write(
            exe_dir.join(DEFAULT_CONFIG_FILE),
            "[rom]\nbios = \"roms/BIOS-bochs-latest\"\n",
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
