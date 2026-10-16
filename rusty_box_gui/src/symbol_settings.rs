//! The symbol engine's settings: one `settings.toml` beside the VM library,
//! for every VM (symbol files are keyed by the exact build they describe,
//! so one store serves every guest at once). A key the file leaves unset is
//! taken from the environment other symbol tools read (`_NT_SYMBOL_PATH`,
//! `DEBUGINFOD_URLS`), then from the built-in default, and every resolved
//! value remembers where it came from, for the Settings page to say.

// A hand-edited file is read here: nothing may panic (spec §11).
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::todo,
        clippy::unimplemented,
        clippy::arithmetic_side_effects,
        clippy::string_slice,
        clippy::get_unwrap,
        clippy::panic_in_result_fn
    )
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rusty_box_symbols::projects::ProjectFolders;
use rusty_box_symbols::service::{Downloads, ServiceConfig, DEFAULT_MAX_DOWNLOAD};
use rusty_box_symbols::symbol_path::{parse_debuginfod_urls, parse_nt_symbol_path};

use crate::error::LibraryError;

/// The settings file's name, in the folder that holds the VM library.
pub(crate) const SETTINGS_FILE: &str = "settings.toml";
/// Microsoft's public symbol server.
pub(crate) const MICROSOFT_SYMBOL_SERVER: &str = "https://msdl.microsoft.com/download/symbols";
/// elfutils' federating debuginfod server.
pub(crate) const ELFUTILS_DEBUGINFOD: &str = "https://debuginfod.elfutils.org/";

/// `[symbols]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SymbolsToml {
    /// The symbol store's folder; relative to this file's folder when relative.
    pub(crate) store: Option<PathBuf>,
    pub(crate) download: Option<DownloadToml>,
    pub(crate) microsoft: Option<Vec<String>>,
    pub(crate) debuginfod: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DownloadToml {
    Automatic,
    Off,
}

/// One rewrite of a path a PDB records (the build machine's) to where the
/// sources are here. Read by source-level debugging; kept and edited now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PathMapToml {
    pub(crate) from: PathBuf,
    pub(crate) to: PathBuf,
}

/// `[[project]]`: one of the owner's own builds.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ProjectToml {
    pub(crate) name: String,
    pub(crate) symbols: Vec<PathBuf>,
    pub(crate) sources: Vec<PathBuf>,
    pub(crate) map: Vec<PathMapToml>,
}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsFile {
    pub(crate) symbols: SymbolsToml,
    #[serde(rename = "project", skip_serializing_if = "Vec::is_empty")]
    pub(crate) projects: Vec<ProjectToml>,
}

/// Where a resolved value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    File,
    Environment { variable: &'static str },
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved<T> {
    pub(crate) value: T,
    pub(crate) origin: Origin,
}

/// The environment variables the defaults are read from, read once, so a
/// test can give its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Environment {
    pub(crate) nt_symbol_path: Option<OsString>,
    pub(crate) nt_alt_symbol_path: Option<OsString>,
    pub(crate) debuginfod_urls: Option<OsString>,
    pub(crate) local_app_data: Option<OsString>,
    pub(crate) xdg_cache_home: Option<OsString>,
    pub(crate) home: Option<OsString>,
}

impl Environment {
    pub(crate) fn from_process() -> Self {
        Environment {
            nt_symbol_path: std::env::var_os("_NT_SYMBOL_PATH"),
            nt_alt_symbol_path: std::env::var_os("_NT_ALT_SYMBOL_PATH"),
            debuginfod_urls: std::env::var_os("DEBUGINFOD_URLS"),
            local_app_data: std::env::var_os("LOCALAPPDATA"),
            xdg_cache_home: std::env::var_os("XDG_CACHE_HOME"),
            home: std::env::var_os("HOME"),
        }
    }
}

