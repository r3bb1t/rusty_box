//! The state the shell and the symbol workers share. One `Mutex` guards it;
//! what readers see is an `Arc<SymbolsView>` that a writer replaces whole,
//! so a reader clones the `Arc` under the lock and works without it. Every
//! lock is taken with `unwrap_or_else(PoisonError::into_inner)`: each
//! change is made inside one critical section, so a poisoned state is still
//! whole.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rusty_box_symbols::http::{Flow, Progress, Retry};
use rusty_box_symbols::identity::{ModuleIdentity, TablesIdentity};
use rusty_box_symbols::projects::{ProjectFolders, ProjectIndex};
use rusty_box_symbols::service::{Consent, ServiceConfig, SymbolSource};
use rusty_box_symbols::store::StoreUsage;
use rusty_box_symbols::tables::SymbolTables;

/// What the engine has for one module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleSymbols {
    /// Asked for, not looked up yet.
    Pending,
    Downloading { done: u64, total: Option<u64> },
    Loaded { from: SymbolSource },
    /// A file the owner attached describes another build; Load anyway
    /// loads it.
    Refused { found: TablesIdentity, path: PathBuf },
    /// Loaded from a file of another build on the owner's word: every name
    /// from it is drawn amber.
    Forced { found: TablesIdentity, path: PathBuf },
    /// Every server answered that it does not have the file. Kept for the
    /// run; Retry asks again.
    NotOnServer,
    Failed { reason: String, retry: Retry },
    /// The owner unloaded it: not looked up again this run unless asked.
    Unloaded,
}

/// One module's state, and its tables when they loaded.
#[derive(Clone)]
pub(crate) struct ModuleEntry {
    pub(crate) state: ModuleSymbols,
    pub(crate) tables: Option<Arc<SymbolTables>>,
}

impl ModuleEntry {
    fn pending() -> Self {
        ModuleEntry {
            state: ModuleSymbols::Pending,
            tables: None,
        }
    }
}

/// What measuring the symbol store came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoreMeasure {
    Measured(StoreUsage),
    Failed(String),
}

/// One published state of the hub.
#[derive(Clone, Default)]
pub(crate) struct SymbolsView {
    pub(crate) modules: BTreeMap<ModuleIdentity, ModuleEntry>,
    pub(crate) projects: Arc<ProjectIndex>,
    /// Why the project folders could not be indexed, when they could not.
    pub(crate) projects_problem: Option<String>,
    /// How much the store holds, once a worker measured it.
    pub(crate) store_usage: Option<StoreMeasure>,
    /// Changes with each publication, so a reader can tell a new view from
    /// the last; it is compared for equality only, so it wraps.
    pub(crate) generation: u64,
}

impl SymbolsView {
    pub(crate) fn tables(&self, identity: &ModuleIdentity) -> Option<&Arc<SymbolTables>> {
        self.modules.get(identity).and_then(|entry| entry.tables.as_ref())
    }

    pub(crate) fn state(&self, identity: &ModuleIdentity) -> Option<&ModuleSymbols> {
        self.modules.get(identity).map(|entry| &entry.state)
    }
}

/// Which modules are looked up first: the kernel, then those the panes show,
/// then the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Priority {
    Kernel,
    Shown,
    Rest,
}

/// A module the shell asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WantedModule {
    pub(crate) identity: ModuleIdentity,
    pub(crate) priority: Priority,
}

/// What the workers look up with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HubConfig {
    pub(crate) store: PathBuf,
    pub(crate) service: ServiceConfig,
    pub(crate) projects: Vec<ProjectFolders>,
}

impl HubConfig {
    pub(crate) fn from_settings(settings: &crate::symbol_settings::SymbolSettings) -> Self {
        HubConfig {
            store: settings.store.value.clone(),
            service: settings.service_config(),
            projects: settings.project_folders(),
        }
    }
}

/// Work for a worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Job {
    Locate { identity: ModuleIdentity },
    Attach {
        identity: ModuleIdentity,
        path: PathBuf,
        consent: Consent,
    },
    /// Index the project folders again.
    Rescan,
    /// Count the store's files and bytes.
    MeasureStore,
}

impl Job {
    fn module(&self) -> Option<&ModuleIdentity> {
        match self {
            Job::Locate { identity } | Job::Attach { identity, .. } => Some(identity),
            Job::Rescan | Job::MeasureStore => None,
        }
    }
}

/// A job a worker took, and the settings it is to run with.
pub(crate) struct Ticket {
    pub(crate) job: Job,
    pub(crate) config: Arc<HubConfig>,
}

struct Queued {
    job: Job,
    priority: Priority,
}

