//! A VM's cross-reference service (spec §4–5): the indexes of its modules,
//! built by one coordinator thread on a pool of workers from images the run
//! thread copies, and the answers read from them.
//!
//! The run thread hands each copied image to [`XrefService::accept`]; one
//! coordinator thread takes them in order and indexes each on
//! [`Workers::for_this_machine`] scoped workers, so the run thread copies
//! the next module while the last is indexed, and neither it nor the UI waits.
//! The indexes live under one lock, taken only to publish or read a state.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, SendError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::decode::XrefKind;
use super::image::{ImageOffset, ModuleImage};
use super::index::{build_index, IndexEnd, IndexWork, ModuleIndex, ModuleSpan, Site, Workers};
use crate::debugger::guest::Root;
use crate::debugger::module_identity::GuestIdentity;

/// Whose address space a module's index was read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndexSpace {
    /// The kernel half, which every process shares.
    Kernel,
    Process { cr3: u64 },
}

/// One module as it was loaded: an index is used only for the same key, so a
/// module that moved or changed build is indexed again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleKey {
    pub(crate) identity: GuestIdentity,
    pub(crate) name: String,
    pub(crate) base: u64,
    pub(crate) size: u64,
    pub(crate) space: IndexSpace,
}

/// Which modules an answer is read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XrefScope {
    /// The module holding the target.
    TargetModule,
    /// Every module of the pause.
    AllModules,
}

/// Linear addresses `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AddressRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
}

/// What references are asked for: a source line may compile to several
/// ranges, so a target is a list of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct XrefTarget {
    pub(crate) ranges: Vec<AddressRange>,
    pub(crate) label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct XrefQuery {
    pub(crate) target: XrefTarget,
    pub(crate) scope: XrefScope,
}

/// A module the run thread is asked to copy, and the address space to read
/// it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CopyModule {
    pub(crate) key: ModuleKey,
    pub(crate) root: Root,
}

/// What the run thread copies: `modules`; `spans` are every module of the
/// pause, which a data pointer may point into; `taken_at` the pause's
/// instruction count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CopyRequest {
    pub(crate) modules: Vec<CopyModule>,
    pub(crate) spans: Vec<ModuleSpan>,
    pub(crate) taken_at: u64,
}

/// One module's index, as far as it got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IndexState {
    /// The run thread is to copy it.
    Copying,
    /// Copied, waiting for the coordinator.
    Queued,
    /// `done` of `of` bytes indexed.
    Indexing { done: u64, of: u64 },
    Ready(Arc<ModuleIndex>),
    Failed(String),
}

/// One reference found: where, how, in which module; for a pointer, the code
/// that reads the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoundSite {
    pub(crate) site: u64,
    pub(crate) kind: XrefKind,
    pub(crate) module: String,
    pub(crate) through: Vec<FoundSite>,
}

/// The references a query has, from the scope's modules that are indexed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct XrefAnswer {
    pub(crate) query: XrefQuery,
    /// By site.
    pub(crate) found: Vec<FoundSite>,
    /// Modules not indexed yet.
    pub(crate) waiting: Vec<String>,
    /// Modules that could not be indexed, each with why.
    pub(crate) failed: Vec<String>,
    /// Pages of the indexed modules that did not read.
    pub(crate) holes: u32,
    /// The oldest pause the indexes were taken at.
    pub(crate) taken_at: Option<u64>,
}

/// A copied image for the coordinator.
pub(crate) struct IndexJob {
    pub(crate) key: ModuleKey,
    pub(crate) image: ModuleImage,
    pub(crate) extra_sync: Vec<ImageOffset>,
    pub(crate) spans: Arc<[ModuleSpan]>,
    pub(crate) taken_at: u64,
}

#[derive(Clone, Debug)]
enum EntryState {
    Copying,
    Queued,
    Indexing { of: u64 },
    Ready(Arc<ModuleIndex>),
    Failed(String),
}

/// One module's state; `job` is the job that set it, so a job whose module
/// was since forgotten or queued again publishes nothing.
struct Entry {
    key: ModuleKey,
    state: EntryState,
    job: u64,
}

/// The module being indexed.
struct Current {
    key: ModuleKey,
}

struct Shared {
    entries: Mutex<Vec<Entry>>,
    current: Mutex<Option<Current>>,
    /// Stops the job being indexed.
    cancel: AtomicBool,
    /// Bytes of the job being indexed done so far.
    progress: AtomicU64,
    /// Jobs numbered below this were cancelled.
    cancelled_below: AtomicU64,
    next_job: AtomicU64,
}

