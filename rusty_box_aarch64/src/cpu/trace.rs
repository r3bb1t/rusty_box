//! The trace cache: straight-line runs of pre-decoded instructions, keyed by
//! the physical address of their first, executed without a fetch, a
//! translation or a decode per instruction.
//!
//! Ported from Bochs's design (cpu/icache.h `bxTraceCache_c`,
//! `bxPageWriteStampTable`; cpu/icache.cc `serveTraceCacheMiss`,
//! `handleSMC`; cpu/cpu.cc `cpu_loop`, `getTraceCacheEntry`), as rusty_box's
//! x86 interpreter ports it (`rusty_box/src/cpu/icache.rs`), with A64's
//! fixed-length instructions in place of x86's:
//!
//! - A trace is at most [`MAX_TRACE`] instructions (Bochs
//!   `BX_MAX_TRACE_LENGTH`), all in one 4 KiB page, and ends after an
//!   instruction [`Op::ends_trace`] names. Entries are direct-mapped by
//!   physical address (Bochs `bxTraceCache_c::hash`), and the ops live in a
//!   pool that is emptied whole when it fills (`alloc_trace`).
//! - Self-modifying code is caught by the page write stamps: each page that
//!   holds a cached instruction has a bit per 128-byte line that does
//!   (`markICacheMask`). Every store the processor makes checks them
//!   (`Cpu::note_store`), and so does every write that does not come from
//!   it — a device's DMA, a host's — through `Cpu::forget_code`; a write to
//!   a marked line forgets every trace on it (`handleSMC`) — as QEMU
//!   invalidates the translation blocks of a page on any write to it, a
//!   device's DMA included (accel/tcg/tb-maint.c
//!   `tb_invalidate_phys_range`, which system/physmem.c calls after a write
//!   to guest RAM). Pages share a stamp when their numbers agree modulo
//!   [`STAMPS`] (Bochs shares them above 4 GiB): that only ever forgets
//!   more.
//! - Only memory whose fetches have no side effect and return what the
//!   last write left is cached: the bus says which ([`FetchSource`]). Any
//!   other fetch runs the per-instruction path.
//!
//! A trace runs only as far as the per-instruction path would run without
//! looking at anything outside the processor: the caller bounds it by the
//! instructions left before its budget, the next event and its exits, and it
//! ends early after any instruction that may have changed what the
//! processor would look at between instructions — a device access, a timer
//! register write, a TLB fill or flush (the next fetch translates again), a
//! write over cached code, or a cache or TLB maintenance instruction. Every
//! instruction in it is fetched, translated, permission-checked, executed
//! and reported to the observer exactly as `Cpu::step` would, so a machine
//! runs the same instructions with the same results either way.

use super::decode::classify;
use super::exec::{Done, Observed, Step};
use super::op::{predecode, Op};
use super::Cpu;
use crate::bus::{CodeEpoch, FetchSource, PhysAddr, PhysBus, PhysSpan};
use crate::instrumentation::Instrumentation;

/// The longest trace, in instructions (Bochs `BX_MAX_TRACE_LENGTH`).
pub(crate) const MAX_TRACE: usize = 32;

/// The cache's sizes. Either way the cache changes nothing but speed.
///
/// With `alloc` the tables are on the heap. The pool and the entries are
/// sized as rusty_box's x86 interpreter sizes its own
/// (`rusty_box/src/cpu/icache.rs`): Bochs's pool (icache.h
/// `BxTraceCacheMemPool`) and 64 Ki entries. A Linux boot runs about
/// 94,000 distinct instructions, and traces that start mid-run hold some of
/// them twice. A pool a ninth of this size, with a quarter of the entries,
/// empties 202 times in an Alpine boot and builds 5.4 times the traces
/// (1,177,959 against 217,178), 2.9% more host instructions for the whole
/// boot: A64's fixed-length words make a build cheap, about 2,100 host
/// instructions, where x86's decode is not. The pool is only reserved until
/// it is used (see [`Pool`]), so its 13.5 MiB cost a machine nothing it
/// does not run.
///
/// The page write stamps cover 256 MiB of pages, not Bochs's 4 GiB
/// (`bxPageWriteStampTable`): the stamps are read at random and must exist
/// zeroed from the start, and 4 MiB of them made every machine's build cost
/// six times as much in a test that builds thousands. They measured no
/// faster for a boot in the board's default 256 MiB of RAM (Alpine 0.96×).
/// Above that, pages share stamps, which only ever forgets more.
///
/// Without `alloc` the tables are inline in the processor, which a machine
/// holds and moves by value, and much smaller.
#[cfg(feature = "alloc")]
mod size {
    /// Trace entries: a power of two, so that the entries of one page are
    /// one contiguous window of 1,024.
    pub(super) const ENTRIES: usize = 65536;
    /// Ops in the pool (Bochs icache.h `BxTraceCacheMemPool`).
    pub(super) const POOL: usize = 576 * 1024;
    /// Page write stamps: a power of two, one per page of 256 MiB.
    pub(super) const STAMPS: usize = 65536;
}
#[cfg(not(feature = "alloc"))]
mod size {
    pub(super) const ENTRIES: usize = 4096;
    pub(super) const POOL: usize = 16384;
    pub(super) const STAMPS: usize = 8192;
}
use size::{ENTRIES, POOL, STAMPS};

