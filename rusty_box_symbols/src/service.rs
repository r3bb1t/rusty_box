//! Finding a module's debug information: the use case, over the store and
//! the network ports. A module is looked up, in order, in the owner's
//! project folders, the store, then the servers (when downloads are on),
//! and is taken only from a file whose declared identity is the module's.
//! A download is verified before it enters the store; a store file that
//! does not load is fetched again, and moved aside as `.bad` only once that
//! download verifies. An ELF debug file that
//! names a dwz supplement gets it the same way, by build-id; without it,
//! the file's own names still load.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cab::{extract_single, is_cabinet, CabError};
use crate::elf::AltLink;
use crate::http::{classify, Flow, HttpClient, HttpError, Progress, Retry, StatusClass};
use crate::identity::{BuildId, IdentityMatch, ModuleIdentity, PdbId, PeImageId, TablesIdentity};
use crate::pe::{pdb_from_binary, BinaryPdbError};
use crate::projects::ProjectIndex;
use crate::store::{checked_file_name, FileStore, Placed, StoreError, SymbolStore};
use crate::tables::{declared_identity, supplement_of, ParseError, SymbolTables};

/// The most a download may be: a cabinet's largest file.
pub const DEFAULT_MAX_DOWNLOAD: u64 = 0x7FFF_8000;

/// Whether the engine may fetch from the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Downloads {
    Automatic,
    Off,
}

/// What the service is set to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceConfig {
    pub downloads: Downloads,
    /// Symbol servers for PE images and their PDBs, in the order tried.
    pub microsoft: Vec<String>,
    /// debuginfod servers for ELF debug files, in the order tried.
    pub debuginfod: Vec<String>,
    pub max_download: u64,
}

/// Which kind of server a module is fetched from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerKind {
    Microsoft,
    Debuginfod,
}

/// Whether a downloaded file was kept in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    InStore { path: PathBuf },
    /// Kept at `path` in place of a store file that did not load, which was
    /// moved to `bad` once this download verified.
    Replaced { path: PathBuf, bad: PathBuf },
    /// The tables loaded, but writing them to the store failed.
    NotKept { reason: String },
}

/// Where loaded tables came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolSource {
    Attached { path: PathBuf },
    Project { name: String, path: PathBuf },
    Store { path: PathBuf },
    /// The URL asked for (never a redirect's signed URL).
    Server { url: String, kept: Kept },
}

/// Whether an attached file of another build is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// Only a file of the module's own build.
    MatchOnly,
    /// The owner's "Load anyway": any file that parses.
    LoadAnyway,
}

#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error("downloads are off, and no project folder or store has this build")]
    DownloadsOff,
    #[error("no {kind:?} server is configured")]
    NoServer { kind: ServerKind },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("{url} answered {status}")]
    Status { url: String, status: u16, retry: Retry },
    #[error("{origin} does not parse: {source}")]
    Parse {
        origin: String,
        #[source]
        source: ParseError,
    },
    #[error("{url} is a cabinet that does not open: {source}")]
    Cabinet {
        url: String,
        #[source]
        source: CabError,
    },
    #[error("{origin} describes {found}, not this module's build")]
    WrongBuild { origin: String, found: TablesIdentity },
    #[error("the module's binary does not give its PDB: {0}")]
    Binary(BinaryPdbError),
}

impl LocateError {
    /// Whether a Retry may succeed: the network and the disk may recover;
    /// a wrong or broken file stays wrong.
    #[must_use]
    pub fn retry(&self) -> Retry {
        match self {
            LocateError::Store(_) => Retry::Retryable,
            LocateError::Http(error) => error.retry(),
            LocateError::Status { retry, .. } => *retry,
            LocateError::DownloadsOff
            | LocateError::NoServer { .. }
            | LocateError::Parse { .. }
            | LocateError::Cabinet { .. }
            | LocateError::WrongBuild { .. }
            | LocateError::Binary(_) => Retry::Final,
        }
    }
}

/// What a lookup came to.
pub enum LocateOutcome {
    Loaded { tables: Arc<SymbolTables>, from: SymbolSource },
    /// An attached file of another build, refused; `Consent::LoadAnyway`
    /// loads it.
    Refused { found: TablesIdentity, wanted: ModuleIdentity, path: PathBuf },
    /// An attached file of another build, loaded on the owner's word.
    Forced { tables: Arc<SymbolTables>, found: TablesIdentity, path: PathBuf },
    /// Every server answered that it does not have the file.
    NotOnServer,
    Unavailable { reason: LocateError },
}

/// Where a module's file lives: its store path, and the URLs to try, by
/// server (a symbol server is asked for the plain name, then the
/// compressed one).
struct Keyed {
    store_path: PathBuf,
    kind: ServerKind,
    servers: Vec<Vec<String>>,
}

/// What the binary-first path found.
enum BinaryLookup {
    Found(PdbId),
    NotOnServer,
}