/// An absolute folder from an environment value. A relative one would
/// resolve against the working directory, where a planted file lives, so it
/// counts as unset (as `library::default_library_dir` reads its own).
fn absolute(value: Option<&OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// The store a fresh install uses: the per-user cache folder (Windows
/// `%LOCALAPPDATA%`, Linux `$XDG_CACHE_HOME` else `~/.cache`, macOS
/// `~/Library/Caches`), and on Android the app's own storage, which holds the
/// settings file (`app_dir`). Without those variables, `app_dir` too.
pub(crate) fn default_store(environment: &Environment, app_dir: &Path) -> PathBuf {
    let cache = if cfg!(target_os = "android") {
        None
    } else if cfg!(windows) {
        absolute(environment.local_app_data.as_ref())
    } else if cfg!(target_os = "macos") {
        absolute(environment.home.as_ref()).map(|home| home.join("Library").join("Caches"))
    } else {
        absolute(environment.xdg_cache_home.as_ref())
            .or_else(|| absolute(environment.home.as_ref()).map(|home| home.join(".cache")))
    };
    match cache {
        Some(cache) => cache.join("rusty_box").join("symbols"),
        None => app_dir.join("symbols"),
    }
}

/// The settings every lookup runs on, each with its origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolSettings {
    pub(crate) store: Resolved<PathBuf>,
    pub(crate) downloads: Resolved<Downloads>,
    pub(crate) microsoft: Resolved<Vec<String>>,
    pub(crate) debuginfod: Resolved<Vec<String>>,
    pub(crate) projects: Vec<ProjectToml>,
}

impl SymbolSettings {
    /// `file`, completed from `environment` and the defaults. `app_dir` is the
    /// folder the settings file is in: a relative path in the file is taken
    /// from there.
    pub(crate) fn resolve(file: &SettingsFile, environment: &Environment, app_dir: &Path) -> SymbolSettings {
        let nt = parse_nt_symbol_path(
            environment.nt_alt_symbol_path.as_ref().and_then(|value| value.to_str()),
            environment.nt_symbol_path.as_ref().and_then(|value| value.to_str()),
        );
        let nt_variable = "_NT_SYMBOL_PATH";
        let store = match &file.symbols.store {
            Some(store) => Resolved {
                value: if store.is_relative() { app_dir.join(store) } else { store.clone() },
                origin: Origin::File,
            },
            None => match nt.store.filter(|store| store.is_absolute()) {
                Some(store) => Resolved {
                    value: store,
                    origin: Origin::Environment { variable: nt_variable },
                },
                None => Resolved {
                    value: default_store(environment, app_dir),
                    origin: Origin::Default,
                },
            },
        };
        let downloads = match file.symbols.download {
            Some(DownloadToml::Automatic) => Resolved {
                value: Downloads::Automatic,
                origin: Origin::File,
            },
            Some(DownloadToml::Off) => Resolved {
                value: Downloads::Off,
                origin: Origin::File,
            },
            None => Resolved {
                value: Downloads::Automatic,
                origin: Origin::Default,
            },
        };
        let microsoft = match &file.symbols.microsoft {
            Some(servers) => Resolved {
                value: servers.clone(),
                origin: Origin::File,
            },
            None if !nt.servers.is_empty() => Resolved {
                value: nt.servers,
                origin: Origin::Environment { variable: nt_variable },
            },
            None => Resolved {
                value: vec![MICROSOFT_SYMBOL_SERVER.to_owned()],
                origin: Origin::Default,
            },
        };
        let from_environment = environment
            .debuginfod_urls
            .as_ref()
            .and_then(|value| value.to_str())
            .map(parse_debuginfod_urls)
            .filter(|servers| !servers.is_empty());
        let debuginfod = match (&file.symbols.debuginfod, from_environment) {
            (Some(servers), _) => Resolved {
                value: servers.clone(),
                origin: Origin::File,
            },
            (None, Some(servers)) => Resolved {
                value: servers,
                origin: Origin::Environment {
                    variable: "DEBUGINFOD_URLS",
                },
            },
            (None, None) => Resolved {
                value: vec![ELFUTILS_DEBUGINFOD.to_owned()],
                origin: Origin::Default,
            },
        };
        SymbolSettings {
            store,
            downloads,
            microsoft,
            debuginfod,
            projects: file.projects.clone(),
        }
    }