const INVALID: u64 = u64::MAX;

// Pages that share a stamp must share their window of entries, or a write
// to one would clear the stamp of the other's traces without forgetting
// them (Bochs icache.h relies on the same agreement between its hashes).
const _: () = assert!(ENTRIES.is_power_of_two() && ENTRIES >= 1024 && (STAMPS * 1024).is_multiple_of(ENTRIES));
const _: () = assert!(STAMPS.is_power_of_two() && POOL >= MAX_TRACE && POOL < 1 << 24);

/// One of the cache's tables: on the heap with `alloc`, so that the
/// processor stays small enough to be moved by value; inline without. The
/// one shape, chosen at compile time (R0's representation alias).
#[cfg(feature = "alloc")]
type Table<T, const N: usize> = alloc::boxed::Box<[T]>;
#[cfg(not(feature = "alloc"))]
type Table<T, const N: usize> = [T; N];

/// A table of `N` copies of `fill`, built where it will live: a heap
/// table never passes through the stack.
#[cfg(feature = "alloc")]
fn table<T: Copy, const N: usize>(fill: T) -> Table<T, N> {
    alloc::vec![fill; N].into_boxed_slice()
}
#[cfg(not(feature = "alloc"))]
const fn table<T: Copy, const N: usize>(fill: T) -> Table<T, N> {
    [fill; N]
}

/// One cached trace.
#[derive(Debug, Clone, Copy)]
struct Entry {
    /// The physical address of its first instruction; [`INVALID`] for none.
    pa: u64,
    /// The epoch of the memory it was read from, when it was read.
    epoch: CodeEpoch,
    /// The cache generation it was built in: a flush moves the generation
    /// on, which forgets every entry at once.
    generation: u32,
    /// Where its ops start in the pool, above its length in instructions
    /// (1 to [`MAX_TRACE`]) in the low byte.
    at: u32,
}

const _: () = assert!(core::mem::size_of::<Entry>() == 24);

const EMPTY: Entry = Entry {
    pa: INVALID,
    epoch: CodeEpoch::new(0),
    generation: 0,
    at: 0,
};

/// Where a trace's ops are in the pool.
#[derive(Debug, Clone, Copy)]
struct Trace {
    start: usize,
    len: usize,
}

/// One instruction of a trace: its op, and the word it was decoded from,
/// for the observer.
#[derive(Debug, Clone, Copy)]
struct Slot {
    op: Op,
    word: u32,
}

/// The ops of every trace, filled from the front and emptied whole
/// (Bochs icache.h `bxICache_c::mpool`, `alloc_trace`), never past
/// [`POOL`]. With `alloc` it is a vector reserved to [`POOL`] up front and
/// grown into, so that a processor touches only the pool it uses: building
/// one costs nothing for the pool, as a test that builds thousands of
/// machines needs. Without, it is inline (R0's representation alias, as
/// [`Table`] is).
#[cfg(feature = "alloc")]
struct Pool {
    slots: alloc::vec::Vec<Slot>,
}
#[cfg(not(feature = "alloc"))]
struct Pool {
    slots: [Slot; POOL],
    len: usize,
}

#[cfg(feature = "alloc")]
impl Pool {
    fn new() -> Self {
        Self {
            slots: alloc::vec::Vec::with_capacity(POOL),
        }
    }

    /// The first free slot.
    #[inline(always)]
    fn len(&self) -> usize {
        self.slots.len()
    }

