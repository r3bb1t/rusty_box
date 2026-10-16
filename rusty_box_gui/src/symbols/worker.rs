//! The symbol workers: four threads, each with its own `SymbolService` over
//! the file system and one shared HTTP connection pool, taking jobs from the
//! hub so at most four downloads run at once.

use std::sync::Arc;

use rusty_box_symbols::fs_store::FsStore;
use rusty_box_symbols::http::HttpClient;
use rusty_box_symbols::identity::ModuleIdentity;
use rusty_box_symbols::projects::{Freshness, ProjectIndex};
use rusty_box_symbols::service::{attach_file, LocateOutcome, SymbolService};
use rusty_box_symbols::store::{FileStore, StoreError, SymbolStore};
use rusty_box_symbols::ureq_http::UreqHttp;

use super::hub::{HubConfig, Job, ModuleEntry, ModuleSymbols, StoreMeasure, SymbolHub, Ticket};

/// How many lookups run at once.
pub(crate) const WORKER_COUNT: u32 = 4;

/// The running workers. Dropping it tells them to stop; they are not
/// joined, because a worker in a download only stops at its next read, and
/// the process need not wait for it: every store write lands whole or not
/// at all.
pub(crate) struct SymbolWorkers {
    hub: SymbolHub,
}

impl Drop for SymbolWorkers {
    fn drop(&mut self) {
        self.hub.close();
    }
}

/// A worker's service over the file system and the network.
fn production_service(config: &HubConfig, writer: u32, http: &UreqHttp) -> Result<SymbolService<FsStore, UreqHttp>, StoreError> {
    let store = SymbolStore::open(FsStore::new(writer), config.store.clone())?;
    Ok(SymbolService::new(store, http.clone(), config.service.clone()))
}

/// Starts the workers on `hub`. When one does not start, the hub is closed,
/// so those that did end too, and the error is returned.
pub(crate) fn start(hub: &SymbolHub) -> std::io::Result<SymbolWorkers> {
    let http = UreqHttp::new();
    for writer in 0..WORKER_COUNT {
        let worker_hub = hub.clone();
        let worker_http = http.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("rusty_box_symbols_{writer}"))
            .spawn(move || {
                serve(&worker_hub, |config| production_service(config, writer, &worker_http), || FsStore::new(writer))
            });
        match spawned {
            // The handle is let go: see `SymbolWorkers`.
            Ok(handle) => drop(handle),
            Err(error) => {
                hub.close();
                return Err(error);
            }
        }
    }
    Ok(SymbolWorkers { hub: hub.clone() })
}

/// The service a worker holds, and the settings it was made from.
pub(crate) struct Current<F: FileStore, H: HttpClient> {
    config: Arc<HubConfig>,
    service: SymbolService<F, H>,
}

/// Takes jobs until the hub closes. `make` builds the service over the
/// store; `bare` gives a file reader alone, for what needs no store.
pub(crate) fn serve<F, H, M, B>(hub: &SymbolHub, mut make: M, mut bare: B)
where
    F: FileStore,
    H: HttpClient,
    M: FnMut(&HubConfig) -> Result<SymbolService<F, H>, StoreError>,
    B: FnMut() -> F,
{
    let mut current: Option<Current<F, H>> = None;
    while let Some(ticket) = hub.next_job() {
        perform(hub, &mut current, &mut make, &mut bare, ticket);
    }
}