/// What the store held at a module's path before a download.
enum StoreCopy {
    Absent,
    /// A file this reader could not use, and why. It stays until a download
    /// of the build verifies: the store may be a debugger's too, and a file
    /// this reader rejects may serve it.
    Unusable { reason: LocateError },
}

impl StoreCopy {
    /// Why the lookup ends when nothing can be downloaded: the store copy's
    /// own fault when there is one.
    fn without_downloads(self) -> LocateError {
        match self {
            StoreCopy::Absent => LocateError::DownloadsOff,
            StoreCopy::Unusable { reason } => reason,
        }
    }
}

/// A file a server had.
struct Fetched {
    url: String,
    bytes: Vec<u8>,
}

/// `name` with its last character made `_`: the compressed name a symbol
/// server may hold (`ntkrnlmp.pd_`).
fn compressed_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next_back() {
        Some(_) => format!("{}_", chars.as_str()),
        None => String::from("_"),
    }
}

fn keyed_urls(servers: &[String], file: &str, key: &str) -> Vec<Vec<String>> {
    servers
        .iter()
        .map(|server| {
            let server = server.trim_end_matches('/');
            vec![format!("{server}/{file}/{key}/{file}"), format!("{server}/{file}/{key}/{}", compressed_name(file))]
        })
        .collect()
}

fn debuginfod_urls(servers: &[String], build_id: &BuildId) -> Vec<Vec<String>> {
    servers
        .iter()
        .map(|server| vec![format!("{}/buildid/{}/debuginfo", server.trim_end_matches('/'), build_id.to_hex())])
        .collect()
}

pub struct SymbolService<F: FileStore, H: HttpClient> {
    store: SymbolStore<F>,
    http: H,
    config: ServiceConfig,
}

impl<F: FileStore, H: HttpClient> SymbolService<F, H> {
    pub fn new(store: SymbolStore<F>, http: H, config: ServiceConfig) -> Self {
        SymbolService { store, http, config }
    }

    pub fn store(&mut self) -> &mut SymbolStore<F> {
        &mut self.store
    }

    #[must_use]
    pub fn http(&self) -> &H {
        &self.http
    }