struct HubState {
    view: Arc<SymbolsView>,
    /// In arrival order; a worker takes the first of the most urgent.
    queue: Vec<Queued>,
    /// Modules a worker is on; a module has one job running at a time.
    busy: BTreeSet<ModuleIdentity>,
    /// Of those, the ones the owner cancelled.
    cancelled: BTreeSet<ModuleIdentity>,
    config: Arc<HubConfig>,
    closing: bool,
    repaint: Option<egui::Context>,
    /// When a missed lookup last checked the project folders for a newer
    /// build.
    projects_checked: Option<Instant>,
}

/// How long one check of the project folders stands for the misses after
/// it, so a pause's burst of lookups walks the folders once: a build made
/// within it is found on the next pause, or by Retry.
pub(crate) const PROJECT_CHECK_INTERVAL: Duration = Duration::from_secs(5);

impl HubState {
    /// Applies `change` to the view and counts the publication; the shell is
    /// asked to draw it when the lock is let go ([`SymbolHub::update`]).
    fn publish<F: FnOnce(&mut SymbolsView)>(&mut self, change: F) {
        let view = Arc::make_mut(&mut self.view);
        change(view);
        view.generation = view.generation.wrapping_add(1);
    }

    /// Applies `change` to the published modules (readers keep their view).
    fn publish_modules<F: FnOnce(&mut BTreeMap<ModuleIdentity, ModuleEntry>)>(&mut self, change: F) {
        self.publish(|view| change(&mut view.modules));
    }

    fn enqueue(&mut self, job: Job, priority: Priority) {
        self.queue.push(Queued { job, priority });
    }

    /// The most urgent job a worker may start now. A module has one job
    /// running at a time: a job for a busy module waits for `finish`.
    fn take(&mut self) -> Option<Ticket> {
        let busy = &self.busy;
        let index = self
            .queue
            .iter()
            .enumerate()
            .filter(|(_, queued)| queued.job.module().is_none_or(|identity| !busy.contains(identity)))
            .min_by_key(|(at, queued)| QueueKey { priority: queued.priority, at: *at })
            .map(|(at, _)| at)?;
        // `index` came from this queue's own enumeration: it is in range.
        let Queued { job, .. } = self.queue.remove(index);
        if let Some(identity) = job.module() {
            self.busy.insert(identity.clone());
        }
        Some(Ticket {
            job,
            config: Arc::clone(&self.config),
        })
    }
}

/// The order a queue is served in: priority, then arrival.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct QueueKey {
    priority: Priority,
    at: usize,
}

struct HubShared {
    state: Mutex<HubState>,
    wake: Condvar,
}

/// The shell's and the workers' handle on the shared state.
#[derive(Clone)]
pub(crate) struct SymbolHub {
    shared: Arc<HubShared>,
}