    /// Add `slot` at [`Self::len`]; the caller has made room.
    #[inline]
    fn push(&mut self, slot: Slot) {
        self.slots.push(slot);
    }

    fn clear(&mut self) {
        self.slots.clear();
    }

    #[inline(always)]
    fn get(&self, index: usize) -> Slot {
        self.slots[index]
    }
}

#[cfg(not(feature = "alloc"))]
impl Pool {
    const fn new() -> Self {
        Self {
            slots: [Slot { op: Op::Nop, word: 0 }; POOL],
            len: 0,
        }
    }

    /// The first free slot.
    #[inline(always)]
    fn len(&self) -> usize {
        self.len
    }

    /// Add `slot` at [`Self::len`]; the caller has made room.
    #[inline]
    fn push(&mut self, slot: Slot) {
        self.slots[self.len] = slot;
        self.len += 1;
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    #[inline(always)]
    fn get(&self, index: usize) -> Slot {
        self.slots[index]
    }
}

/// The traces a processor has built.
pub(crate) struct TraceCache {
    entries: Table<Entry, ENTRIES>,
    pool: Pool,
    generation: u32,
    /// Bochs `pageWriteStampTable::fineGranularityMapping`: per page (by
    /// [`stamp_index`]), a bit per 128-byte line holding a cached
    /// instruction.
    stamps: Table<u32, STAMPS>,
}

#[inline(always)]
const fn slot(pa: u64) -> usize {
    ((pa >> 2) as usize) & (ENTRIES - 1)
}

#[inline(always)]
const fn stamp_index(pa: u64) -> usize {
    ((pa >> 12) as usize) & (STAMPS - 1)
}

/// The 128-byte lines of a page that the bytes at page offsets `first` to
/// `last` (inclusive) touch (Bochs `markICache`).
#[inline]
const fn line_mask(first: u64, last: u64) -> u32 {
    let first_line = (first >> 7) as u32;
    let lines = (last >> 7) as u32 - first_line + 1;
    let range = if lines >= 32 { u32::MAX } else { (1u32 << lines) - 1 };
    range << first_line
}

impl Entry {
    #[inline(always)]
    const fn trace(&self) -> Trace {
        Trace {
            start: (self.at >> 8) as usize,
            len: (self.at & 0xFF) as usize,
        }
    }

    /// The lines of its page its instructions are on.
    #[inline]
    const fn lines(&self) -> u32 {
        let first = self.pa & 0xFFF;
        line_mask(first, first + 4 * self.trace().len as u64 - 1)
    }
}

impl TraceCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: table::<_, ENTRIES>(EMPTY),
            pool: Pool::new(),
            generation: 0,
            stamps: table::<_, STAMPS>(0),
        }
    }

    /// The trace starting at `pa`, built from memory of `epoch`, if it is
    /// cached.
    #[inline(always)]
    fn lookup(&self, pa: u64, epoch: CodeEpoch) -> Option<Trace> {
        let e = self.entries[slot(pa)];
        (e.pa == pa && e.generation == self.generation && e.epoch == epoch).then_some(e.trace())
    }

    /// Forget every trace (Bochs `flushTraceCacheEntries`). The stamps stay:
    /// a stamp with no trace behind it costs a write one look.
    pub(crate) fn flush(&mut self) {
        self.pool.clear();
        if self.generation == u32::MAX {
            self.entries.fill(EMPTY);
            self.generation = 0;
        } else {
            self.generation += 1;
        }
    }

    /// Whether the page of `pa` may hold a cached instruction.
    #[inline(always)]
    pub(crate) fn holds_code(&self, pa: u64) -> bool {
        self.stamps[stamp_index(pa)] != 0
    }

    /// Forget the traces on the `mask` lines of the page of `pa`; whether
    /// any line was marked (Bochs `handleSMC`).
    fn forget_lines(&mut self, pa: u64, mask: u32) -> bool {
        let index = stamp_index(pa);
        let mask = mask & self.stamps[index];
        if mask == 0 {
            return false;
        }
        // Every trace that starts in the page is in its window, one line of
        // 32 entries at a time; a trace reaches forward only, so none past
        // the last written line can touch it.
        let window = slot(pa & !0xFFF);
        for line in 0..32 {
            if 1u32 << line > mask {
                break;
            }
            for k in 0..32 {
                let e = &mut self.entries[(window + line * 32 + k) & (ENTRIES - 1)];
                if e.pa != INVALID && e.generation == self.generation && stamp_index(e.pa) == index && e.lines() & mask != 0 {
                    e.pa = INVALID;
                }
            }
        }
        self.stamps[index] &= !mask;
        true
    }
}