/// One job: the service is made again when the settings changed. When the
/// store does not open, an attach still reads its file through `bare`.
pub(crate) fn perform<F, H, M, B>(
    hub: &SymbolHub,
    current: &mut Option<Current<F, H>>,
    make: &mut M,
    bare: &mut B,
    ticket: Ticket,
) where
    F: FileStore,
    H: HttpClient,
    M: FnMut(&HubConfig) -> Result<SymbolService<F, H>, StoreError>,
    B: FnMut() -> F,
{
    let stale = current
        .as_ref()
        .is_none_or(|current| !Arc::ptr_eq(&current.config, &ticket.config));
    if stale {
        *current = match make(&ticket.config) {
            Ok(service) => Some(Current {
                config: Arc::clone(&ticket.config),
                service,
            }),
            Err(error) => {
                let reason = format!("the symbol store does not open: {error}");
                match &ticket.job {
                    // A picked file is read without the store.
                    Job::Attach { identity, path, consent } => {
                        hub.finish(identity, entry_of(attach_file(&mut bare(), path, identity, *consent)));
                    }
                    Job::Locate { identity } => hub.finish(
                        identity,
                        ModuleEntry {
                            state: ModuleSymbols::Failed {
                                reason,
                                retry: rusty_box_symbols::http::Retry::Retryable,
                            },
                            tables: None,
                        },
                    ),
                    Job::Rescan => hub.fail_projects(reason),
                    Job::MeasureStore => hub.set_store_usage(StoreMeasure::Failed(reason)),
                }
                return;
            }
        };
    }
    let Some(current) = current.as_mut() else {
        return;
    };
    match ticket.job {
        // A failure is published by `rescan` itself.
        Job::Rescan => match rescan(hub, current) {
            Indexed::Rebuilt | Indexed::Failed => {}
        },
        Job::Locate { identity } => {
            let mut outcome = locate(hub, current, &identity);
            // A module none of the projects' indexed files matched may be in
            // a build made since: index again when the folders changed. One
            // check stands for a burst of misses (`PROJECT_CHECK_INTERVAL`).
            if !matches!(outcome, LocateOutcome::Loaded { .. })
                && !current.config.projects.is_empty()
                && hub.project_check_due(std::time::Instant::now())
            {
                let projects = hub.view().projects.clone();
                match projects.freshness(current.service.store().files(), &current.config.projects) {
                    Ok(Freshness::Current) => {}
                    Ok(Freshness::Stale) => {
                        if rescan(hub, current) == Indexed::Rebuilt {
                            outcome = locate(hub, current, &identity);
                        }
                    }
                    Err(error) => hub.fail_projects(error.to_string()),
                }
            }
            hub.finish(&identity, entry_of(outcome));
        }
        Job::Attach { identity, path, consent } => {
            let outcome = current.service.attach(&path, &identity, consent);
            hub.finish(&identity, entry_of(outcome));
        }
        Job::MeasureStore => hub.set_store_usage(match current.service.store().usage() {
            Ok(usage) => StoreMeasure::Measured(usage),
            Err(error) => StoreMeasure::Failed(error.to_string()),
        }),
    }
}

fn locate<F: FileStore, H: HttpClient>(hub: &SymbolHub, current: &mut Current<F, H>, identity: &ModuleIdentity) -> LocateOutcome {
    let projects = hub.view().projects.clone();
    current
        .service
        .locate(identity, &projects, |progress| hub.progress(identity, progress))
}

/// Whether a rescan built a new index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Indexed {
    Rebuilt,
    Failed,
}

/// Indexes the project folders and publishes the index, or why it failed.
fn rescan<F: FileStore, H: HttpClient>(hub: &SymbolHub, current: &mut Current<F, H>) -> Indexed {
    match ProjectIndex::build(current.service.store().files(), &current.config.projects) {
        Ok(index) => {
            hub.set_projects(index);
            Indexed::Rebuilt
        }
        Err(error) => {
            hub.fail_projects(error.to_string());
            Indexed::Failed
        }
    }
}

