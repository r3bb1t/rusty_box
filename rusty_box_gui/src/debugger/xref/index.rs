//! A module's cross-reference index, built in parallel (spec §4).
//!
//! Executable sections are cut into pieces at holes and at offsets known to
//! start an instruction; a piece is decoded linearly from its start, so
//! pieces are independent and any number of workers take them in any order.
//! A piece longer than `MAX_PIECE` is cut again at arbitrary offsets: the
//! later part is decoded from its cut, and the merge — in address order, on
//! one thread — continues the earlier part's stream until it reaches an
//! instruction start the later part also found, keeping the later part's
//! sites from there (two linear sweeps that share a start agree after it);
//! with no shared start in `CONVERGE_WINDOW` bytes it decodes the later part
//! again from the earlier stream. The index therefore equals one linear
//! sweep of each trusted piece, whatever the number of workers.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::decode::{code_size, references_of, XrefKind};
use super::image::{ModuleImage, PageRead, PAGE_BYTES};
use crate::debugger::pe::ImageWidth;

/// A piece past this is cut again.
pub(crate) const MAX_PIECE: u64 = 64 << 10;
/// How far past a cut the merge looks for a start both parts share.
pub(crate) const CONVERGE_WINDOW: u64 = 256;
/// An index past this many sites is refused.
pub(crate) const MAX_SITES: usize = 1 << 24;
const MAX_WORKERS: usize = 16;
/// The longest x86 instruction.
const MAX_INSTRUCTION_BYTES: u64 = 15;

/// One reference: `site` refers to `target` as `kind` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Site {
    pub(crate) target: u64,
    pub(crate) site: u64,
    pub(crate) kind: XrefKind,
}

/// Every reference a module's image holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleIndex {
    pub(crate) base: u64,
    pub(crate) size: u64,
    /// Sorted by target, then site.
    pub(crate) sites: Vec<Site>,
    /// Pages of the image that did not read.
    pub(crate) holes: u32,
    /// The instruction count of the pause the image was copied at.
    pub(crate) taken_at: u64,
}

/// How many threads index at once, the calling one included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Workers(pub(crate) usize);

impl Workers {
    /// Every core but one, which the VM's run thread and the UI keep; at least
    /// one, at most 16.
    pub(crate) fn for_this_machine() -> Workers {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Workers(cores.saturating_sub(1).clamp(1, MAX_WORKERS))
    }
}

/// A loaded module a data pointer may point into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ModuleSpan {
    pub(crate) base: u64,
    pub(crate) size: u64,
}

/// What an index is built with: the owner's cancel, the bytes done so far,
/// the workers, and the modules sorted by base.
pub(crate) struct IndexWork<'a> {
    pub(crate) cancel: &'a AtomicBool,
    pub(crate) progress: &'a AtomicU64,
    pub(crate) workers: Workers,
    pub(crate) modules: &'a [ModuleSpan],
}

/// Why an index was not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndexEnd {
    Cancelled,
    TooManySites,
    NoMemory,
    /// A worker thread stopped without its pieces.
    WorkerFailed,
}

impl IndexEnd {
    pub(crate) fn describe(self) -> String {
        match self {
            IndexEnd::Cancelled => "cancelled".to_owned(),
            IndexEnd::TooManySites => format!("more than {MAX_SITES} references"),
            IndexEnd::NoMemory => "there is no memory for the index".to_owned(),
            IndexEnd::WorkerFailed => "an indexing thread stopped".to_owned(),
        }
    }
}

/// How a code piece begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Begin {
    /// At an offset known to start an instruction, or after a hole.
    Trusted,
    /// At an arbitrary cut of a longer piece: the stream before it decides.
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PieceKind {
    Code(Begin),
    Pointers,
}

/// Image offsets `start..end` to index; `limit` ends the readable bytes an
/// instruction starting inside may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Piece {
    start: u64,
    end: u64,
    limit: u64,
    kind: PieceKind,
}

/// What one piece gave: its sites; for a split piece, the instruction starts
/// in its first `CONVERGE_WINDOW` bytes; and where its stream goes on after
/// its end.
#[derive(Clone, Debug, Default)]
struct PieceResult {
    sites: Vec<Site>,
    starts: Vec<u64>,
    next: u64,
}