/// A queued job and its number.
struct Numbered {
    job: u64,
    index: IndexJob,
}

enum Coordinator {
    NotStarted,
    Running(Sender<Numbered>),
    /// No thread could be started (the browser build): jobs are indexed on
    /// the thread that hands them over.
    Inline,
}

/// A VM's cross-reference indexes and the thread that builds them.
pub(crate) struct XrefService {
    shared: Arc<Shared>,
    coordinator: Mutex<Coordinator>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// Sets `key`'s state, as job `job` left it.
    fn set(&self, key: &ModuleKey, state: EntryState, job: u64) {
        let mut entries = locked(&self.entries);
        match entries.iter_mut().find(|entry| entry.key == *key) {
            Some(entry) => {
                entry.state = state;
                entry.job = job;
            }
            None => entries.push(Entry { key: key.clone(), state, job }),
        }
    }

    /// Whether job `job` still owns `key`'s entry.
    fn owns(&self, key: &ModuleKey, job: u64) -> bool {
        locked(&self.entries).iter().any(|entry| entry.key == *key && entry.job == job)
    }

    /// Indexes one job on `workers`, and publishes what came of it.
    fn index(&self, numbered: Numbered, workers: Workers) {
        let Numbered { job, index } = numbered;
        self.cancel.store(false, Ordering::Relaxed);
        if job < self.cancelled_below.load(Ordering::Relaxed) || !self.owns(&index.key, job) {
            self.drop_job(&index.key, job);
            return;
        }
        self.progress.store(0, Ordering::Relaxed);
        self.set(&index.key, EntryState::Indexing { of: index.image.bytes.len() as u64 }, job);
        *locked(&self.current) = Some(Current { key: index.key.clone() });
        let work = IndexWork { cancel: &self.cancel, progress: &self.progress, workers, modules: &index.spans };
        let outcome = build_index(&index.image, &index.extra_sync, index.taken_at, &work);
        *locked(&self.current) = None;
        match outcome {
            Ok(built) if self.owns(&index.key, job) => self.set(&index.key, EntryState::Ready(Arc::new(built)), job),
            Ok(_) | Err(IndexEnd::Cancelled) => self.drop_job(&index.key, job),
            Err(end) if self.owns(&index.key, job) => self.set(&index.key, EntryState::Failed(end.describe()), job),
            Err(_) => {}
        }
    }

    /// Removes `key`'s entry when job `job` still owns it.
    fn drop_job(&self, key: &ModuleKey, job: u64) {
        locked(&self.entries).retain(|entry| !(entry.key == *key && entry.job == job));
    }
}

/// The coordinator: indexes each job handed over, in order, until the
/// service is dropped.
fn coordinate(shared: &Shared, jobs: Receiver<Numbered>) {
    while let Ok(numbered) = jobs.recv() {
        shared.index(numbered, Workers::for_this_machine());
    }
}

impl XrefService {
    pub(crate) fn new() -> XrefService {
        XrefService {
            shared: Arc::new(Shared {
                entries: Mutex::new(Vec::new()),
                current: Mutex::new(None),
                cancel: AtomicBool::new(false),
                progress: AtomicU64::new(0),
                cancelled_below: AtomicU64::new(0),
                next_job: AtomicU64::new(0),
            }),
            coordinator: Mutex::new(Coordinator::NotStarted),
        }
    }

    /// Queues a copied image for indexing (the run thread's side).
    pub(crate) fn accept(&self, index: IndexJob) {
        let job = self.shared.next_job.fetch_add(1, Ordering::Relaxed);
        self.shared.set(&index.key, EntryState::Queued, job);
        let numbered = Numbered { job, index };
        let mut coordinator = locked(&self.coordinator);
        let unsent = match &*coordinator {
            Coordinator::Running(sender) => match sender.send(numbered) {
                Ok(()) => return,
                Err(SendError(unsent)) => unsent,
            },
            Coordinator::NotStarted => {
                let (sender, receiver) = channel();
                let shared = Arc::clone(&self.shared);
                let spawned = std::thread::Builder::new()
                    .name("rusty_box_xrefs".to_owned())
                    .spawn(move || coordinate(&shared, receiver));
                match spawned {
                    // The coordinator ends when the service, and with it the
                    // sender, is dropped; it is not joined.
                    Ok(handle) => {
                        drop(handle);
                        match sender.send(numbered) {
                            Ok(()) => {
                                *coordinator = Coordinator::Running(sender);
                                return;
                            }
                            Err(SendError(unsent)) => unsent,
                        }
                    }
                    Err(_) => numbered,
                }
            }
            Coordinator::Inline => numbered,
        };
        *coordinator = Coordinator::Inline;
        drop(coordinator);
        self.shared.index(unsent, Workers(1));
    }