    pub(crate) fn service_config(&self) -> ServiceConfig {
        ServiceConfig {
            downloads: self.downloads.value,
            microsoft: self.microsoft.value.clone(),
            debuginfod: self.debuginfod.value.clone(),
            max_download: DEFAULT_MAX_DOWNLOAD,
        }
    }

    pub(crate) fn project_folders(&self) -> Vec<ProjectFolders> {
        self.projects
            .iter()
            .map(|project| ProjectFolders {
                name: project.name.clone(),
                symbols: project.symbols.clone(),
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SettingsError {
    #[error("failed to read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is not valid: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to write the settings: {source}")]
    Serialize {
        #[source]
        source: toml::ser::Error,
    },
    #[error(transparent)]
    Save(#[from] LibraryError),
    #[error("{} did not load, and it could not be kept aside before saving over it: {source}", path.display())]
    SetAside {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// How many `.broken` copies are tried before saving is refused.
const MAX_SET_ASIDE: u32 = 100;

/// Copies the file at `path` to the first free `<name>.broken`,
/// `<name>.broken-2`, … beside it, whole or not at all; `None` when there is
/// no file. An earlier copy is never replaced.
fn set_aside(path: &Path) -> Result<Option<PathBuf>, SettingsError> {
    let failed = |source: std::io::Error| SettingsError::SetAside { path: path.to_owned(), source };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(failed(source)),
    };
    let name = path.file_name().map_or_else(|| std::ffi::OsString::from(SETTINGS_FILE), std::ffi::OsStr::to_os_string);
    for number in 1..=MAX_SET_ASIDE {
        let mut aside_name = name.clone();
        if number == 1 {
            aside_name.push(".broken");
        } else {
            aside_name.push(format!(".broken-{number}"));
        }
        let aside = path.with_file_name(aside_name);
        if aside.try_exists().map_err(failed)? {
            continue;
        }
        crate::library::write_atomically(&aside, &bytes)?;
        return Ok(Some(aside));
    }
    Err(failed(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("{MAX_SET_ASIDE} copies are already kept beside it"),
    )))
}

/// What loading the settings file came to: the file's settings (empty when
/// it is absent or broken), and why it did not load when it is broken. A
/// broken file is never rewritten until the owner saves from the page, and
/// then it is kept aside first.
#[derive(Debug)]
pub(crate) struct LoadedSettings {
    pub(crate) file: SettingsFile,
    pub(crate) problem: Option<SettingsError>,
}

pub(crate) fn load(path: &Path) -> LoadedSettings {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedSettings {
                file: SettingsFile::default(),
                problem: None,
            }
        }
        Err(source) => {
            return LoadedSettings {
                file: SettingsFile::default(),
                problem: Some(SettingsError::Read {
                    path: path.to_owned(),
                    source,
                }),
            }
        }
    };
    match toml::from_str(&text) {
        Ok(file) => LoadedSettings { file, problem: None },
        Err(source) => LoadedSettings {
            file: SettingsFile::default(),
            problem: Some(SettingsError::Parse {
                path: path.to_owned(),
                source,
            }),
        },
    }
}

/// Writes `file` to `path` whole or not at all.
pub(crate) fn save(path: &Path, file: &SettingsFile) -> Result<(), SettingsError> {
    let text = toml::to_string_pretty(file).map_err(|source| SettingsError::Serialize { source })?;
    crate::library::write_atomically(path, text.as_bytes())?;
    Ok(())
}

/// The settings the shell holds: the file it was read from, what was in it,
/// why it did not load when it did not, and the settings resolved from it
/// and the environment.
#[derive(Debug)]
pub(crate) struct SettingsState {
    pub(crate) path: PathBuf,
    pub(crate) file: SettingsFile,
    pub(crate) problem: Option<String>,
    /// Where the file that did not load was kept when the owner saved over
    /// it.
    pub(crate) set_aside: Option<PathBuf>,
    pub(crate) environment: Environment,
    pub(crate) resolved: SymbolSettings,
}

impl SettingsState {
    pub(crate) fn load(path: PathBuf, environment: Environment) -> Self {
        let loaded = load(&path);
        let resolved = SymbolSettings::resolve(&loaded.file, &environment, &app_dir_of(&path));
        SettingsState {
            path,
            file: loaded.file,
            // The TOML parser's message ends with a line break; the page
            // continues the sentence after it.
            problem: loaded.problem.map(|problem| problem.to_string().trim_end().to_owned()),
            set_aside: None,
            environment,
            resolved,
        }
    }

    /// Writes `file` and resolves the settings from it: once saved, it is
    /// what the engine runs on, and a problem the old file had is gone. A
    /// file on disk that did not load is first kept aside, so what the owner
    /// wrote in it survives; when it cannot be, nothing is written.
    pub(crate) fn save(&mut self, file: SettingsFile) -> Result<(), SettingsError> {
        if self.problem.is_some() {
            if let Some(aside) = set_aside(&self.path)? {
                self.set_aside = Some(aside);
            }
        }
        save(&self.path, &file)?;
        self.resolved = SymbolSettings::resolve(&file, &self.environment, &app_dir_of(&self.path));
        self.file = file;
        self.problem = None;
        Ok(())
    }
}

/// The Settings page's edits, as the text its fields hold: a list is one
/// entry a line, a path map `from => to`. An empty field leaves its key out
/// of the file, so the environment or the default applies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SettingsDraft {
    pub(crate) store: String,
    pub(crate) downloads: Option<DownloadToml>,
    pub(crate) microsoft: String,
    pub(crate) debuginfod: String,
    pub(crate) projects: Vec<ProjectDraft>,
}

/// One project's fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProjectDraft {
    pub(crate) name: String,
    pub(crate) symbols: String,
    pub(crate) sources: String,
    pub(crate) map: String,
}

/// Why a draft is not a settings file yet.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DraftError {
    #[error("project {number} has no name")]
    UnnamedProject { number: usize },
    #[error("in {project}'s path maps, \"{line}\" is not `from => to`")]
    BadMap { project: String, line: String },
}