/// A worker's finished piece.
struct Done {
    index: usize,
    result: PieceResult,
}

/// Image offsets `start..end` of readable bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    start: u64,
    end: u64,
}

/// The readable runs of `from..to`: pages that read, joined, clipped.
fn runs(image: &ModuleImage, from: u64, to: u64) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut at = from;
    while at < to {
        let page_end = at.saturating_add(PAGE_BYTES).saturating_sub(at.wrapping_rem(PAGE_BYTES)).min(to);
        let page = usize::try_from(at.wrapping_div(PAGE_BYTES)).ok();
        let read = page.and_then(|page| image.pages.get(page)).is_some_and(|page| *page == PageRead::Read);
        if read {
            match runs.last_mut() {
                Some(run) if run.end == at => run.end = page_end,
                Some(_) | None => runs.push(Run { start: at, end: page_end }),
            }
        }
        at = page_end;
    }
    runs
}

/// The trusted pieces and the pointer chunks of `image`, in address order:
/// executable sections cut at holes and at `sync`; other sections cut into
/// `MAX_PIECE` chunks.
fn trusted_pieces(image: &ModuleImage, sync: &[u64]) -> Vec<Piece> {
    let image_end = image.bytes.len() as u64;
    let mut pieces = Vec::new();
    for section in &image.sections {
        let start = u64::from(section.rva).min(image_end);
        let end = start.saturating_add(u64::from(section.virtual_size)).min(image_end);
        for run in runs(image, start, end) {
            if section.executable {
                let first = sync.partition_point(|offset| *offset <= run.start);
                let last = sync.partition_point(|offset| *offset < run.end);
                let mut piece_start = run.start;
                for cut in sync.get(first..last).unwrap_or(&[]) {
                    pieces.push(Piece { start: piece_start, end: *cut, limit: run.end, kind: PieceKind::Code(Begin::Trusted) });
                    piece_start = *cut;
                }
                pieces.push(Piece { start: piece_start, end: run.end, limit: run.end, kind: PieceKind::Code(Begin::Trusted) });
            } else {
                let mut chunk = run.start;
                while chunk < run.end {
                    let chunk_end = chunk.saturating_add(MAX_PIECE).min(run.end);
                    pieces.push(Piece { start: chunk, end: chunk_end, limit: run.end, kind: PieceKind::Pointers });
                    chunk = chunk_end;
                }
            }
        }
    }
    pieces.sort_unstable_by_key(|piece| piece.start);
    pieces
}

/// `pieces` with every code piece past `MAX_PIECE` cut into `MAX_PIECE`
/// parts, the later ones `Split`.
fn split_pieces(pieces: &[Piece]) -> Vec<Piece> {
    let mut split = Vec::new();
    for piece in pieces {
        let PieceKind::Code(_) = piece.kind else {
            split.push(*piece);
            continue;
        };
        let mut start = piece.start;
        let mut kind = piece.kind;
        loop {
            let end = start.saturating_add(MAX_PIECE).min(piece.end);
            split.push(Piece { start, end, limit: piece.limit, kind });
            if end >= piece.end {
                break;
            }
            start = end;
            kind = PieceKind::Code(Begin::Split);
        }
    }
    split
}

/// The sync points `image` and `extra` give, ascending, each once.
fn sync_offsets(image: &ModuleImage, extra: &[super::image::ImageOffset]) -> Vec<u64> {
    let mut sync: Vec<u64> = image.sync.iter().chain(extra).map(|offset| offset.0).filter(|offset| image.in_code(*offset)).collect();
    sync.sort_unstable();
    sync.dedup();
    sync
}