impl SymbolHub {
    pub(crate) fn new(config: HubConfig) -> Self {
        SymbolHub {
            shared: Arc::new(HubShared {
                state: Mutex::new(HubState {
                    view: Arc::new(SymbolsView::default()),
                    queue: Vec::new(),
                    busy: BTreeSet::new(),
                    cancelled: BTreeSet::new(),
                    config: Arc::new(config),
                    closing: false,
                    repaint: None,
                    projects_checked: None,
                }),
                wake: Condvar::new(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `change` under the lock. When it published, the shell is asked
    /// to draw once the lock is let go, so whatever egui runs on that
    /// request may read the hub.
    fn update<R, F: FnOnce(&mut HubState) -> R>(&self, change: F) -> R {
        let mut state = self.lock();
        let before = state.view.generation;
        let result = change(&mut state);
        let repaint = if state.view.generation == before { None } else { state.repaint.clone() };
        drop(state);
        if let Some(context) = repaint {
            context.request_repaint();
        }
        result
    }

    // ── the shell's side ──

    /// What the engine has now.
    pub(crate) fn view(&self) -> Arc<SymbolsView> {
        Arc::clone(&self.lock().view)
    }

    /// The shell draws each publication as it lands.
    pub(crate) fn set_repaint(&self, context: egui::Context) {
        self.lock().repaint = Some(context);
    }

    /// Asks for `modules`: one never asked for is looked up; one still
    /// waiting is moved up when it is now more urgent.
    pub(crate) fn want(&self, modules: &[WantedModule]) {
        self.update(|state| self.want_locked(state, modules));
    }

    /// [`Self::want`]'s work, under the lock.
    fn want_locked(&self, state: &mut HubState, modules: &[WantedModule]) {
        let mut fresh = Vec::new();
        for wanted in modules {
            if !state.view.modules.contains_key(&wanted.identity) {
                fresh.push(wanted.identity.clone());
                continue;
            }
            for queued in &mut state.queue {
                let same = queued.job.module() == Some(&wanted.identity);
                if same && wanted.priority < queued.priority {
                    queued.priority = wanted.priority;
                }
            }
        }
        if fresh.is_empty() {
            return;
        }
        for wanted in modules.iter().filter(|wanted| fresh.contains(&wanted.identity)) {
            state.enqueue(
                Job::Locate {
                    identity: wanted.identity.clone(),
                },
                wanted.priority,
            );
        }
        state.publish_modules(|modules| {
            for identity in fresh {
                modules.insert(identity, ModuleEntry::pending());
            }
        });
        self.shared.wake.notify_all();
    }

    /// Looks a module up again: after a failure, a 404, a refusal, an
    /// unload, or a cancel.
    pub(crate) fn retry(&self, identity: &ModuleIdentity) {
        self.update(|state| {
            if state.busy.contains(identity) {
                return;
            }
            state.queue.retain(|queued| queued.job.module() != Some(identity));
            state.enqueue(
                Job::Locate {
                    identity: identity.clone(),
                },
                Priority::Shown,
            );
            state.publish_modules(|modules| {
                modules.insert(identity.clone(), ModuleEntry::pending());
            });
            self.shared.wake.notify_all();
        });
    }

    /// Stops a lookup: a waiting one leaves the queue, a running one stops
    /// at its next read. Either fails, retryably.
    pub(crate) fn cancel(&self, identity: &ModuleIdentity) {
        self.update(|state| {
            let before = state.queue.len();
            state.queue.retain(|queued| queued.job.module() != Some(identity));
            if state.busy.contains(identity) {
                state.cancelled.insert(identity.clone());
                return;
            }
            if state.queue.len() != before {
                state.publish_modules(|modules| {
                    modules.insert(identity.clone(), cancelled());
                });
            }
        });
    }

    /// Drops a module's tables; it is not looked up again until Retry.
    pub(crate) fn unload(&self, identity: &ModuleIdentity) {
        self.update(|state| {
            state.queue.retain(|queued| queued.job.module() != Some(identity));
            if state.busy.contains(identity) {
                state.cancelled.insert(identity.clone());
            }
            state.publish_modules(|modules| {
                modules.insert(
                    identity.clone(),
                    ModuleEntry {
                        state: ModuleSymbols::Unloaded,
                        tables: None,
                    },
                );
            });
        });
    }

    /// Loads the file at `path` for `identity`, ahead of every lookup. A
    /// lookup already running for it is cancelled, and the file is loaded
    /// once that lookup ends.
    pub(crate) fn attach(&self, identity: &ModuleIdentity, path: PathBuf, consent: Consent) {
        self.update(|state| {
            if state.busy.contains(identity) {
                state.cancelled.insert(identity.clone());
            }
            state.queue.retain(|queued| queued.job.module() != Some(identity));
            state.enqueue(
                Job::Attach {
                    identity: identity.clone(),
                    path,
                    consent,
                },
                Priority::Kernel,
            );
            state.publish_modules(|modules| {
                modules.insert(identity.clone(), ModuleEntry::pending());
            });
            self.shared.wake.notify_all();
        });
    }

    /// Indexes the project folders again, ahead of every lookup.
    pub(crate) fn rescan(&self) {
        let mut state = self.lock();
        state.queue.retain(|queued| queued.job != Job::Rescan);
        state.enqueue(Job::Rescan, Priority::Kernel);
        self.shared.wake.notify_all();
    }

    /// Measures the store again, behind every lookup.
    pub(crate) fn measure_store(&self) {
        let mut state = self.lock();
        state.queue.retain(|queued| queued.job != Job::MeasureStore);
        state.enqueue(Job::MeasureStore, Priority::Rest);
        self.shared.wake.notify_all();
    }

    /// New settings: the workers take them with their next job, the projects
    /// are indexed again, the store is measured again, and every module that
    /// was not found is looked up again under them.
    pub(crate) fn reconfigure(&self, config: HubConfig) {
        self.update(|state| {
            state.config = Arc::new(config);
            state.queue.retain(|queued| queued.job != Job::Rescan && queued.job != Job::MeasureStore);
            state.enqueue(Job::Rescan, Priority::Kernel);
            state.enqueue(Job::MeasureStore, Priority::Rest);
            let missing: Vec<ModuleIdentity> = state
                .view
                .modules
                .iter()
                .filter(|(identity, entry)| {
                    matches!(entry.state, ModuleSymbols::NotOnServer | ModuleSymbols::Failed { .. })
                        && !state.busy.contains(*identity)
                })
                .map(|(identity, _)| identity.clone())
                .collect();
            for identity in &missing {
                state.enqueue(
                    Job::Locate {
                        identity: identity.clone(),
                    },
                    Priority::Rest,
                );
            }
            state.publish_modules(|modules| {
                for identity in missing {
                    modules.insert(identity, ModuleEntry::pending());
                }
            });
            self.shared.wake.notify_all();
        });
    }

    /// Tells the workers to stop: a waiting one wakes and returns, a busy
    /// one stops at its next read.
    pub(crate) fn close(&self) {
        let mut state = self.lock();
        state.closing = true;
        let busy: Vec<ModuleIdentity> = state.busy.iter().cloned().collect();
        state.cancelled.extend(busy);
        self.shared.wake.notify_all();
    }

    // ── the workers' side ──

    /// The next job, waiting for one; `None` once the hub is closing.
    pub(crate) fn next_job(&self) -> Option<Ticket> {
        let mut state = self.lock();
        loop {
            if state.closing {
                return None;
            }
            if let Some(ticket) = state.take() {
                return Some(ticket);
            }
            state = self.shared.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Whether no one holds the hub's lock now: how a test sees that the
    /// shell is asked to draw outside it.
    #[cfg(test)]
    pub(crate) fn lock_is_free(&self) -> bool {
        self.shared.state.try_lock().is_ok()
    }

    /// The next job, when one is queued now: how tests drive a worker one
    /// job at a time.
    #[cfg(test)]
    pub(crate) fn try_next_job(&self) -> Option<Ticket> {
        let mut state = self.lock();
        if state.closing {
            return None;
        }
        state.take()
    }

    /// Whether a lookup that missed may check the project folders for a
    /// newer build at `now`: once per [`PROJECT_CHECK_INTERVAL`], whichever
    /// worker asks.
    pub(crate) fn project_check_due(&self, now: Instant) -> bool {
        let mut state = self.lock();
        let due = state
            .projects_checked
            .is_none_or(|last| now.saturating_duration_since(last) >= PROJECT_CHECK_INTERVAL);
        if due {
            state.projects_checked = Some(now);
        }
        due
    }

    /// A download's progress; `Flow::Cancel` when the owner cancelled it or
    /// the hub is closing.
    pub(crate) fn progress(&self, identity: &ModuleIdentity, progress: Progress) -> Flow {
        self.update(|state| {
            if state.cancelled.contains(identity) {
                return Flow::Cancel;
            }
            state.publish_modules(|modules| {
                modules.insert(
                    identity.clone(),
                    ModuleEntry {
                        state: ModuleSymbols::Downloading {
                            done: progress.done,
                            total: progress.total,
                        },
                        tables: None,
                    },
                );
            });
            Flow::Continue
        })
    }

    /// A job's outcome for `identity`. A module unloaded while it was being
    /// looked up stays unloaded; one with a newer job queued (an attach)
    /// shows that job's outcome when it ends; one cancelled shows the
    /// cancel. A job held back for this module may start now.
    pub(crate) fn finish(&self, identity: &ModuleIdentity, entry: ModuleEntry) {
        self.update(|state| {
            state.busy.remove(identity);
            let was_cancelled = state.cancelled.remove(identity);
            self.shared.wake.notify_all();
            let unloaded = matches!(state.view.state(identity), Some(ModuleSymbols::Unloaded));
            let superseded = state.queue.iter().any(|queued| queued.job.module() == Some(identity));
            if unloaded || superseded {
                return;
            }
            state.publish_modules(|modules| {
                // A cancelled download ends as a failure, shown as the cancel.
                let entry = if was_cancelled && matches!(entry.state, ModuleSymbols::Failed { .. }) {
                    cancelled()
                } else {
                    entry
                };
                modules.insert(identity.clone(), entry);
            });
        });
    }

    /// A freshly built project index.
    pub(crate) fn set_projects(&self, projects: ProjectIndex) {
        self.update(|state| {
            state.publish(|view| {
                view.projects = Arc::new(projects);
                view.projects_problem = None;
            });
        });
    }

    /// Why the project folders could not be indexed; the last index stands.
    pub(crate) fn fail_projects(&self, reason: String) {
        self.update(|state| state.publish(|view| view.projects_problem = Some(reason)));
    }

    /// What measuring the store came to.
    pub(crate) fn set_store_usage(&self, measure: StoreMeasure) {
        self.update(|state| state.publish(|view| view.store_usage = Some(measure)));
    }
}

/// A cancelled lookup: failed, and Retry may succeed.
fn cancelled() -> ModuleEntry {
    ModuleEntry {
        state: ModuleSymbols::Failed {
            reason: "cancelled".to_owned(),
            retry: Retry::Retryable,
        },
        tables: None,
    }
}