impl Cpu {
    /// Run the instructions from the PC on, at most `limit` (at least one)
    /// of them: a cached trace's, or a single instruction by the
    /// per-instruction path where no trace can be used. Returns what the
    /// last instruction run did.
    #[inline(always)]
    pub(crate) fn run_trace<B: PhysBus, I: Instrumentation>(&mut self, bus: &mut B, observer: &mut I, limit: u64) -> Step {
        let pc = self.pc;
        let mut observed = Observed { bus, observer };
        let pa = match self.fetch_address(&mut observed, pc) {
            Ok(pa) => pa,
            Err(fault) => return self.finish(pc, None, Err(fault), observed.observer).step(),
        };
        let trace = match observed.fetch_source(PhysAddr::new(pa)) {
            FetchSource::Stable(epoch) => match self.traces.lookup(pa, epoch) {
                Some(trace) => Some(trace),
                None => self.build_trace(&mut observed, pa, epoch),
            },
            FetchSource::Volatile => None,
        };
        let Some(trace) = trace else {
            let mut fetched = None;
            let result = self.execute_at(&mut observed, pc, pa, &mut fetched);
            return self.finish(pc, fetched, result, observed.observer).step();
        };
        self.trace_break = false;
        let count = (trace.len as u64).min(limit) as usize;
        let mut pc = pc;
        for i in trace.start..trace.start + count {
            let Slot { op, word } = self.traces.pool.get(i);
            observed.observer.before_execution(pc, word);
            let result = self.exec_op(&mut observed, pc, op);
            match self.finish(pc, Some(word), result, observed.observer) {
                Done::Next => {}
                done => return done.step(),
            }
            if self.trace_break || self.timer_dirty || observed.devices_touched() {
                break;
            }
            pc = pc.wrapping_add(4);
        }
        Step::Retired
    }

    /// Decode the trace starting at `pa`, of `epoch`, into the cache.
    /// `None` when its first word cannot be read, which a stable source
    /// never refuses; the per-instruction path then takes the fetch's
    /// abort.
    #[cold]
    #[inline(never)]
    fn build_trace<B: PhysBus, I: Instrumentation>(&mut self, bus: &mut Observed<'_, B, I>, pa: u64, epoch: CodeEpoch) -> Option<Trace> {
        if self.traces.pool.len() + MAX_TRACE > POOL {
            self.traces.flush();
        }
        let start = self.traces.pool.len();
        let room = ((0x1000 - (pa & 0xFFF)) / 4) as usize;
        let clock = self.clock();
        let mut len = 0;
        while len < MAX_TRACE.min(room) {
            let addr = pa + 4 * len as u64;
            let Ok(word) = bus.fetch(PhysAddr::new(addr), clock) else {
                break;
            };
            let op = predecode(classify(word), word);
            self.traces.pool.push(Slot { op, word });
            len += 1;
            if op.ends_trace() {
                break;
            }
        }
        if len == 0 {
            return None;
        }
        let entry = Entry {
            pa,
            epoch,
            generation: self.traces.generation,
            at: ((start as u32) << 8) | len as u32,
        };
        self.traces.entries[slot(pa)] = entry;
        self.traces.stamps[stamp_index(pa)] |= entry.lines();
        Some(Trace { start, len })
    }

    /// Forget every trace holding an instruction in `span`, which something
    /// has written or is about to write; the running trace ends if any was.
    /// A span of more pages than there are stamps would visit every stamp,
    /// some more than once, and is answered by forgetting every trace.
    pub(crate) fn forget_code(&mut self, span: PhysSpan) {
        let (start, end) = (span.start.get(), span.end.get());
        if span.len() / 0x1000 >= STAMPS as u64 {
            self.flush_traces();
            return;
        }
        let mut page = start & !0xFFF;
        while page < end {
            let page_end = page.saturating_add(0x1000);
            if self.traces.holds_code(page) {
                let first = start.max(page) - page;
                let last = end.min(page_end) - 1 - page;
                if self.traces.forget_lines(page, line_mask(first, last)) {
                    self.trace_break = true;
                }
            }
            page = page_end;
        }
    }

    /// Forget every trace, and end the running one.
    pub(crate) fn flush_traces(&mut self) {
        self.traces.flush();
        self.trace_break = true;
    }
}