/// One instruction at `at` (bytes up to `limit`): its length and what it
/// refers to, recorded into `sites`. A byte that starts no instruction is
/// stepped over.
fn decode_at(image: &ModuleImage, at: u64, limit: u64, sites: &mut Vec<Site>) -> u64 {
    let end = at.saturating_add(MAX_INSTRUCTION_BYTES).min(limit);
    let Ok(start) = usize::try_from(at) else {
        return 1;
    };
    let Ok(end) = usize::try_from(end) else {
        return 1;
    };
    let Some(bytes) = image.bytes.get(start..end).filter(|bytes| !bytes.is_empty()) else {
        return 1;
    };
    let Ok(instruction) = code_size(image.width).decode(bytes) else {
        return 1;
    };
    let length = u64::from(instruction.ilen());
    if length == 0 || length > bytes.len() as u64 {
        return 1;
    }
    let address = image.base.wrapping_add(at);
    for reference in references_of(&instruction, address, image.width).iter() {
        sites.push(Site { target: reference.target, site: address, kind: reference.kind });
    }
    length
}

/// Decodes from `from` while the stream is before `end`; the starts below
/// `window` go to `starts`. Returns where the stream goes on.
fn sweep(image: &ModuleImage, from: u64, end: u64, limit: u64, window: u64, out: &mut PieceResult) -> u64 {
    let mut at = from;
    while at < end {
        if at < window {
            out.starts.push(at);
        }
        at = at.saturating_add(decode_at(image, at, limit, &mut out.sites));
    }
    at
}

/// The pointer-aligned values of `piece` that point into one of `modules`.
fn scan_pointers(image: &ModuleImage, piece: &Piece, modules: &[ModuleSpan], out: &mut PieceResult) {
    let width: u64 = match image.width {
        ImageWidth::Bits32 => 4,
        ImageWidth::Bits64 => 8,
    };
    let Some(mut at) = piece.start.checked_next_multiple_of(width) else {
        out.next = piece.end;
        return;
    };
    while at < piece.end && at.saturating_add(width) <= piece.limit {
        let value = usize::try_from(at).ok().and_then(|start| {
            let bytes = image.bytes.get(start..)?;
            match image.width {
                ImageWidth::Bits32 => bytes.first_chunk::<4>().map(|chunk| u64::from(u32::from_le_bytes(*chunk))),
                ImageWidth::Bits64 => bytes.first_chunk::<8>().map(|chunk| u64::from_le_bytes(*chunk)),
            }
        });
        if let Some(value) = value.filter(|value| points_into(modules, *value)) {
            out.sites.push(Site { target: value, site: image.base.wrapping_add(at), kind: XrefKind::Pointer });
        }
        at = at.saturating_add(width);
    }
    out.next = piece.end;
}

/// Whether `value` lies in one of `modules`, sorted by base.
fn points_into(modules: &[ModuleSpan], value: u64) -> bool {
    let after = modules.partition_point(|module| module.base <= value);
    after
        .checked_sub(1)
        .and_then(|index| modules.get(index))
        .is_some_and(|module| value.wrapping_sub(module.base) < module.size)
}

/// One piece, as a worker indexes it.
fn index_piece(image: &ModuleImage, piece: &Piece, modules: &[ModuleSpan]) -> PieceResult {
    let mut result = PieceResult::default();
    match piece.kind {
        PieceKind::Pointers => scan_pointers(image, piece, modules, &mut result),
        PieceKind::Code(begin) => {
            let window = match begin {
                Begin::Trusted => piece.start,
                Begin::Split => piece.start.saturating_add(CONVERGE_WINDOW),
            };
            result.next = sweep(image, piece.start, piece.end, piece.limit, window, &mut result);
        }
    }
    result
}

/// Indexes `pieces` on `work.workers` threads (this one among them), each
/// result in its piece's place.
fn index_pieces(image: &ModuleImage, pieces: &[Piece], work: &IndexWork<'_>) -> Result<Vec<PieceResult>, IndexEnd> {
    let next = AtomicUsize::new(0);
    let take = || {
        let mut done = Vec::new();
        loop {
            if work.cancel.load(Ordering::Relaxed) {
                return done;
            }
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(piece) = pieces.get(index) else {
                return done;
            };
            let result = index_piece(image, piece, work.modules);
            work.progress.fetch_add(piece.end.saturating_sub(piece.start), Ordering::Relaxed);
            done.push(Done { index, result });
        }
    };
    let workers = work.workers.0.clamp(1, MAX_WORKERS);
    let finished = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 1..workers {
            match std::thread::Builder::new().name("rusty_box_xref".to_owned()).spawn_scoped(scope, take) {
                Ok(handle) => handles.push(handle),
                // A thread that does not start leaves its pieces to the rest.
                Err(_) => break,
            }
        }
        let mut finished = take();
        for handle in handles {
            match handle.join() {
                Ok(done) => finished.extend(done),
                Err(_) => return Err(IndexEnd::WorkerFailed),
            }
        }
        Ok(finished)
    })?;
    if work.cancel.load(Ordering::Relaxed) {
        return Err(IndexEnd::Cancelled);
    }
    let mut results: Vec<Option<PieceResult>> = Vec::new();
    results.resize_with(pieces.len(), || None);
    for done in finished {
        if let Some(slot) = results.get_mut(done.index) {
            *slot = Some(done.result);
        }
    }
    results.into_iter().map(|result| result.ok_or(IndexEnd::Cancelled)).collect()
}