    #[must_use]
    pub fn config(&self) -> &ServiceConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: ServiceConfig) {
        self.config = config;
    }

    /// Finds `wanted`'s tables. A PE module whose PDB identity did not read
    /// in the guest is first identified through its own binary, fetched by
    /// TimeDateStamp and SizeOfImage.
    pub fn locate<P>(&mut self, wanted: &ModuleIdentity, projects: &ProjectIndex, mut progress: P) -> LocateOutcome
    where
        P: FnMut(Progress) -> Flow,
    {
        let wanted = match wanted {
            ModuleIdentity::Pe { file_name, image, pdb: None } => match self.pdb_through_binary(file_name, *image, &mut progress) {
                Ok(BinaryLookup::Found(pdb)) => wanted.with_pdb(pdb),
                Ok(BinaryLookup::NotOnServer) => return LocateOutcome::NotOnServer,
                Err(reason) => return LocateOutcome::Unavailable { reason },
            },
            ModuleIdentity::Pe { pdb: Some(_), .. } | ModuleIdentity::Elf { .. } => wanted.clone(),
        };
        if let Some(found) = projects.find(&wanted) {
            // A file changed since it was indexed is passed by.
            if let Ok(tables) = self.parse_local(&found.path, projects, &mut progress) {
                if wanted.matches(&tables.identity()) == IdentityMatch::Same {
                    return LocateOutcome::Loaded {
                        tables: Arc::new(tables),
                        from: SymbolSource::Project { name: found.project.clone(), path: found.path.clone() },
                    };
                }
            }
        }
        let keyed = match self.keyed(&wanted) {
            Ok(keyed) => keyed,
            Err(reason) => return LocateOutcome::Unavailable { reason },
        };
        // A half-written or corrupt file, or one filed under the wrong key,
        // is fetched again below and replaced only by a verified download.
        let store_copy = match self.store.lookup(&keyed.store_path) {
            Ok(Some(bytes)) => {
                let origin = keyed.store_path.display().to_string();
                match self.parse(&bytes, projects, &mut progress) {
                    Ok(tables) if wanted.matches(&tables.identity()) == IdentityMatch::Same => {
                        return LocateOutcome::Loaded { tables: Arc::new(tables), from: SymbolSource::Store { path: keyed.store_path } };
                    }
                    Ok(tables) => StoreCopy::Unusable { reason: LocateError::WrongBuild { origin, found: tables.identity() } },
                    Err(source) => StoreCopy::Unusable { reason: LocateError::Parse { origin, source } },
                }
            }
            Ok(None) => StoreCopy::Absent,
            Err(error) => return LocateOutcome::Unavailable { reason: error.into() },
        };
        match self.config.downloads {
            Downloads::Off => LocateOutcome::Unavailable { reason: store_copy.without_downloads() },
            Downloads::Automatic => self.download(&wanted, keyed, store_copy, projects, &mut progress),
        }
    }

    /// Files a verified download at `path`. A store copy that did not load
    /// is moved aside as `.bad` first, and the outcome says where.
    fn keep(&mut self, path: PathBuf, bytes: &[u8], store_copy: StoreCopy) -> Kept {
        let bad = match store_copy {
            StoreCopy::Absent => None,
            StoreCopy::Unusable { .. } => match self.store.quarantine(&path) {
                Ok(bad) => Some(bad),
                Err(error) => {
                    return Kept::NotKept { reason: format!("the store's copy that did not load could not be moved aside: {error}") }
                }
            },
        };
        match (self.store.place(&path, bytes), bad) {
            // Another writer filing the same build first leaves the same
            // build in the store.
            (Ok(Placed::Written | Placed::AlreadyPresent), None) => Kept::InStore { path },
            (Ok(Placed::Written | Placed::AlreadyPresent), Some(bad)) => Kept::Replaced { path, bad },
            (Err(error), None) => Kept::NotKept { reason: error.to_string() },
            (Err(error), Some(bad)) => {
                Kept::NotKept { reason: format!("{error}; the copy that did not load is kept as {}", bad.display()) }
            }
        }
    }

    /// Loads a file the owner picked for `wanted`: taken when it is the
    /// module's build, else refused, or loaded as forced on `LoadAnyway`.
    pub fn attach(&mut self, path: &Path, wanted: &ModuleIdentity, consent: Consent) -> LocateOutcome {
        let tables = match self.parse_local(path, &ProjectIndex::default(), &mut |_| Flow::Continue) {
            Ok(tables) => tables,
            Err(reason) => return LocateOutcome::Unavailable { reason },
        };
        let found = tables.identity();
        match (wanted.matches(&found), consent) {
            (IdentityMatch::Same, Consent::MatchOnly | Consent::LoadAnyway) => {
                LocateOutcome::Loaded { tables: Arc::new(tables), from: SymbolSource::Attached { path: path.to_owned() } }
            }
            (IdentityMatch::Different | IdentityMatch::Unknown, Consent::MatchOnly) => {
                LocateOutcome::Refused { found, wanted: wanted.clone(), path: path.to_owned() }
            }
            (IdentityMatch::Different | IdentityMatch::Unknown, Consent::LoadAnyway) => {
                LocateOutcome::Forced { tables: Arc::new(tables), found, path: path.to_owned() }
            }
        }
    }

    fn parse_local<P>(&mut self, path: &Path, projects: &ProjectIndex, progress: &mut P) -> Result<SymbolTables, LocateError>
    where
        P: FnMut(Progress) -> Flow,
    {
        let bytes = self.store.files().read(path)?;
        self.parse(&bytes, projects, progress)
            .map_err(|source| LocateError::Parse { origin: path.display().to_string(), source })
    }

    /// The tables of `bytes`, with their dwz supplement when they name one
    /// and it can be had.
    fn parse<P>(&mut self, bytes: &[u8], projects: &ProjectIndex, progress: &mut P) -> Result<SymbolTables, ParseError>
    where
        P: FnMut(Progress) -> Flow,
    {
        let supplement = match supplement_of(bytes)? {
            Some(link) => self.supplement(&link, projects, progress),
            None => None,
        };
        SymbolTables::parse_with_supplement(bytes, supplement.as_deref())
    }

    /// A dwz supplement by build-id: a project's, the store's, else
    /// debuginfod's (verified, then kept). Anything that does not give it
    /// gives `None`; the file it serves loads without it.
    fn supplement<P>(&mut self, link: &AltLink, projects: &ProjectIndex, progress: &mut P) -> Option<Vec<u8>>
    where
        P: FnMut(Progress) -> Flow,
    {
        let wanted = TablesIdentity::Elf { build_id: Some(link.build_id.clone()) };
        let is_it = |bytes: &[u8]| declared_identity(bytes).is_ok_and(|found| found == wanted);
        if let Some(found) = projects.find_build_id(&link.build_id) {
            if let Ok(bytes) = self.store.files().read(&found.path) {
                if is_it(&bytes) {
                    return Some(bytes);
                }
            }
        }
        let store_path = self.store.debug_file_path(&link.build_id);
        if let Ok(Some(bytes)) = self.store.lookup(&store_path) {
            if is_it(&bytes) {
                return Some(bytes);
            }
        }
        match self.config.downloads {
            Downloads::Off => return None,
            Downloads::Automatic => {}
        }
        let keyed = Keyed {
            store_path,
            kind: ServerKind::Debuginfod,
            servers: debuginfod_urls(&self.config.debuginfod, &link.build_id),
        };
        let fetched = self.fetch(&keyed, progress).ok()??;
        if !is_it(&fetched.bytes) {
            return None;
        }
        match self.store.place(&keyed.store_path, &fetched.bytes) {
            // Kept or not, the supplement serves this lookup.
            Ok(Placed::Written | Placed::AlreadyPresent) | Err(_) => Some(fetched.bytes),
        }
    }

    fn keyed(&self, wanted: &ModuleIdentity) -> Result<Keyed, LocateError> {
        match wanted {
            ModuleIdentity::Pe { pdb: Some(pdb), .. } => Ok(Keyed {
                store_path: self.store.pdb_path(pdb)?,
                kind: ServerKind::Microsoft,
                servers: keyed_urls(&self.config.microsoft, &pdb.name, &pdb.store_key()),
            }),
            // `locate` identifies a PE module through its binary first.
            ModuleIdentity::Pe { file_name, image, pdb: None } => Ok(Keyed {
                store_path: self.store.binary_path(file_name, *image)?,
                kind: ServerKind::Microsoft,
                servers: keyed_urls(&self.config.microsoft, checked_file_name(file_name)?, &image.store_key()),
            }),
            ModuleIdentity::Elf { build_id, .. } => Ok(Keyed {
                store_path: self.store.debug_file_path(build_id),
                kind: ServerKind::Debuginfod,
                servers: debuginfod_urls(&self.config.debuginfod, build_id),
            }),
        }
    }

    /// The body of the first server that has `keyed`'s file, as fetched
    /// (cabinets opened); `None` when every server answered 404.
    fn fetch<P>(&mut self, keyed: &Keyed, progress: &mut P) -> Result<Option<Fetched>, LocateError>
    where
        P: FnMut(Progress) -> Flow,
    {
        if keyed.servers.is_empty() {
            return Err(LocateError::NoServer { kind: keyed.kind });
        }
        let mut failure: Option<LocateError> = None;
        for urls in &keyed.servers {
            for url in urls {
                let response = match self.http.get(url, self.config.max_download, &mut *progress) {
                    Ok(response) => response,
                    Err(error @ HttpError::Cancelled { .. }) => return Err(error.into()),
                    Err(error) => {
                        failure = Some(error.into());
                        break;
                    }
                };
                match classify(response.status) {
                    StatusClass::Ok => {
                        let bytes = if is_cabinet(&response.body) {
                            extract_single(&response.body, self.config.max_download)
                                .map_err(|source| LocateError::Cabinet { url: url.clone(), source })?
                                .bytes
                        } else {
                            response.body
                        };
                        return Ok(Some(Fetched { url: url.clone(), bytes }));
                    }
                    // The plain name missing: its compressed name next.
                    StatusClass::NotFound => continue,
                    StatusClass::Retryable => {
                        failure = Some(LocateError::Status { url: url.clone(), status: response.status, retry: Retry::Retryable });
                        break;
                    }
                    StatusClass::Final => {
                        failure = Some(LocateError::Status { url: url.clone(), status: response.status, retry: Retry::Final });
                        break;
                    }
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    fn download<P>(
        &mut self,
        wanted: &ModuleIdentity,
        keyed: Keyed,
        store_copy: StoreCopy,
        projects: &ProjectIndex,
        progress: &mut P,
    ) -> LocateOutcome
    where
        P: FnMut(Progress) -> Flow,
    {
        let fetched = match self.fetch(&keyed, progress) {
            Ok(Some(fetched)) => fetched,
            Ok(None) => return LocateOutcome::NotOnServer,
            Err(reason) => return LocateOutcome::Unavailable { reason },
        };
        let tables = match self.parse(&fetched.bytes, projects, progress) {
            Ok(tables) => tables,
            Err(source) => return LocateOutcome::Unavailable { reason: LocateError::Parse { origin: fetched.url, source } },
        };
        let found = tables.identity();
        if wanted.matches(&found) != IdentityMatch::Same {
            return LocateOutcome::Unavailable { reason: LocateError::WrongBuild { origin: fetched.url, found } };
        }
        let kept = self.keep(keyed.store_path, &fetched.bytes, store_copy);
        LocateOutcome::Loaded { tables: Arc::new(tables), from: SymbolSource::Server { url: fetched.url, kept } }
    }

    /// The PDB identity of the PE module whose image key is `image`, read
    /// from its binary: the store's copy, else the server's (verified to be
    /// that image before it is stored).
    fn pdb_through_binary<P>(&mut self, file_name: &str, image: PeImageId, progress: &mut P) -> Result<BinaryLookup, LocateError>
    where
        P: FnMut(Progress) -> Flow,
    {
        let binary = ModuleIdentity::Pe { file_name: file_name.to_owned(), image, pdb: None };
        let keyed = self.keyed(&binary)?;
        // Not this image, or broken: fetched again, and replaced only by a
        // verified download.
        let store_copy = match self.store.lookup(&keyed.store_path)? {
            Some(bytes) => match pdb_from_binary(bytes, image) {
                Ok(pdb) => return Ok(BinaryLookup::Found(pdb)),
                Err(error) => StoreCopy::Unusable { reason: LocateError::Binary(error) },
            },
            None => StoreCopy::Absent,
        };
        match self.config.downloads {
            Downloads::Off => return Err(store_copy.without_downloads()),
            Downloads::Automatic => {}
        }
        let Some(fetched) = self.fetch(&keyed, progress)? else {
            return Ok(BinaryLookup::NotOnServer);
        };
        let pdb = pdb_from_binary(fetched.bytes.clone(), image).map_err(LocateError::Binary)?;
        match self.keep(keyed.store_path, &fetched.bytes, store_copy) {
            // The binary only names the PDB: kept or not, it served this
            // lookup, as a supplement does.
            Kept::InStore { .. } | Kept::Replaced { .. } | Kept::NotKept { .. } => Ok(BinaryLookup::Found(pdb)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{BuildId, Guid};
    use crate::memory_store::MemoryStore;
    use crate::projects::ProjectFolders;
    use crate::scripted_http::ScriptedHttp;
    use crate::units::Rva;
    use rusty_box_symbols_fixtures::cab::{cabinet, CabinetKind};
    use rusty_box_symbols_fixtures::dwarf::{dwz_pair, DwzForms, SUPPLEMENT_BUILD_ID};
    use rusty_box_symbols_fixtures::elf::{build_elf, ElfSpec, Symtab, BUILD_ID, TEXT_RVA};
    use rusty_box_symbols_fixtures::pdb::{build_pdb, FixturePublic, PdbFixture, PublicKind};
    use rusty_box_symbols_fixtures::pe::{pe32_file, rsds};

    const MSDL: &str = "https://msdl.example/download/symbols";
    const DEBUGINFOD: &str = "https://debuginfod.example";

    /// The Windows 11 kernel's RSDS GUID bytes (see `identity`'s tests).
    const GUID_BYTES: [u8; 16] = [
        0xFE, 0xC6, 0x61, 0xB6, 0x99, 0x72, 0x16, 0x1D, 0x0F, 0xE3, 0x7C, 0x48, 0xB8, 0xBC, 0x54, 0x70,
    ];

    fn guid() -> Guid {
        Guid::from_le_bytes(GUID_BYTES)
    }

    fn kernel() -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: "ntoskrnl.exe".to_owned(),
            image: PeImageId { time_date_stamp: 0x0BAD_F00D, size_of_image: 0x3000 },
            pdb: Some(PdbId { name: "ntkrnlmp.pdb".to_owned(), guid: guid(), age: 1 }),
        }
    }

    /// A PDB of `guid` and `age` whose one public names RVA 0x1010.
    fn pdb(guid: [u8; 16], age: u32) -> Vec<u8> {
        let mut fixture = PdbFixture::new(guid, age);
        fixture.publics.push(FixturePublic { name: "PsActiveProcessHead".into(), section: 1, offset: 0x10, kind: PublicKind::Data });
        build_pdb(&fixture)
    }

    fn kernel_url() -> String {
        format!("{MSDL}/ntkrnlmp.pdb/B661C6FE72991D160FE37C48B8BC54701/ntkrnlmp.pdb")
    }

    fn config(downloads: Downloads) -> ServiceConfig {
        ServiceConfig {
            downloads,
            microsoft: vec![MSDL.to_owned()],
            debuginfod: vec![DEBUGINFOD.to_owned()],
            max_download: DEFAULT_MAX_DOWNLOAD,
        }
    }

    fn service(files: MemoryStore, http: ScriptedHttp, downloads: Downloads) -> SymbolService<MemoryStore, ScriptedHttp> {
        let store = SymbolStore::open(files, PathBuf::from("store")).unwrap();
        SymbolService::new(store, http, config(downloads))
    }

    fn store_path() -> PathBuf {
        PathBuf::from("store").join("ntkrnlmp.pdb").join("B661C6FE72991D160FE37C48B8BC54701").join("ntkrnlmp.pdb")
    }

    fn go(service: &mut SymbolService<MemoryStore, ScriptedHttp>, wanted: &ModuleIdentity) -> LocateOutcome {
        service.locate(wanted, &ProjectIndex::default(), |_| Flow::Continue)
    }

    fn loaded_from(outcome: &LocateOutcome) -> Option<&SymbolSource> {
        match outcome {
            LocateOutcome::Loaded { from, .. } => Some(from),
            LocateOutcome::Refused { .. } | LocateOutcome::Forced { .. } | LocateOutcome::NotOnServer | LocateOutcome::Unavailable { .. } => None,
        }
    }

    fn unavailable(outcome: &LocateOutcome) -> Option<&LocateError> {
        match outcome {
            LocateOutcome::Unavailable { reason } => Some(reason),
            LocateOutcome::Loaded { .. } | LocateOutcome::Refused { .. } | LocateOutcome::Forced { .. } | LocateOutcome::NotOnServer => None,
        }
    }

    fn loaded_tables(outcome: LocateOutcome) -> Arc<SymbolTables> {
        match outcome {
            LocateOutcome::Loaded { tables, .. } => tables,
            LocateOutcome::Refused { .. } | LocateOutcome::Forced { .. } | LocateOutcome::NotOnServer | LocateOutcome::Unavailable { .. } => {
                panic!("expected loaded tables")
            }
        }
    }

    #[test]
    fn a_project_file_comes_before_the_store_and_the_store_before_the_server() {
        let mut files = MemoryStore::default();
        let build = PathBuf::from("work").join("build").join("ntkrnlmp.pdb");
        files.insert(build.clone(), pdb(GUID_BYTES, 1));
        let projects = [ProjectFolders { name: "kernel".into(), symbols: vec![PathBuf::from("work")] }];
        let index = ProjectIndex::build(&mut files, &projects).unwrap();
        files.insert(store_path(), pdb(GUID_BYTES, 1));
        let mut http = ScriptedHttp::default();
        http.respond(&kernel_url(), 200, pdb(GUID_BYTES, 1));
        let mut service = service(files, http, Downloads::Automatic);
        let outcome = service.locate(&kernel(), &index, |_| Flow::Continue);
        assert_eq!(loaded_from(&outcome), Some(&SymbolSource::Project { name: "kernel".into(), path: build }));
        let outcome = go(&mut service, &kernel());
        assert_eq!(loaded_from(&outcome), Some(&SymbolSource::Store { path: store_path() }));
        assert!(service.http().requests().is_empty(), "the server is never asked for a local file");
    }

    #[test]
    fn a_download_is_kept_in_the_store_and_named() {
        let mut http = ScriptedHttp::default();
        http.respond(&kernel_url(), 200, pdb(GUID_BYTES, 1));
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let outcome = go(&mut service, &kernel());
        assert_eq!(
            loaded_from(&outcome),
            Some(&SymbolSource::Server { url: kernel_url(), kept: Kept::InStore { path: store_path() } })
        );
        assert_eq!(loaded_tables(outcome).address_of("PsActiveProcessHead").map(|rva| rva.0), Some(0x1010));
        assert!(service.store().lookup(&store_path()).unwrap().is_some());
    }

    #[test]
    fn a_pdb_missing_by_its_name_is_fetched_as_a_cabinet() {
        let mut http = ScriptedHttp::default();
        let compressed = format!("{MSDL}/ntkrnlmp.pdb/B661C6FE72991D160FE37C48B8BC54701/ntkrnlmp.pd_");
        http.respond(&compressed, 200, cabinet("ntkrnlmp.pdb", &pdb(GUID_BYTES, 1), CabinetKind::MsZip));
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let outcome = go(&mut service, &kernel());
        assert!(matches!(loaded_from(&outcome), Some(SymbolSource::Server { .. })));
        assert_eq!(service.http().requests(), [kernel_url(), compressed]);
        // The store keeps the PDB itself, not the cabinet.
        assert_eq!(service.store().lookup(&store_path()).unwrap(), Some(pdb(GUID_BYTES, 1)));
    }

    #[test]
    fn a_download_of_another_build_never_enters_the_store() {
        let mut http = ScriptedHttp::default();
        http.respond(&kernel_url(), 200, pdb([9; 16], 1));
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let outcome = go(&mut service, &kernel());
        assert!(matches!(unavailable(&outcome), Some(LocateError::WrongBuild { .. })));
        assert_eq!(service.store().lookup(&store_path()).unwrap(), None);
    }

    const TRUNCATED: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0truncated";

    fn bad_path() -> PathBuf {
        PathBuf::from(format!("{}.bad", store_path().display()))
    }

    /// Review focus 4: a crash mid-download, a truncated copy. The download
    /// that replaces it names where the broken copy went.
    #[test]
    fn a_store_file_that_does_not_parse_is_replaced_once_a_download_verifies() {
        let mut files = MemoryStore::default();
        files.insert(store_path(), TRUNCATED.to_vec());
        let mut http = ScriptedHttp::default();
        http.respond(&kernel_url(), 200, pdb(GUID_BYTES, 1));
        let mut service = service(files, http, Downloads::Automatic);
        let outcome = go(&mut service, &kernel());
        assert_eq!(
            loaded_from(&outcome),
            Some(&SymbolSource::Server { url: kernel_url(), kept: Kept::Replaced { path: store_path(), bad: bad_path() } })
        );
        assert_eq!(service.store().lookup(&bad_path()).unwrap(), Some(TRUNCATED.to_vec()), "moved aside, not deleted");
        assert_eq!(service.store().lookup(&store_path()).unwrap(), Some(pdb(GUID_BYTES, 1)));
    }

    /// A store shared with a debugger keeps a file this reader cannot load
    /// until a verified replacement exists: with downloads off the file's
    /// own fault is the reason, and with no server copy it stays.
    #[test]
    fn a_store_file_that_does_not_parse_stays_when_nothing_replaces_it() {
        let mut files = MemoryStore::default();
        files.insert(store_path(), TRUNCATED.to_vec());
        let mut service = service(files, ScriptedHttp::default(), Downloads::Off);
        let outcome = go(&mut service, &kernel());
        assert!(matches!(unavailable(&outcome), Some(LocateError::Parse { .. })));
        assert_eq!(service.store().lookup(&store_path()).unwrap(), Some(TRUNCATED.to_vec()));
        assert_eq!(service.store().lookup(&bad_path()).unwrap(), None);

        let mut files = MemoryStore::default();
        files.insert(store_path(), TRUNCATED.to_vec());
        let mut service = self::service(files, ScriptedHttp::default(), Downloads::Automatic);
        assert!(matches!(go(&mut service, &kernel()), LocateOutcome::NotOnServer));
        assert_eq!(service.store().lookup(&store_path()).unwrap(), Some(TRUNCATED.to_vec()));
        assert_eq!(service.store().lookup(&bad_path()).unwrap(), None);
    }

    #[test]
    fn a_store_binary_that_does_not_read_stays_when_nothing_replaces_it() {
        let module = ModuleIdentity::Pe {
            file_name: "ntoskrnl.exe".to_owned(),
            image: PeImageId { time_date_stamp: 0x0BAD_F00D, size_of_image: 0x3000 },
            pdb: None,
        };
        let kept = PathBuf::from("store").join("ntoskrnl.exe").join("0BADF00D3000").join("ntoskrnl.exe");
        let mut files = MemoryStore::default();
        files.insert(kept.clone(), b"MZ cut short".to_vec());
        let mut service = service(files, ScriptedHttp::default(), Downloads::Off);
        let outcome = go(&mut service, &module);
        assert!(matches!(unavailable(&outcome), Some(LocateError::Binary(_))));
        assert_eq!(service.store().lookup(&kept).unwrap(), Some(b"MZ cut short".to_vec()));
    }

    #[test]
    fn a_network_failure_is_retryable_and_leaves_local_files_working() {
        let other = ModuleIdentity::Pe {
            file_name: "hal.dll".to_owned(),
            image: PeImageId { time_date_stamp: 2, size_of_image: 0x1000 },
            pdb: Some(PdbId { name: "hal.pdb".to_owned(), guid: Guid::from_le_bytes([5; 16]), age: 1 }),
        };
        let key = match &other {
            ModuleIdentity::Pe { pdb: Some(pdb), .. } => pdb.store_key(),
            ModuleIdentity::Pe { pdb: None, .. } | ModuleIdentity::Elf { .. } => String::new(),
        };
        let mut files = MemoryStore::default();
        let hal_path = PathBuf::from("store").join("hal.pdb").join(key).join("hal.pdb");
        files.insert(hal_path.clone(), pdb([5; 16], 1));
        let mut http = ScriptedHttp::default();
        http.unreachable(&kernel_url());
        let mut service = service(files, http, Downloads::Automatic);
        let outcome = go(&mut service, &kernel());
        let reason = unavailable(&outcome).expect("unavailable");
        assert_eq!(reason.retry(), Retry::Retryable, "{reason}");
        assert_eq!(loaded_from(&go(&mut service, &other)), Some(&SymbolSource::Store { path: hal_path }));

        let mut http = ScriptedHttp::default();
        http.respond(&kernel_url(), 503, Vec::new());
        let mut service = self::service(MemoryStore::default(), http, Downloads::Automatic);
        let outcome = go(&mut service, &kernel());
        assert!(matches!(unavailable(&outcome), Some(LocateError::Status { status: 503, retry: Retry::Retryable, .. })));
    }

    #[test]
    fn a_file_on_no_server_is_not_on_the_server() {
        let mut service = service(MemoryStore::default(), ScriptedHttp::default(), Downloads::Automatic);
        assert!(matches!(go(&mut service, &kernel()), LocateOutcome::NotOnServer));
    }

    #[test]
    fn with_downloads_off_nothing_is_fetched() {
        let mut service = service(MemoryStore::default(), ScriptedHttp::default(), Downloads::Off);
        assert!(matches!(unavailable(&go(&mut service, &kernel())), Some(LocateError::DownloadsOff)));
        assert!(service.http().requests().is_empty());
    }

    #[test]
    fn a_file_from_another_build_is_refused_with_both_identities() {
        let mut files = MemoryStore::default();
        let picked = PathBuf::from("Downloads").join("ntkrnlmp.pdb");
        files.insert(picked.clone(), pdb(GUID_BYTES, 2));
        let mut service = service(files, ScriptedHttp::default(), Downloads::Off);
        let LocateOutcome::Refused { found, wanted, path } = service.attach(&picked, &kernel(), Consent::MatchOnly) else {
            panic!("expected a refusal")
        };
        assert_eq!(found, TablesIdentity::Pdb { guid: guid(), age: 2 });
        assert_eq!(wanted, kernel());
        assert_eq!(path, picked);
    }

    #[test]
    fn load_anyway_loads_a_mismatched_file_as_forced() {
        let mut files = MemoryStore::default();
        let picked = PathBuf::from("Downloads").join("ntkrnlmp.pdb");
        files.insert(picked.clone(), pdb(GUID_BYTES, 2));
        let mut service = service(files, ScriptedHttp::default(), Downloads::Off);
        let LocateOutcome::Forced { tables, found, path } = service.attach(&picked, &kernel(), Consent::LoadAnyway) else {
            panic!("forced")
        };
        assert_eq!(found, TablesIdentity::Pdb { guid: guid(), age: 2 });
        assert_eq!(path, picked);
        assert!(tables.address_of("PsActiveProcessHead").is_some());
        // The right build attaches without asking.
        let right = PathBuf::from("Downloads").join("right.pdb");
        service.store().files().insert(right.clone(), pdb(GUID_BYTES, 1));
        assert_eq!(loaded_from(&service.attach(&right, &kernel(), Consent::MatchOnly)), Some(&SymbolSource::Attached { path: right }));
    }

    /// A user-mode module's pageable debug directory.
    #[test]
    fn a_module_without_its_pdb_identity_is_found_through_its_binary() {
        let binary = pe32_file(&rsds(GUID_BYTES, 1, r"c:\build\ntkrnlmp.pdb"));
        let module = ModuleIdentity::Pe {
            file_name: "ntoskrnl.exe".to_owned(),
            image: PeImageId { time_date_stamp: 0x0BAD_F00D, size_of_image: 0x3000 },
            pdb: None,
        };
        let mut http = ScriptedHttp::default();
        let binary_url = format!("{MSDL}/ntoskrnl.exe/0BADF00D3000/ntoskrnl.exe");
        http.respond(&binary_url, 200, binary);
        http.respond(&kernel_url(), 200, pdb(GUID_BYTES, 1));
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let outcome = go(&mut service, &module);
        assert!(matches!(loaded_from(&outcome), Some(SymbolSource::Server { .. })));
        assert_eq!(service.http().requests(), [binary_url, kernel_url()]);
        let kept = PathBuf::from("store").join("ntoskrnl.exe").join("0BADF00D3000").join("ntoskrnl.exe");
        assert!(service.store().lookup(&kept).unwrap().is_some(), "the binary is kept");
    }

    #[test]
    fn an_elf_module_is_fetched_from_debuginfod() {
        let build_id = BuildId(BUILD_ID.to_vec());
        let module = ModuleIdentity::Elf { file_name: "widget".into(), build_id: build_id.clone() };
        let file = build_elf(ElfSpec::pie(), &[]);
        let url = format!("{DEBUGINFOD}/buildid/{}/debuginfo", build_id.to_hex());
        let mut http = ScriptedHttp::default();
        http.respond(&url, 200, file);
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let outcome = go(&mut service, &module);
        let debug_path = service.store().debug_file_path(&build_id);
        assert_eq!(loaded_from(&outcome), Some(&SymbolSource::Server { url, kept: Kept::InStore { path: debug_path } }));
    }

    /// A dwz debug file's supplement is fetched by the build-id its
    /// `.gnu_debugaltlink` names, verified, and kept.
    #[test]
    fn a_dwz_supplement_is_fetched_by_its_build_id() {
        let pair = dwz_pair(DwzForms::Gnu, TEXT_RVA);
        let main = build_elf(ElfSpec { symtab: Symtab::Without, ..ElfSpec::pie() }, &pair.main);
        let build_id = BuildId(BUILD_ID.to_vec());
        let supplement_id = BuildId(SUPPLEMENT_BUILD_ID.to_vec());
        let module = ModuleIdentity::Elf { file_name: "widget".into(), build_id: build_id.clone() };
        let mut http = ScriptedHttp::default();
        http.respond(&format!("{DEBUGINFOD}/buildid/{}/debuginfo", build_id.to_hex()), 200, main);
        let supplement_url = format!("{DEBUGINFOD}/buildid/{}/debuginfo", supplement_id.to_hex());
        http.respond(&supplement_url, 200, pair.supplement);
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let tables = loaded_tables(go(&mut service, &module));
        assert_eq!(tables.symbolize(Rva(TEXT_RVA)).map(|hit| hit.name), Some("dwz_function".to_owned()));
        assert!(service.http().requests().contains(&supplement_url));
        let kept = service.store().debug_file_path(&supplement_id);
        assert!(service.store().lookup(&kept).unwrap().is_some(), "the supplement is kept");
    }

    /// Review focus 5: the supplement is not on the server.
    #[test]
    fn a_missing_supplement_does_not_fail_the_lookup() {
        let pair = dwz_pair(DwzForms::Standard, TEXT_RVA);
        let main = build_elf(ElfSpec { symtab: Symtab::Without, ..ElfSpec::pie() }, &pair.main);
        let build_id = BuildId(BUILD_ID.to_vec());
        let module = ModuleIdentity::Elf { file_name: "widget".into(), build_id: build_id.clone() };
        let mut http = ScriptedHttp::default();
        http.respond(&format!("{DEBUGINFOD}/buildid/{}/debuginfo", build_id.to_hex()), 200, main);
        let mut service = service(MemoryStore::default(), http, Downloads::Automatic);
        let tables = loaded_tables(go(&mut service, &module));
        assert_eq!(tables.symbolize(Rva(TEXT_RVA + 0x14)).map(|hit| hit.name), Some("own_function".to_owned()));
        assert_eq!(tables.symbolize(Rva(TEXT_RVA)), None, "missing, not wrong");
    }
}