/// The separator between a path map's two sides.
const MAP_ARROW: &str = "=>";

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

fn joined<T: AsRef<std::ffi::OsStr>>(items: &[T]) -> String {
    items.iter().map(|item| Path::new(item.as_ref()).display().to_string()).collect::<Vec<String>>().join("\n")
}

impl SettingsDraft {
    /// The fields for `file`.
    pub(crate) fn of(file: &SettingsFile) -> Self {
        let servers = |list: &Option<Vec<String>>| list.as_ref().map_or_else(String::new, |list| list.join("\n"));
        SettingsDraft {
            store: file.symbols.store.as_ref().map_or_else(String::new, |store| store.display().to_string()),
            downloads: file.symbols.download,
            microsoft: servers(&file.symbols.microsoft),
            debuginfod: servers(&file.symbols.debuginfod),
            projects: file
                .projects
                .iter()
                .map(|project| ProjectDraft {
                    name: project.name.clone(),
                    symbols: joined(&project.symbols),
                    sources: joined(&project.sources),
                    map: project
                        .map
                        .iter()
                        .map(|map| format!("{} {MAP_ARROW} {}", map.from.display(), map.to.display()))
                        .collect::<Vec<String>>()
                        .join("\n"),
                })
                .collect(),
        }
    }

    /// The file these fields describe.
    pub(crate) fn to_file(&self) -> Result<SettingsFile, DraftError> {
        let servers = |text: &str| Some(lines(text)).filter(|list| !list.is_empty());
        let mut projects = Vec::with_capacity(self.projects.len());
        for (index, project) in self.projects.iter().enumerate() {
            let name = project.name.trim();
            if name.is_empty() {
                // The page numbers projects from one.
                return Err(DraftError::UnnamedProject { number: index.saturating_add(1) });
            }
            let mut map = Vec::new();
            for line in lines(&project.map) {
                let Some((from, to)) = line.split_once(MAP_ARROW) else {
                    return Err(DraftError::BadMap { project: name.to_owned(), line });
                };
                let from = from.trim();
                let to = to.trim();
                if from.is_empty() || to.is_empty() {
                    return Err(DraftError::BadMap { project: name.to_owned(), line });
                }
                map.push(PathMapToml { from: PathBuf::from(from), to: PathBuf::from(to) });
            }
            projects.push(ProjectToml {
                name: name.to_owned(),
                symbols: lines(&project.symbols).into_iter().map(PathBuf::from).collect(),
                sources: lines(&project.sources).into_iter().map(PathBuf::from).collect(),
                map,
            });
        }
        Ok(SettingsFile {
            symbols: SymbolsToml {
                store: Some(self.store.trim()).filter(|store| !store.is_empty()).map(PathBuf::from),
                download: self.downloads,
                microsoft: servers(&self.microsoft),
                debuginfod: servers(&self.debuginfod),
            },
            projects,
        })
    }
}