/// The pieces' sites in one stream: a split piece's from where the stream
/// before it meets one of its starts, or decoded again from that stream.
fn merge(image: &ModuleImage, pieces: &[Piece], results: Vec<PieceResult>) -> Vec<Vec<Site>> {
    let mut merged = Vec::new();
    let mut stream = 0u64;
    let mut results = results.into_iter();
    for piece in pieces {
        let Some(result) = results.next() else {
            break;
        };
        match piece.kind {
            PieceKind::Pointers => merged.push(result.sites),
            PieceKind::Code(Begin::Trusted) => {
                stream = result.next;
                merged.push(result.sites);
            }
            PieceKind::Code(Begin::Split) => {
                let window = piece.start.saturating_add(CONVERGE_WINDOW);
                let mut redone = PieceResult::default();
                let mut at = stream.max(piece.start);
                loop {
                    if at >= piece.end {
                        stream = at;
                        merged.push(redone.sites);
                        break;
                    }
                    if result.starts.binary_search(&at).is_ok() {
                        let from = image.base.wrapping_add(at);
                        redone.sites.extend(result.sites.iter().filter(|site| site.site >= from));
                        stream = result.next;
                        merged.push(redone.sites);
                        break;
                    }
                    if at >= window {
                        stream = sweep(image, at, piece.end, piece.limit, at, &mut redone);
                        merged.push(redone.sites);
                        break;
                    }
                    at = at.saturating_add(decode_at(image, at, piece.limit, &mut redone.sites));
                }
            }
        }
    }
    merged
}

/// Gathers `parts` into one sorted list, or says why it cannot.
fn gather(parts: Vec<Vec<Site>>) -> Result<Vec<Site>, IndexEnd> {
    let total = parts.iter().map(Vec::len).fold(0usize, usize::saturating_add);
    if total > MAX_SITES {
        return Err(IndexEnd::TooManySites);
    }
    let mut sites = Vec::new();
    sites.try_reserve_exact(total).map_err(|_| IndexEnd::NoMemory)?;
    for part in parts {
        sites.extend(part);
    }
    sites.sort_unstable();
    Ok(sites)
}