    /// Marks `keys` as being copied (the shell's side, when it asks for them).
    pub(crate) fn mark_copying(&self, keys: &[ModuleKey]) {
        for key in keys {
            self.shared.set(key, EntryState::Copying, u64::MAX);
        }
    }

    /// Records that `key` could not be copied, and why.
    pub(crate) fn fail(&self, key: ModuleKey, reason: String) {
        self.shared.set(&key, EntryState::Failed(reason), u64::MAX);
    }

    pub(crate) fn state(&self, key: &ModuleKey) -> Option<IndexState> {
        let entries = locked(&self.shared.entries);
        let entry = entries.iter().find(|entry| entry.key == *key)?;
        Some(match &entry.state {
            EntryState::Copying => IndexState::Copying,
            EntryState::Queued => IndexState::Queued,
            EntryState::Indexing { of } => {
                IndexState::Indexing { done: self.shared.progress.load(Ordering::Relaxed).min(*of), of: *of }
            }
            EntryState::Ready(index) => IndexState::Ready(Arc::clone(index)),
            EntryState::Failed(reason) => IndexState::Failed(reason.clone()),
        })
    }

    /// Stops the job being indexed and drops every queued one; their modules
    /// are no longer indexed or waiting.
    pub(crate) fn cancel(&self) {
        self.shared.cancelled_below.store(self.shared.next_job.load(Ordering::Relaxed), Ordering::Relaxed);
        self.shared.cancel.store(true, Ordering::Relaxed);
        locked(&self.shared.entries)
            .retain(|entry| !matches!(entry.state, EntryState::Copying | EntryState::Queued | EntryState::Indexing { .. }));
    }