/// The folder the settings file is in, which a relative path in it is taken
/// from.
fn app_dir_of(settings_file: &Path) -> PathBuf {
    settings_file.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

/// The settings file beside the library folder `library_dir`: in its parent,
/// the folder the app keeps its files in.
pub(crate) fn settings_file_beside(library_dir: &Path) -> PathBuf {
    library_dir
        .parent()
        .unwrap_or(library_dir)
        .join(SETTINGS_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SettingsFile {
        toml::from_str(text).expect("settings")
    }

    fn app_dir() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\me\AppData\Roaming\rusty_box")
        } else {
            PathBuf::from("/home/me/.local/share/rusty_box")
        }
    }

    fn absolute_path(windows: &str, unix: &str) -> PathBuf {
        PathBuf::from(if cfg!(windows) { windows } else { unix })
    }

    #[test]
    fn the_file_wins_over_the_environment_and_the_environment_over_the_default() {
        let store = absolute_path(r"D:\Symbols", "/srv/symbols");
        let environment = Environment {
            nt_symbol_path: Some(OsString::from(format!(
                "srv*{}*https://symbols.example/",
                store.display()
            ))),
            debuginfod_urls: Some(OsString::from("https://debuginfod.example/")),
            ..Environment::default()
        };
        let none = SymbolSettings::resolve(&SettingsFile::default(), &environment, &app_dir());
        assert_eq!(none.store, Resolved { value: store, origin: Origin::Environment { variable: "_NT_SYMBOL_PATH" } });
        assert_eq!(none.microsoft.value, ["https://symbols.example/"]);
        assert_eq!(none.debuginfod.origin, Origin::Environment { variable: "DEBUGINFOD_URLS" });
        assert_eq!(none.downloads, Resolved { value: Downloads::Automatic, origin: Origin::Default });

        let set = file(
            "[symbols]\nstore = 'cache'\ndownload = \"off\"\nmicrosoft = []\ndebuginfod = [\"https://mine.example/\"]\n",
        );
        let resolved = SymbolSettings::resolve(&set, &environment, &app_dir());
        assert_eq!(resolved.store, Resolved { value: app_dir().join("cache"), origin: Origin::File });
        assert_eq!(resolved.downloads, Resolved { value: Downloads::Off, origin: Origin::File });
        assert!(resolved.microsoft.value.is_empty(), "an empty list in the file is a choice");
        assert_eq!(resolved.debuginfod.value, ["https://mine.example/"]);
    }

    #[test]
    fn with_nothing_set_the_defaults_hold() {
        let resolved = SymbolSettings::resolve(&SettingsFile::default(), &Environment::default(), &app_dir());
        assert_eq!(resolved.microsoft, Resolved { value: vec![MICROSOFT_SYMBOL_SERVER.to_owned()], origin: Origin::Default });
        assert_eq!(resolved.debuginfod.value, [ELFUTILS_DEBUGINFOD]);
        assert_eq!(resolved.store, Resolved { value: app_dir().join("symbols"), origin: Origin::Default });
    }

    /// A relative variable would name a folder under the working directory.
    #[test]
    fn a_relative_store_in_the_environment_is_not_used() {
        let environment = Environment {
            nt_symbol_path: Some(OsString::from("srv*relative*https://symbols.example/")),
            ..Environment::default()
        };
        let resolved = SymbolSettings::resolve(&SettingsFile::default(), &environment, &app_dir());
        assert_eq!(resolved.store.origin, Origin::Default);
    }

    #[test]
    fn projects_are_kept_with_their_sources_and_path_maps() {
        let set = file(
            "[[project]]\nname = \"mydriver\"\nsymbols = ['build']\nsources = ['src']\nmap = [{ from = 'C:\\\\agent', to = 'D:\\\\work' }]\n",
        );
        assert_eq!(set.projects[0].map[0].from, PathBuf::from(r"C:\agent"));
        let resolved = SymbolSettings::resolve(&set, &Environment::default(), &app_dir());
        assert_eq!(
            resolved.project_folders(),
            [ProjectFolders { name: "mydriver".to_owned(), symbols: vec![PathBuf::from("build")] }]
        );
    }

    /// Review focus 5: a file that does not parse says why and is left as
    /// it is.
    #[test]
    fn a_broken_file_reports_why_and_is_not_rewritten() {
        let dir = std::env::temp_dir().join(format!("rusty-box-gui-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "[symbols]\nstore = 3\n").expect("write");
        let loaded = load(&path);
        assert_eq!(loaded.file, SettingsFile::default());
        assert!(matches!(loaded.problem, Some(SettingsError::Parse { .. })), "{:?}", loaded.problem);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "[symbols]\nstore = 3\n");
        let missing = load(&dir.join("absent.toml"));
        assert!(missing.problem.is_none());
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn saved_settings_load_back_the_same() {
        let dir = std::env::temp_dir().join(format!("rusty-box-gui-settings-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(SETTINGS_FILE);
        let settings = file("[symbols]\ndownload = \"off\"\n[[project]]\nname = \"p\"\nsymbols = ['out']\n");
        save(&path, &settings).expect("save");
        assert_eq!(load(&path).file, settings);
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn the_settings_file_sits_beside_the_library_folder() {
        let library = app_dir().join("vms");
        assert_eq!(settings_file_beside(&library), app_dir().join(SETTINGS_FILE));
    }

    /// The page's fields hold what the file says, and give the same file
    /// back; an empty field leaves its key out.
    #[test]
    fn a_draft_gives_back_the_file_it_was_made_from() {
        // TOML literal strings: each backslash is one in the path.
        let settings = file(
            "[symbols]\nstore = 'D:\\Symbols'\ndownload = \"off\"\nmicrosoft = ['https://a.example/', 'https://b.example/']\n\
             [[project]]\nname = \"mydriver\"\nsymbols = ['build', 'out']\nsources = ['src']\n\
             map = [{ from = 'C:\\agent', to = 'D:\\work' }]\n",
        );
        let draft = SettingsDraft::of(&settings);
        assert_eq!(draft.microsoft, "https://a.example/\nhttps://b.example/");
        assert_eq!(draft.projects[0].map, "C:\\agent => D:\\work");
        assert_eq!(draft.to_file(), Ok(settings));
        let empty = SettingsDraft::of(&SettingsFile::default());
        assert_eq!(empty.to_file(), Ok(SettingsFile::default()));
    }

    #[test]
    fn a_draft_with_a_bad_map_or_an_unnamed_project_says_which() {
        let mut draft = SettingsDraft::default();
        draft.projects.push(ProjectDraft { name: "p".to_owned(), map: "C:\\agent -> D:\\work".to_owned(), ..ProjectDraft::default() });
        assert_eq!(
            draft.to_file(),
            Err(DraftError::BadMap { project: "p".to_owned(), line: "C:\\agent -> D:\\work".to_owned() })
        );
        draft.projects[0].name.clear();
        assert_eq!(draft.to_file(), Err(DraftError::UnnamedProject { number: 1 }));
    }

    /// Review focus 5: saving rewrites the file, resolves from it, and
    /// clears the problem the old file had.
    #[test]
    fn saving_resolves_from_the_saved_file() {
        let dir = std::env::temp_dir().join(format!("rusty-box-gui-settings-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "[symbols]\nstore = 3\n").expect("write");
        let mut state = SettingsState::load(path.clone(), Environment::default());
        assert!(state.problem.is_some());
        state.save(file("[symbols]\ndownload = \"off\"\n")).expect("save");
        assert_eq!(state.problem, None);
        assert_eq!(state.resolved.downloads, Resolved { value: Downloads::Off, origin: Origin::File });
        assert_eq!(load(&path).file, state.file);
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    /// Review focus 5: what the owner wrote in a file that did not load
    /// survives saving from the page, beside it.
    #[test]
    fn saving_over_a_broken_file_keeps_it_aside() {
        let dir = std::env::temp_dir().join(format!("rusty-box-gui-settings-aside-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "[symbols]\nstore = 3\n").expect("write");
        let mut state = SettingsState::load(path.clone(), Environment::default());
        state.save(file("[symbols]\ndownload = \"off\"\n")).expect("save");
        let aside = dir.join(format!("{SETTINGS_FILE}.broken"));
        assert_eq!(state.set_aside.as_deref(), Some(aside.as_path()));
        assert_eq!(std::fs::read_to_string(&aside).expect("kept"), "[symbols]\nstore = 3\n");
        // The file loads now: saving again keeps nothing more aside.
        state.save(file("[symbols]\ndownload = \"automatic\"\n")).expect("save again");
        assert_eq!(std::fs::read_to_string(&aside).expect("still kept"), "[symbols]\nstore = 3\n");
        assert!(!dir.join(format!("{SETTINGS_FILE}.broken-2")).exists());
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    /// The page writes "<problem>. The engine runs on …": a problem ending
    /// in a line break would start that sentence on a line of its own.
    #[test]
    fn a_broken_files_problem_ends_where_its_text_does() {
        let dir = std::env::temp_dir().join(format!("rusty-box-gui-settings-problem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "[symbols]\nstore = 3\n").expect("write");
        let state = SettingsState::load(path, Environment::default());
        let problem = state.problem.expect("the file does not parse");
        assert_eq!(problem.trim_end(), problem, "{problem:?}");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn a_second_broken_file_does_not_replace_the_first_kept_aside() {
        let dir = std::env::temp_dir().join(format!("rusty-box-gui-settings-aside2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(dir.join(format!("{SETTINGS_FILE}.broken")), "first").expect("write");
        std::fs::write(&path, "second = [").expect("write");
        let mut state = SettingsState::load(path.clone(), Environment::default());
        state.save(SettingsFile::default()).expect("save");
        let second = dir.join(format!("{SETTINGS_FILE}.broken-2"));
        assert_eq!(state.set_aside.as_deref(), Some(second.as_path()));
        assert_eq!(std::fs::read_to_string(dir.join(format!("{SETTINGS_FILE}.broken"))).expect("first"), "first");
        assert_eq!(std::fs::read_to_string(&second).expect("second"), "second = [");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