fn holes(image: &ModuleImage) -> u32 {
    let count = image.pages.iter().filter(|page| **page == PageRead::Hole).count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The index of `image`, its pieces taken by `work.workers` threads. `extra`
/// adds sync points (the module's PDB procedures); `taken_at` is the pause's
/// instruction count.
pub(crate) fn build_index(
    image: &ModuleImage,
    extra: &[super::image::ImageOffset],
    taken_at: u64,
    work: &IndexWork<'_>,
) -> Result<ModuleIndex, IndexEnd> {
    if work.cancel.load(Ordering::Relaxed) {
        return Err(IndexEnd::Cancelled);
    }
    let pieces = split_pieces(&trusted_pieces(image, &sync_offsets(image, extra)));
    let results = index_pieces(image, &pieces, work)?;
    let sites = gather(merge(image, &pieces, results))?;
    Ok(ModuleIndex { base: image.base, size: image.bytes.len() as u64, sites, holes: holes(image), taken_at })
}

/// The reference the parallel index must equal: each trusted piece swept in
/// order on this thread.
#[cfg(test)]
pub(crate) fn build_index_in_order(
    image: &ModuleImage,
    extra: &[super::image::ImageOffset],
    taken_at: u64,
    modules: &[ModuleSpan],
) -> Result<ModuleIndex, IndexEnd> {
    let pieces = trusted_pieces(image, &sync_offsets(image, extra));
    let parts = pieces
        .iter()
        .map(|piece| {
            let mut result = PieceResult::default();
            match piece.kind {
                PieceKind::Pointers => scan_pointers(image, piece, modules, &mut result),
                PieceKind::Code(_) => {
                    result.next = sweep(image, piece.start, piece.end, piece.limit, piece.start, &mut result);
                }
            }
            result.sites
        })
        .collect();
    let sites = gather(parts)?;
    Ok(ModuleIndex { base: image.base, size: image.bytes.len() as u64, sites, holes: holes(image), taken_at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::xref::decode::XrefKind;
    use crate::debugger::xref::image::{ModuleImage, PageRead};
    use crate::debugger::xref::test_image::{module_image, ImageSpec};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    const BASE: u64 = 0x1_4000_0000;
    const TAKEN_AT: u64 = 7;

    struct Run {
        cancel: AtomicBool,
        progress: AtomicU64,
        modules: Vec<ModuleSpan>,
    }

    impl Run {
        fn over(image: &ModuleImage) -> Run {
            Run {
                cancel: AtomicBool::new(false),
                progress: AtomicU64::new(0),
                modules: vec![ModuleSpan { base: image.base, size: image.bytes.len() as u64 }],
            }
        }

        fn work(&self, workers: usize) -> IndexWork<'_> {
            IndexWork { cancel: &self.cancel, progress: &self.progress, workers: Workers(workers), modules: &self.modules }
        }
    }

    fn index(image: &ModuleImage, workers: usize) -> ModuleIndex {
        let run = Run::over(image);
        build_index(image, &[], TAKEN_AT, &run.work(workers)).expect("an index")
    }

    /// `len` bytes from a linear congruential generator seeded `seed`.
    fn noise(seed: u64, len: usize) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                (state >> 56) as u8
            })
            .collect()
    }

    fn rel32(from_next: u64, to: u64) -> [u8; 4] {
        (to.wrapping_sub(from_next) as u32).to_le_bytes()
    }

    #[test]
    fn calls_jumps_and_a_data_pointer_are_indexed_by_target() {
        let spec = ImageSpec::bits64(0x1000);
        let f = 0x1100u64;
        let g = u64::from(spec.data_free()) + 8;
        let mut text = Vec::new();
        text.push(0xE8);
        text.extend_from_slice(&rel32(0x1005, f));
        text.push(0xE9);
        text.extend_from_slice(&rel32(0x100A, f));
        text.extend_from_slice(&[0x48, 0x8D, 0x0D]);
        text.extend_from_slice(&rel32(0x1011, g));
        text.resize(0x100, 0x90);
        text.push(0xC3);
        let data = (BASE + f).to_le_bytes();
        let image = module_image(BASE, &spec, &text, &data, &[0x1000]);
        let index = index(&image, 4);
        let to_f: Vec<Site> = index.sites.iter().filter(|site| site.target == BASE + f).copied().collect();
        assert_eq!(
            to_f,
            [
                Site { target: BASE + f, site: BASE + 0x1000, kind: XrefKind::Call },
                Site { target: BASE + f, site: BASE + 0x1005, kind: XrefKind::Jump },
                Site { target: BASE + f, site: BASE + u64::from(spec.data_free()), kind: XrefKind::Pointer },
            ]
        );
        assert!(index.sites.contains(&Site { target: BASE + g, site: BASE + 0x100A, kind: XrefKind::AddressOf }));
        assert_eq!(index.taken_at, TAKEN_AT);
        assert_eq!(index.holes, 0);
        assert!(index.sites.windows(2).all(|pair| pair[0] <= pair[1]), "sorted by target, then site");
    }

    #[test]
    fn the_parallel_index_equals_the_one_thread_sweep() {
        for seed in 1..=64u64 {
            let spec = ImageSpec::bits64(0x10_0000);
            let text = noise(seed, 0x10_0000);
            let sync: Vec<u64> = noise(seed ^ 0xFF, 8).iter().map(|byte| 0x1000 + u64::from(*byte) * 0x1000).collect();
            let mut sync = sync;
            sync.sort_unstable();
            sync.dedup();
            let image = module_image(BASE, &spec, &text, &[], &sync);
            let parallel = index(&image, 8);
            let in_order = build_index_in_order(&image, &[], TAKEN_AT, &Run::over(&image).modules).expect("an index");
            assert_eq!(parallel.sites.len(), in_order.sites.len(), "seed {seed}");
            assert!(parallel.sites == in_order.sites, "seed {seed}: the parallel index differs");
        }
    }

    /// `E8` repeated: from any start, every five bytes are a `call`, so the
    /// streams of two starts that differ by other than a multiple of five
    /// never meet, and the merge must decode the split piece again.
    #[test]
    fn a_split_that_never_converges_is_decoded_again_in_order() {
        let spec = ImageSpec::bits64(200 << 10);
        let text = vec![0xE8; 200 << 10];
        let image = module_image(BASE, &spec, &text, &[], &[]);
        let parallel = index(&image, 8);
        let in_order = build_index_in_order(&image, &[], TAKEN_AT, &Run::over(&image).modules).expect("an index");
        assert!(!parallel.sites.is_empty());
        assert!(parallel.sites == in_order.sites);
        assert!(parallel.sites.iter().all(|site| (site.site - BASE - 0x1000) % 5 == 0), "one stream, from .text's start");
    }

    #[test]
    fn a_hole_ends_a_piece_and_is_counted() {
        let spec = ImageSpec::bits64(0x3000);
        let mut text = vec![0x90; 0x3000];
        // An `E8` on the last byte before the hole has no room for its
        // displacement; a whole call starts the page after it.
        text[0x0FFF] = 0xE8;
        text[0x2000] = 0xE8;
        text[0x2001..0x2005].copy_from_slice(&rel32(0x3005, 0x1000));
        let mut image = module_image(BASE, &spec, &text, &[], &[]);
        image.pages[2] = PageRead::Hole;
        image.bytes[0x2000..0x3000].fill(0);
        let index = index(&image, 4);
        assert_eq!(index.holes, 1);
        assert!(index.sites.iter().all(|site| site.site != BASE + 0x1FFF));
        assert!(index.sites.iter().all(|site| !(BASE + 0x2000..BASE + 0x3000).contains(&site.site)));
        assert!(index.sites.contains(&Site { target: BASE + 0x1000, site: BASE + 0x3000, kind: XrefKind::Call }));
    }

    #[test]
    fn cancelling_stops_the_index() {
        let spec = ImageSpec::bits64(0x10_0000);
        let image = module_image(BASE, &spec, &noise(3, 0x10_0000), &[], &[]);
        let run = Run::over(&image);
        run.cancel.store(true, Ordering::Relaxed);
        assert_eq!(build_index(&image, &[], TAKEN_AT, &run.work(4)).err(), Some(IndexEnd::Cancelled));

        let size = 64 << 20;
        let spec = ImageSpec::bits64(size);
        let image = module_image(BASE, &spec, &noise(4, size as usize), &[], &[]);
        let run = Run::over(&image);
        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                while run.progress.load(Ordering::Relaxed) == 0 {
                    std::thread::yield_now();
                }
                run.cancel.store(true, Ordering::Relaxed);
            });
            build_index(&image, &[], TAKEN_AT, &run.work(4))
        });
        assert_eq!(outcome.err(), Some(IndexEnd::Cancelled));
        assert!(run.progress.load(Ordering::Relaxed) < u64::from(size));
    }

    #[test]
    fn one_worker_and_sixteen_agree() {
        let spec = ImageSpec::bits64(0x8_0000);
        let image = module_image(BASE, &spec, &noise(9, 0x8_0000), &[], &[0x1000, 0x4_0000]);
        assert!(index(&image, 1).sites == index(&image, 16).sites);
    }

    #[test]
    fn the_worker_count_leaves_a_core_free() {
        let workers = Workers::for_this_machine().0;
        assert!((1..=16).contains(&workers));
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        if cores > 1 {
            assert!(workers < cores);
        }
    }
}