    /// Drops `keys`' indexes (Reindex), stopping the job indexing one of them.
    pub(crate) fn forget(&self, keys: &[ModuleKey]) {
        locked(&self.shared.entries).retain(|entry| !keys.contains(&entry.key));
        if locked(&self.shared.current).as_ref().is_some_and(|current| keys.contains(&current.key)) {
            self.shared.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The references to `query`'s target in the indexes of `keys`.
    pub(crate) fn answer(&self, query: &XrefQuery, keys: &[ModuleKey]) -> XrefAnswer {
        let mut ready: Vec<ReadyIndex> = Vec::new();
        let mut waiting = Vec::new();
        let mut failed = Vec::new();
        {
            let entries = locked(&self.shared.entries);
            for key in keys {
                match entries.iter().find(|entry| entry.key == *key).map(|entry| &entry.state) {
                    Some(EntryState::Ready(index)) => ready.push(ReadyIndex { module: key.name.clone(), index: Arc::clone(index) }),
                    Some(EntryState::Failed(reason)) => failed.push(format!("{}: {reason}", key.name)),
                    Some(EntryState::Copying | EntryState::Queued | EntryState::Indexing { .. }) | None => {
                        waiting.push(key.name.clone());
                    }
                }
            }
        }
        let mut found = Vec::new();
        for module in &ready {
            for range in &query.target.ranges {
                for site in sites_to(&module.index, *range) {
                    let through = match site.kind {
                        XrefKind::Pointer => readers_of(&ready, site.site),
                        _ => Vec::new(),
                    };
                    found.push(FoundSite { site: site.site, kind: site.kind, module: module.module.clone(), through });
                }
            }
        }
        found.sort_by_key(|found| found.site);
        found.dedup_by(|a, b| a.site == b.site && a.kind == b.kind);
        XrefAnswer {
            query: query.clone(),
            found,
            waiting,
            failed,
            holes: ready.iter().map(|module| module.index.holes).fold(0, u32::saturating_add),
            taken_at: ready.iter().map(|module| module.index.taken_at).min(),
        }
    }
}

impl Drop for XrefService {
    /// A VM that goes away stops its indexing at once.
    fn drop(&mut self) {
        self.shared.cancelled_below.store(u64::MAX, Ordering::Relaxed);
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}

/// An index an answer reads, and its module's name.
struct ReadyIndex {
    module: String,
    index: Arc<ModuleIndex>,
}

/// The sites of `index` whose target lies in `range`.
fn sites_to(index: &ModuleIndex, range: AddressRange) -> &[Site] {
    let first = index.sites.partition_point(|site| site.target < range.start);
    let last = index.sites.partition_point(|site| site.target < range.end);
    index.sites.get(first..last.max(first)).unwrap_or(&[])
}

/// The code in `ready` that reads, or calls or jumps through, the pointer at
/// `address`.
fn readers_of(ready: &[ReadyIndex], address: u64) -> Vec<FoundSite> {
    let range = AddressRange { start: address, end: address.saturating_add(1) };
    let mut readers = Vec::new();
    for module in ready {
        for site in sites_to(&module.index, range) {
            if matches!(site.kind, XrefKind::CallThrough | XrefKind::JumpThrough | XrefKind::Memory) {
                readers.push(FoundSite { site: site.site, kind: site.kind, module: module.module.clone(), through: Vec::new() });
            }
        }
    }
    readers.sort_by_key(|reader| reader.site);
    readers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::module_identity::GuestIdentity;
    use crate::debugger::xref::decode::XrefKind;
    use crate::debugger::xref::index::ModuleSpan;
    use crate::debugger::xref::test_image::{module_image, ImageSpec};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const A: u64 = 0x1_4000_0000;
    const B: u64 = 0x1_8000_0000;
    const F: u64 = 0x1100;

    fn key(name: &str, base: u64, size: u64) -> ModuleKey {
        ModuleKey { identity: GuestIdentity::Unnamed, name: name.to_owned(), base, size, space: IndexSpace::Kernel }
    }

    fn rel32(next: u64, to: u64) -> [u8; 4] {
        (to.wrapping_sub(next) as u32).to_le_bytes()
    }

    /// Module B: `call f` at its `.text` start, `f: ret` at 0x1100.
    fn module_b() -> ModuleImage {
        let spec = ImageSpec::bits64(0x1000);
        let mut text = vec![0xE8];
        text.extend_from_slice(&rel32(0x1005, F));
        text.resize(0x100, 0x90);
        text.push(0xC3);
        module_image(B, &spec, &text, &[], &[0x1000])
    }

    /// Module A: `call [rip+slot]` at its `.text` start, the slot holding
    /// B's `f`.
    fn module_a() -> ModuleImage {
        let spec = ImageSpec::bits64(0x1000);
        let slot = u64::from(spec.data_free());
        let mut text = vec![0xFF, 0x15];
        text.extend_from_slice(&rel32(0x1006, slot));
        module_image(A, &spec, &text, &(B + F).to_le_bytes(), &[0x1000])
    }

    fn size_of(image: &ModuleImage) -> u64 {
        image.bytes.len() as u64
    }

    fn spans() -> Arc<[ModuleSpan]> {
        let a = module_a();
        let b = module_b();
        Arc::from(vec![ModuleSpan { base: A, size: size_of(&a) }, ModuleSpan { base: B, size: size_of(&b) }])
    }

    fn job(key: ModuleKey, image: ModuleImage) -> IndexJob {
        IndexJob { key, image, extra_sync: Vec::new(), spans: spans(), taken_at: 42 }
    }

    /// Waits for `key` to leave `Queued` and `Indexing`.
    fn settled(service: &XrefService, key: &ModuleKey) -> Option<IndexState> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = service.state(key);
            match state {
                Some(IndexState::Queued | IndexState::Copying | IndexState::Indexing { .. }) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                other => return other,
            }
        }
    }

    /// A service holding both modules' indexes, and their keys.
    struct Both {
        service: XrefService,
        keys: Vec<ModuleKey>,
    }

    fn both() -> Both {
        let service = XrefService::new();
        let a = module_a();
        let b = module_b();
        let keys = vec![key("a.sys", A, size_of(&a)), key("b.sys", B, size_of(&b))];
        service.accept(job(keys[0].clone(), a));
        service.accept(job(keys[1].clone(), b));
        for key in &keys {
            assert!(matches!(settled(&service, key), Some(IndexState::Ready(_))), "{} indexed", key.name);
        }
        Both { service, keys }
    }

    fn query_f(scope: XrefScope) -> XrefQuery {
        XrefQuery { target: XrefTarget { ranges: vec![AddressRange { start: B + F, end: B + F + 1 }], label: "b!f".into() }, scope }
    }

    #[test]
    fn an_answer_lists_calls_and_jumps_from_every_module_in_scope() {
        let Both { service, keys } = both();
        let answer = service.answer(&query_f(XrefScope::AllModules), &keys);
        let sites: Vec<u64> = answer.found.iter().map(|found| found.site).collect();
        assert_eq!(sites, [A + u64::from(ImageSpec::bits64(0x1000).data_free()), B + 0x1000]);
        assert_eq!(answer.found[0].kind, XrefKind::Pointer);
        assert_eq!(answer.found[0].module, "a.sys");
        assert_eq!(answer.found[1].kind, XrefKind::Call);
        assert_eq!(answer.found[1].module, "b.sys");
        assert!(answer.waiting.is_empty() && answer.failed.is_empty());
    }

    #[test]
    fn a_pointer_lists_the_code_that_reads_it() {
        let Both { service, keys } = both();
        let answer = service.answer(&query_f(XrefScope::AllModules), &keys);
        let through: Vec<FoundSite> = answer.found[0].through.clone();
        assert_eq!(
            through,
            [FoundSite { site: A + 0x1000, kind: XrefKind::CallThrough, module: "a.sys".into(), through: Vec::new() }]
        );
    }

    #[test]
    fn the_target_module_scope_asks_only_its_keys() {
        let Both { service, keys } = both();
        let answer = service.answer(&query_f(XrefScope::TargetModule), &keys[1..]);
        assert_eq!(answer.found.len(), 1);
        assert_eq!(answer.found[0].kind, XrefKind::Call);
    }

    #[test]
    fn a_module_still_indexing_is_named_as_waiting() {
        let service = XrefService::new();
        let unknown = key("c.sys", 0x2_0000_0000, 0x3000);
        service.mark_copying(std::slice::from_ref(&unknown));
        let answer = service.answer(&query_f(XrefScope::AllModules), &[unknown]);
        assert_eq!(answer.waiting, ["c.sys"]);
        assert!(answer.found.is_empty());
    }

    #[test]
    fn an_index_for_a_moved_module_is_not_used() {
        let Both { service, keys } = both();
        let moved = key("b.sys", B + 0x10_0000, keys[1].size);
        let answer = service.answer(&query_f(XrefScope::TargetModule), &[moved]);
        assert_eq!(answer.waiting, ["b.sys"]);
        assert!(answer.found.is_empty());
    }

    #[test]
    fn a_failed_copy_is_named_with_its_reason() {
        let service = XrefService::new();
        let broken = key("d.sys", 0x3_0000_0000, 0x3000);
        service.fail(broken.clone(), "its headers do not read".into());
        let answer = service.answer(&query_f(XrefScope::AllModules), &[broken]);
        assert_eq!(answer.failed, ["d.sys: its headers do not read"]);
    }

    #[test]
    fn a_cancelled_index_is_not_kept() {
        let service = XrefService::new();
        let size = 64 << 20;
        let spec = ImageSpec::bits64(size);
        let mut text = vec![0x90; size as usize];
        text[0] = 0xE8;
        let image = module_image(A, &spec, &text, &[], &[]);
        let big = key("big.sys", A, image.bytes.len() as u64);
        service.accept(job(big.clone(), image.clone()));
        service.cancel();
        let state = settled(&service, &big);
        assert!(!matches!(state, Some(IndexState::Ready(_))), "{state:?}");
        service.accept(job(big.clone(), image));
        assert!(matches!(settled(&service, &big), Some(IndexState::Ready(_))));
    }

    #[test]
    fn a_forgotten_module_is_not_answered_from_until_indexed_again() {
        let Both { service, keys } = both();
        service.forget(&keys[1..]);
        let answer = service.answer(&query_f(XrefScope::TargetModule), &keys[1..]);
        assert_eq!(answer.waiting, ["b.sys"]);
        assert!(answer.found.is_empty());
        service.accept(job(keys[1].clone(), module_b()));
        assert!(matches!(settled(&service, &keys[1]), Some(IndexState::Ready(_))));
        assert_eq!(service.answer(&query_f(XrefScope::TargetModule), &keys[1..]).found.len(), 1);
    }

    #[test]
    fn the_answer_says_when_the_index_was_taken() {
        let Both { service, keys } = both();
        let answer = service.answer(&query_f(XrefScope::AllModules), &keys);
        assert_eq!(answer.taken_at, Some(42));
        assert_eq!(answer.holes, 0);
    }
}