/// What a lookup's outcome is to the shell.
pub(crate) fn entry_of(outcome: LocateOutcome) -> ModuleEntry {
    match outcome {
        LocateOutcome::Loaded { tables, from } => ModuleEntry {
            state: ModuleSymbols::Loaded { from },
            tables: Some(tables),
        },
        LocateOutcome::Refused { found, path, .. } => ModuleEntry {
            state: ModuleSymbols::Refused { found, path },
            tables: None,
        },
        LocateOutcome::Forced { tables, found, path } => ModuleEntry {
            state: ModuleSymbols::Forced { found, path },
            tables: Some(tables),
        },
        LocateOutcome::NotOnServer => ModuleEntry {
            state: ModuleSymbols::NotOnServer,
            tables: None,
        },
        LocateOutcome::Unavailable { reason } => ModuleEntry {
            state: ModuleSymbols::Failed {
                retry: reason.retry(),
                reason: reason.to_string(),
            },
            tables: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::hub::{Priority, WantedModule};
    use rusty_box_symbols::http::{Flow, Progress, Retry};
    use rusty_box_symbols::identity::{Guid, PdbId, PeImageId};
    use rusty_box_symbols::memory_store::MemoryStore;
    use rusty_box_symbols::projects::ProjectFolders;
    use rusty_box_symbols::scripted_http::ScriptedHttp;
    use rusty_box_symbols::service::{Consent, Downloads, ServiceConfig, SymbolSource, DEFAULT_MAX_DOWNLOAD};
    use rusty_box_symbols_fixtures::pdb::{build_pdb, FixturePublic, PdbFixture, PublicKind};
    use std::path::PathBuf;

    const MSDL: &str = "https://msdl.example/symbols";

    fn module(name: &str, guid_byte: u8) -> ModuleIdentity {
        ModuleIdentity::Pe {
            file_name: format!("{name}.sys"),
            image: PeImageId { time_date_stamp: u32::from(guid_byte), size_of_image: 0x1000 },
            pdb: Some(PdbId { name: format!("{name}.pdb"), guid: Guid::from_le_bytes([guid_byte; 16]), age: 1 }),
        }
    }

    fn pdb(guid_byte: u8) -> Vec<u8> {
        let mut fixture = PdbFixture::new([guid_byte; 16], 1);
        fixture.publics.push(FixturePublic { name: "Entry".into(), section: 1, offset: 0, kind: PublicKind::Function });
        build_pdb(&fixture)
    }

    fn url(identity: &ModuleIdentity) -> String {
        match identity {
            ModuleIdentity::Pe { pdb: Some(pdb), .. } => format!("{MSDL}/{0}/{1}/{0}", pdb.name, pdb.store_key()),
            ModuleIdentity::Pe { pdb: None, .. } | ModuleIdentity::Elf { .. } => String::new(),
        }
    }

    fn config(projects: Vec<ProjectFolders>) -> HubConfig {
        HubConfig {
            store: PathBuf::from("store"),
            service: ServiceConfig {
                downloads: Downloads::Automatic,
                microsoft: vec![MSDL.to_owned()],
                debuginfod: Vec::new(),
                max_download: DEFAULT_MAX_DOWNLOAD,
            },
            projects,
        }
    }

    /// A worker over an in-memory store and scripted server, its service made
    /// the first time a job asks for it.
    struct Worker {
        ready: Option<SymbolService<MemoryStore, ScriptedHttp>>,
        current: Option<Current<MemoryStore, ScriptedHttp>>,
    }

    impl Worker {
        fn new(config: &HubConfig, files: MemoryStore, http: ScriptedHttp) -> Self {
            let store = SymbolStore::open(files, config.store.clone()).expect("store");
            Worker { ready: Some(SymbolService::new(store, http, config.service.clone())), current: None }
        }

        fn perform(&mut self, hub: &SymbolHub, ticket: Ticket) {
            let ready = &mut self.ready;
            perform(hub, &mut self.current, &mut |_: &HubConfig| Ok(ready.take().expect("made once")), &mut MemoryStore::default, ticket);
        }

        /// Performs every job queued now.
        fn drain(&mut self, hub: &SymbolHub) {
            while let Some(ticket) = hub.try_next_job() {
                self.perform(hub, ticket);
            }
        }

        fn files(&mut self) -> &mut MemoryStore {
            self.current.as_mut().expect("made").service.store().files()
        }
    }

    fn want(hub: &SymbolHub, identity: &ModuleIdentity, priority: Priority) {
        hub.want(&[WantedModule { identity: identity.clone(), priority }]);
    }

    #[test]
    fn a_wanted_module_is_looked_up_once_and_its_tables_published() {
        let kernel = module("ntkrnlmp", 1);
        let mut http = ScriptedHttp::default();
        http.respond(&url(&kernel), 200, pdb(1));
        let config = config(Vec::new());
        let hub = SymbolHub::new(config.clone());
        let mut worker = Worker::new(&config, MemoryStore::default(), http);
        want(&hub, &kernel, Priority::Kernel);
        want(&hub, &kernel, Priority::Kernel);
        assert_eq!(hub.view().state(&kernel), Some(&ModuleSymbols::Pending));
        worker.drain(&hub);
        let view = hub.view();
        assert!(matches!(view.state(&kernel), Some(ModuleSymbols::Loaded { from: SymbolSource::Server { .. } })));
        assert!(view.tables(&kernel).and_then(|tables| tables.address_of("Entry")).is_some());
        assert_eq!(worker.files().paths().len(), 1, "kept in the store, looked up once");
    }

    #[test]
    fn the_kernel_is_looked_up_before_the_rest() {
        let hub = SymbolHub::new(config(Vec::new()));
        let driver = module("driver", 2);
        let kernel = module("ntkrnlmp", 1);
        want(&hub, &driver, Priority::Rest);
        want(&hub, &kernel, Priority::Kernel);
        let first = hub.try_next_job().expect("a job");
        assert_eq!(first.job, Job::Locate { identity: kernel });
    }

    #[test]
    fn a_cancelled_lookup_fails_retryably_and_retry_asks_again() {
        let kernel = module("ntkrnlmp", 1);
        let mut http = ScriptedHttp::default();
        http.respond(&url(&kernel), 200, pdb(1));
        let config = config(Vec::new());
        let hub = SymbolHub::new(config.clone());
        let mut worker = Worker::new(&config, MemoryStore::default(), http);
        want(&hub, &kernel, Priority::Kernel);
        let ticket = hub.try_next_job().expect("a job");
        hub.cancel(&kernel);
        assert_eq!(hub.progress(&kernel, Progress { done: 1, total: None }), Flow::Cancel);
        worker.perform(&hub, ticket);
        assert_eq!(
            hub.view().state(&kernel),
            Some(&ModuleSymbols::Failed { reason: "cancelled".to_owned(), retry: Retry::Retryable })
        );
        hub.retry(&kernel);
        assert_eq!(hub.view().state(&kernel), Some(&ModuleSymbols::Pending));
        worker.drain(&hub);
        assert!(matches!(hub.view().state(&kernel), Some(ModuleSymbols::Loaded { .. })));
    }

    /// Review focus 3: a file of another build is refused, and Load anyway
    /// loads it as forced.
    #[test]
    fn load_anyway_after_a_refusal_loads_the_file_as_forced() {
        let kernel = module("ntkrnlmp", 1);
        let picked = PathBuf::from("Downloads").join("ntkrnlmp.pdb");
        let mut files = MemoryStore::default();
        files.insert(picked.clone(), pdb(9));
        let config = config(Vec::new());
        let hub = SymbolHub::new(config.clone());
        let mut worker = Worker::new(&config, files, ScriptedHttp::default());
        hub.attach(&kernel, picked.clone(), Consent::MatchOnly);
        worker.drain(&hub);
        assert!(matches!(hub.view().state(&kernel), Some(ModuleSymbols::Refused { path, .. }) if *path == picked));
        assert!(hub.view().tables(&kernel).is_none());
        hub.attach(&kernel, picked, Consent::LoadAnyway);
        worker.drain(&hub);
        assert!(matches!(hub.view().state(&kernel), Some(ModuleSymbols::Forced { .. })));
        assert!(hub.view().tables(&kernel).is_some());
    }

    /// The owner attaches a file while the server lookup runs, and a second
    /// worker is free: the attached file is what shows, whichever job ends
    /// last.
    #[test]
    fn a_file_attached_during_a_lookup_is_not_lost() {
        let kernel = module("ntkrnlmp", 1);
        let picked = PathBuf::from("Downloads").join("ntkrnlmp.pdb");
        let mut http = ScriptedHttp::default();
        http.respond(&url(&kernel), 404, Vec::new());
        let config = config(Vec::new());
        let hub = SymbolHub::new(config.clone());
        let holding_the_file = || {
            let mut files = MemoryStore::default();
            files.insert(picked.clone(), pdb(1));
            files
        };
        let mut looking = Worker::new(&config, holding_the_file(), http);
        let mut free = Worker::new(&config, holding_the_file(), ScriptedHttp::default());
        want(&hub, &kernel, Priority::Kernel);
        let lookup = hub.try_next_job().expect("the lookup");
        hub.attach(&kernel, picked, Consent::MatchOnly);
        // The free worker takes what it may now and finishes first.
        free.drain(&hub);
        looking.perform(&hub, lookup);
        free.drain(&hub);
        assert!(matches!(hub.view().state(&kernel), Some(ModuleSymbols::Loaded { .. })));
        assert!(hub.view().tables(&kernel).is_some());
    }

    /// A pause asks for many modules at once; their misses walk the project
    /// folders once between them, not once each.
    #[test]
    fn a_burst_of_misses_lists_the_project_folders_once() {
        let folder = PathBuf::from("work").join("build");
        let projects = vec![ProjectFolders { name: "mydriver".into(), symbols: vec![folder] }];
        let config = config(projects);
        let hub = SymbolHub::new(config.clone());
        let mut worker = Worker::new(&config, MemoryStore::default(), ScriptedHttp::default());
        for missing in [module("one", 10), module("two", 11), module("three", 12)] {
            want(&hub, &missing, Priority::Rest);
        }
        worker.drain(&hub);
        assert_eq!(worker.files().listings(), 1);
    }

    /// The store does not open (its network drive is gone) and the owner
    /// attaches a file from disk: it loads, since reading it needs no store.
    #[test]
    fn a_file_attaches_while_the_store_does_not_open() {
        let kernel = module("ntkrnlmp", 1);
        let picked = PathBuf::from("Downloads").join("ntkrnlmp.pdb");
        let config = config(Vec::new());
        let hub = SymbolHub::new(config);
        hub.attach(&kernel, picked.clone(), Consent::MatchOnly);
        let ticket = hub.try_next_job().expect("the attach");
        let mut current: Option<Current<MemoryStore, ScriptedHttp>> = None;
        let mut unopened = |config: &HubConfig| -> Result<SymbolService<MemoryStore, ScriptedHttp>, StoreError> {
            Err(StoreError::Inspect { path: config.store.clone(), source: std::io::Error::other("the drive is gone") })
        };
        let mut bare = || {
            let mut files = MemoryStore::default();
            files.insert(picked.clone(), pdb(1));
            files
        };
        perform(&hub, &mut current, &mut unopened, &mut bare, ticket);
        assert!(
            matches!(hub.view().state(&kernel), Some(ModuleSymbols::Loaded { from: SymbolSource::Attached { .. } })),
            "{:?}",
            hub.view().state(&kernel)
        );
        assert!(hub.view().tables(&kernel).is_some());
    }

    /// The owner rebuilt the driver after the folders were indexed: the
    /// lookup misses, sees the folder changed, indexes again and finds it.
    #[test]
    fn a_build_made_after_indexing_is_found_by_indexing_again() {
        let driver = module("mydriver", 4);
        let folder = PathBuf::from("work").join("build");
        let projects = vec![ProjectFolders { name: "mydriver".into(), symbols: vec![folder.clone()] }];
        let config = config(projects);
        let hub = SymbolHub::new(config.clone());
        let mut worker = Worker::new(&config, MemoryStore::default(), ScriptedHttp::default());
        hub.rescan();
        worker.drain(&hub);
        assert!(hub.view().projects.is_empty());
        worker.files().insert(folder.join("mydriver.pdb"), pdb(4));
        want(&hub, &driver, Priority::Shown);
        worker.drain(&hub);
        assert!(matches!(hub.view().state(&driver), Some(ModuleSymbols::Loaded { from: SymbolSource::Project { .. } })));
        assert_eq!(hub.view().projects.len(), 1);
    }

    #[test]
    fn an_unloaded_module_stays_unloaded_until_retried() {
        let kernel = module("ntkrnlmp", 1);
        let mut http = ScriptedHttp::default();
        http.respond(&url(&kernel), 200, pdb(1));
        let config = config(Vec::new());
        let hub = SymbolHub::new(config.clone());
        let mut worker = Worker::new(&config, MemoryStore::default(), http);
        want(&hub, &kernel, Priority::Kernel);
        worker.drain(&hub);
        hub.unload(&kernel);
        want(&hub, &kernel, Priority::Kernel);
        worker.drain(&hub);
        assert_eq!(hub.view().state(&kernel), Some(&ModuleSymbols::Unloaded));
        assert!(hub.view().tables(&kernel).is_none());
        hub.retry(&kernel);
        worker.drain(&hub);
        assert!(matches!(hub.view().state(&kernel), Some(ModuleSymbols::Loaded { .. })));
    }

    /// The shell is asked to draw only once the hub has let go of its lock:
    /// whatever egui runs on a repaint request may read the hub.
    #[test]
    fn the_shell_is_asked_to_draw_after_the_hub_lets_go() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hub = SymbolHub::new(config(Vec::new()));
        let requests = Arc::new(AtomicUsize::new(0));
        let while_locked = Arc::new(AtomicUsize::new(0));
        let context = egui::Context::default();
        let probe = hub.clone();
        let counted = Arc::clone(&requests);
        let locked = Arc::clone(&while_locked);
        context.set_request_repaint_callback(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            if !probe.lock_is_free() {
                locked.fetch_add(1, Ordering::SeqCst);
            }
        });
        hub.set_repaint(context);
        let kernel = module("ntkrnlmp", 1);
        want(&hub, &kernel, Priority::Kernel);
        hub.cancel(&kernel);
        // egui reports a request when it moves the next frame earlier, so
        // two in one frame report once.
        assert!(requests.load(Ordering::SeqCst) >= 1, "a publication asks for a frame");
        assert_eq!(while_locked.load(Ordering::SeqCst), 0, "no request is made under the hub's lock");
    }

    #[test]
    fn a_reader_keeps_its_view_while_the_hub_moves_on() {
        let hub = SymbolHub::new(config(Vec::new()));
        let before = hub.view();
        want(&hub, &module("ntkrnlmp", 1), Priority::Kernel);
        let after = hub.view();
        assert!(before.modules.is_empty(), "a published view never changes");
        assert_eq!(after.modules.len(), 1);
        assert_ne!(after.generation, before.generation);
    }

    #[test]
    fn closing_the_hub_ends_a_waiting_worker() {
        let hub = SymbolHub::new(config(Vec::new()));
        let worker_hub = hub.clone();
        let worker = std::thread::spawn(move || {
            serve(
                &worker_hub,
                |config: &HubConfig| {
                    SymbolStore::open(MemoryStore::default(), config.store.clone())
                        .map(|store| SymbolService::new(store, ScriptedHttp::default(), config.service.clone()))
                },
                MemoryStore::default,
            )
        });
        hub.close();
        worker.join().expect("the worker ends");
    }
}
