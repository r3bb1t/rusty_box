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
//!   to guest RAM). Pages share a stamp when their numbers agree modulo the
//!   number of stamps, which covers the machine's RAM (see [`size`]), as
//!   Bochs shares them above 4 GiB: that only ever forgets more.
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
//! write over cached code, or a TLB maintenance instruction. The first
//! instruction's fetch is translated and permission-checked as `Cpu::step`
//! checks every fetch; the rest of the trace is in the same page, whose
//! translation and execute permission nothing between them can change (a
//! change ends the trace), so each of their fetches would find the same.
//! Every instruction is executed and reported to the observer exactly as
//! `Cpu::step` would, so a machine runs the same instructions with the same
//! results either way. The one exception is the whole iterations of a
//! counter poll, which a trace whose first instruction is the poll's
//! counter read may fast-forward, within the bound its caller gives
//! ([`PollBound`]), when the observer does not hear of every instruction:
//! they leave exactly the state running them would (`poll.rs`). A trace
//! that may begin a poll is marked when it is built ([`POLL`]), so that
//! its lookup misses and the decision is made on the miss path: any other
//! trace costs nothing more to find or run.
//!
//! Traces are linked (Bochs cpu.cc `linkTrace`, cpu.h `BX_LINK_TRACE`): a
//! trace that ends in a taken direct branch to its own page goes on into
//! the cached trace there without returning to the run loop, when nothing
//! the loop checks between traces can stop it — the run has instructions
//! left, no interrupt is deliverable, and the run watches no exit address.
//! Bochs remembers the next trace in the branch, valid while a timestamp
//! that every invalidation moves is unchanged; here the next trace is the
//! cache's own entry for the target, found by its address, epoch and
//! generation, which is what any invalidation changes (`Cpu::linked_trace`).

use super::decode::classify;
use super::exec::{Done, Fault, Observed, Step};
use super::id::MAX_PA_BITS;
use super::op::{predecode, Op, OpSource};
use super::poll::PollStep;
use super::{Btype, Cpu, Levels};
use crate::bus::{CodeEpoch, FetchSource, PhysAddr, PhysBus, PhysSpan};
use crate::instrumentation::{Execution, Instrumentation};

#[cfg(all(test, feature = "alloc"))]
#[path = "../../tests/unit/trace.rs"]
mod tests;

/// The longest trace, in instructions (Bochs `BX_MAX_TRACE_LENGTH`).
pub(crate) const MAX_TRACE: usize = 32;

/// The cache's sizes. Whatever they are, the cache changes nothing but
/// speed.
///
/// With `alloc` the tables are on the heap, and the pool and the entries
/// are sized as rusty_box's x86 interpreter sizes its own
/// (`rusty_box/src/cpu/icache.rs`): Bochs's pool (icache.h
/// `BxTraceCacheMemPool`) and 64 Ki entries. A Linux boot runs about 94,000
/// distinct instructions, and traces that start mid-run hold some of them
/// twice; at these sizes an Alpine boot builds 217,178 traces and empties
/// the pool 3 times. A trace build is cheap, about 2,100 host instructions
/// for 32 of A64's fixed-length words. The pool grows as traces are built
/// (see [`Pool`]), so a machine holds only the pool it has used.
///
/// The page write stamps are a table read at random, zeroed from the start,
/// with one stamp per page of the machine's RAM: [`MIN_STAMPS`], as many
/// as the board's default 256 MiB has, or more for more RAM, up to
/// [`MAX_STAMPS`] (Bochs `bxPageWriteStampTable`, 4 GiB of pages). Consecutive
/// pages, up to the count, have stamps of their own (see
/// [`TraceCache::cover`]).
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
    /// The fewest page write stamps: a power of two, one per page of
    /// 256 MiB.
    pub(super) const MIN_STAMPS: usize = 65536;
    /// The most: one per page of 4 GiB (Bochs icache.h
    /// `PHY_MEM_PAGES_IN_4G_SPACE`).
    pub(super) const MAX_STAMPS: usize = 1024 * 1024;
}
#[cfg(not(feature = "alloc"))]
mod size {
    pub(super) const ENTRIES: usize = 4096;
    pub(super) const POOL: usize = 16384;
    /// Page write stamps, always this many.
    pub(super) const MIN_STAMPS: usize = 8192;
    pub(super) const MAX_STAMPS: usize = MIN_STAMPS;
}
use size::{ENTRIES, MAX_STAMPS, MIN_STAMPS, POOL};

const INVALID: u64 = u64::MAX;

/// Set in an entry's `pa` when the trace may begin a counter poll
/// ([`TraceCache::may_poll`]): its first instruction reads the system
/// counter ([`Op::ReadCounter`]). No physical address has this bit,
/// so [`TraceCache::lookup`] misses such a trace and
/// [`TraceCache::lookup_poll`] finds it. An entry's address is 4-aligned,
/// so a marked one is never [`INVALID`].
const POLL: u64 = 1 << 63;

const _: () = assert!(MAX_PA_BITS < 63);

// Pages that share a stamp must share their window of entries, or a write
// to one would clear the stamp of the other's traces without forgetting
// them (Bochs icache.h relies on the same agreement between its hashes):
// every stamp count is a power of two from MIN_STAMPS up, so a multiple of
// MIN_STAMPS.
const _: () = assert!(
    ENTRIES.is_power_of_two() && ENTRIES >= 1024 && (MIN_STAMPS * 1024).is_multiple_of(ENTRIES)
);
const _: () = assert!(
    MIN_STAMPS.is_power_of_two() && MAX_STAMPS.is_power_of_two() && MIN_STAMPS <= MAX_STAMPS
);
const _: () = assert!(POOL >= MAX_TRACE && POOL < 1 << 24);

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
    /// The physical address of its first instruction, with [`POLL`] set
    /// when the trace may begin a counter poll; [`INVALID`] for none.
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
/// [`POOL`]. With `alloc` it is a vector that grows as traces are built and
/// keeps its capacity when emptied, so that a processor holds only the
/// pool it has used: building one allocates nothing for the pool, as a test
/// that builds thousands of machines needs, and a machine that runs little
/// code never reserves the 13.5 MiB of a full pool. Without, it is inline
/// (R0's representation alias, as [`Table`] is).
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
    const fn new() -> Self {
        Self {
            slots: alloc::vec::Vec::new(),
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

    /// The op in slot `index`, where it lies.
    #[inline(always)]
    fn op(&self, index: usize) -> &Op {
        &self.slots[index].op
    }

    /// The word the op in slot `index` was decoded from.
    #[inline(always)]
    fn word(&self, index: usize) -> u32 {
        self.slots[index].word
    }
}

#[cfg(not(feature = "alloc"))]
impl Pool {
    const fn new() -> Self {
        Self {
            slots: [Slot {
                op: Op::Nop,
                word: 0,
            }; POOL],
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

    /// The op in slot `index`, where it lies.
    #[inline(always)]
    fn op(&self, index: usize) -> &Op {
        &self.slots[index].op
    }

    /// The word the op in slot `index` was decoded from.
    #[inline(always)]
    fn word(&self, index: usize) -> u32 {
        self.slots[index].word
    }
}

/// The op in slot `.0` of the pool, which [`Cpu::exec_op`] reads in place.
struct PoolSlot(usize);

impl OpSource for PoolSlot {
    #[inline(always)]
    fn op<'c, L: Levels>(&'c self, cpu: &'c Cpu<L>) -> &'c Op {
        cpu.traces.pool.op(self.0)
    }
}

#[cfg(test)]
impl<L: Levels> Cpu<L> {
    /// [`Cpu::exec_op`] reading `op` where a trace's op lies, in slot 0 of
    /// the pool ([`PoolSlot`]), as [`Cpu::run_trace`] does. Every trace is
    /// forgotten first, so the pool holds `op` alone.
    pub(super) fn exec_pooled<'f, B: PhysBus>(
        &mut self,
        bus: &mut B,
        pc: u64,
        op: Op,
        word: u32,
        raised: &'f mut Option<Fault>,
    ) -> super::exec::OpResult<'f> {
        self.traces.flush();
        self.traces.pool.push(Slot { op, word });
        self.exec_op(bus, pc, PoolSlot(0), raised)
    }
}

/// How far a counter poll whose read starts a trace may be fast-forwarded
/// (`poll.rs`), when the observer does not hear of every instruction
/// ([`Execution::Skippable`]).
#[derive(Debug, Clone, Copy)]
pub(super) struct PollBound<'x> {
    /// The instructions, the read included, that [`Cpu::run_trace`]'s
    /// caller lets run before it next looks outside the processor: what is
    /// left of its budget, of its slice and before the next event
    /// ([`Cpu::bound_run`]).
    pub(super) left: u64,
    /// The addresses the caller stops at, before the instruction there
    /// runs: no instruction a skip passes is at one.
    pub(super) exits: &'x [u64],
}

/// The traces a processor has built.
pub(crate) struct TraceCache {
    entries: Table<Entry, ENTRIES>,
    pool: Pool,
    generation: u32,
    /// The end of the run of instructions [`Cpu::run_trace`]'s caller lets
    /// the processor make before it next looks outside it, as a tick
    /// modulo 2^32 ([`Cpu::bound_run`]): a run is shorter than 2^32
    /// instructions, each of which retires in one tick. Only the miss path
    /// of a trace marked [`POLL`] reads it. It is a `u32`, and shares one
    /// 8-byte slot with `generation`, the only other field narrower than 8
    /// bytes (asserted below the struct), so the cache's size, and the
    /// processor's layout, on which the trace loop's register allocation
    /// depends, hold no room of their own for it.
    run_end: u32,
    /// Bochs `pageWriteStampTable::fineGranularityMapping`: per page (by
    /// [`TraceCache::stamp_index`]), a bit per 128-byte line holding a
    /// cached instruction. A power of two from [`MIN_STAMPS`] to
    /// [`MAX_STAMPS`] long.
    stamps: Table<u32, MIN_STAMPS>,
}

// `generation` and `run_end` together take one 8-byte slot of the cache.
const _: () = assert!(
    core::mem::size_of::<TraceCache>()
        == core::mem::size_of::<Table<Entry, ENTRIES>>()
            + core::mem::size_of::<Pool>()
            + core::mem::size_of::<Table<u32, MIN_STAMPS>>()
            + 8
);

#[inline(always)]
const fn slot(pa: u64) -> usize {
    ((pa >> 2) as usize) & (ENTRIES - 1)
}

/// The stamp of the page of `pa` among `count` (a power of two): pages
/// whose numbers agree modulo `count` share one.
#[inline(always)]
const fn stamp_index(pa: u64, count: usize) -> usize {
    ((pa >> 12) as usize) & (count - 1)
}

/// The 128-byte lines of a page that the bytes at page offsets `first` to
/// `last` (inclusive) touch (Bochs `markICache`).
#[inline]
const fn line_mask(first: u64, last: u64) -> u32 {
    let first_line = (first >> 7) as u32;
    let lines = (last >> 7) as u32 - first_line + 1;
    let range = if lines >= 32 {
        u32::MAX
    } else {
        (1u32 << lines) - 1
    };
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

    /// The physical address of its first instruction, unmarked.
    #[inline]
    const fn address(&self) -> u64 {
        self.pa & !POLL
    }

    /// The lines of its page its instructions are on.
    #[inline]
    const fn lines(&self) -> u32 {
        let first = self.address() & 0xFFF;
        line_mask(first, first + 4 * self.trace().len as u64 - 1)
    }
}

impl TraceCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: table::<_, ENTRIES>(EMPTY),
            pool: Pool::new(),
            generation: 0,
            run_end: 0,
            stamps: table::<_, MIN_STAMPS>(0),
        }
    }

    /// Give every page of `bytes` of consecutive memory (a machine's RAM) a
    /// stamp of its own, as far as [`MAX_STAMPS`] reach: the fewest stamps,
    /// a power of two, that number its pages apart. Every trace is forgotten
    /// when the table changes, since the marks of the old one go with it.
    #[cfg(feature = "alloc")]
    pub(crate) fn cover(&mut self, bytes: u64) {
        let pages = usize::try_from(bytes.div_ceil(0x1000)).unwrap_or(MAX_STAMPS);
        let stamps = pages.next_power_of_two().clamp(MIN_STAMPS, MAX_STAMPS);
        if stamps != self.stamps.len() {
            self.stamps = alloc::vec![0; stamps].into_boxed_slice();
            self.flush();
        }
    }

    /// The stamp of the page of `pa`.
    #[inline(always)]
    fn stamp_index(&self, pa: u64) -> usize {
        stamp_index(pa, self.stamps.len())
    }

    /// How many stamps there are.
    #[inline]
    fn stamp_count(&self) -> usize {
        self.stamps.len()
    }

    /// The trace starting at `pa`, built from memory of `epoch`, if it is
    /// cached.
    #[inline(always)]
    fn lookup(&self, pa: u64, epoch: CodeEpoch) -> Option<Trace> {
        let e = self.entries[slot(pa)];
        (e.pa == pa && e.generation == self.generation && e.epoch == epoch).then_some(e.trace())
    }

    /// The trace starting at `pa`, built from memory of `epoch`, if it is
    /// cached and marked [`POLL`]: the traces [`Self::lookup`] does not
    /// give.
    #[inline]
    fn lookup_poll(&self, pa: u64, epoch: CodeEpoch) -> Option<Trace> {
        let e = self.entries[slot(pa)];
        (e.pa == pa | POLL && e.generation == self.generation && e.epoch == epoch)
            .then_some(e.trace())
    }

    /// Whether the entry for `pa` is the trace starting there marked
    /// [`POLL`]: every trace [`Self::lookup_poll`] gives, and a trace
    /// [`Cpu::build_trace`] has just built and marked.
    #[inline]
    fn marked_poll(&self, pa: u64) -> bool {
        self.entries[slot(pa)].pa == pa | POLL
    }

    /// Whether the trace of the `len` ops at `start` in the pool may begin
    /// a counter poll (`poll.rs`): its first op reads the counter, and the
    /// walk of a poll may pass each op after it as far as the first that
    /// may branch, or the trace's end. A poll's walk from any other trace
    /// fails whatever the registers hold.
    fn may_poll(&self, start: usize, len: usize) -> bool {
        if !matches!(self.pool.op(start), Op::ReadCounter { .. }) {
            return false;
        }
        for i in start + 1..start + len {
            match PollStep::of(*self.pool.op(i)) {
                PollStep::Rejected => return false,
                PollStep::Straight => {}
                PollStep::Branch => return true,
            }
        }
        true
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
        self.stamps[self.stamp_index(pa)] != 0
    }

    /// Forget the traces on the `mask` lines of the page of `pa`; whether
    /// any line was marked (Bochs `handleSMC`).
    fn forget_lines(&mut self, pa: u64, mask: u32) -> bool {
        let index = self.stamp_index(pa);
        let mask = mask & self.stamps[index];
        if mask == 0 {
            return false;
        }
        // Every trace that starts in the page is in its window, one line of
        // 32 entries at a time; a trace reaches forward only, so none past
        // the last written line can touch it.
        let window = slot(pa & !0xFFF);
        let count = self.stamps.len();
        for line in 0..32 {
            if 1u32 << line > mask {
                break;
            }
            for k in 0..32 {
                let e = &mut self.entries[(window + line * 32 + k) & (ENTRIES - 1)];
                if e.pa != INVALID
                    && e.generation == self.generation
                    && stamp_index(e.address(), count) == index
                    && e.lines() & mask != 0
                {
                    e.pa = INVALID;
                }
            }
        }
        self.stamps[index] &= !mask;
        true
    }
}

impl<L: Levels> Cpu<L> {
    /// Run the instructions from the PC on: a cached trace's, at most
    /// `limit` (at least one) of them, or a single instruction by the
    /// per-instruction path where no trace can be used. Returns what the
    /// last instruction run did.
    ///
    /// A trace runs only with PSTATE.BTYPE clear. Of its instructions only
    /// a branch through a register sets BTYPE (FEAT_BTI), and such a branch
    /// ends the trace, so its later instructions start with BTYPE clear,
    /// as Arm `BranchTargetCheck` would leave it. With BTYPE set, the one
    /// instruction runs by [`Cpu::step`], which makes that check.
    ///
    /// When `observer` does not hear of every instruction, a counter poll
    /// whose read starts the trace may first be fast-forwarded (`poll.rs`):
    /// whole iterations of it are skipped before the trace runs, so one
    /// call may retire more than `limit` instructions, but never past the
    /// end of the run [`Cpu::bound_run`] set, which `limit` must not pass,
    /// nor past any of `exits`, the addresses the caller stops at. Every
    /// instruction skipped counts as retired, and the skip leaves more than
    /// the rest of the trace before the end of that run. A poll's iteration
    /// comes back to its read by a taken branch, which starts a trace
    /// there, so the check is made once per trace, on the miss path a
    /// poll's trace takes ([`POLL`]).
    ///
    /// With no `exits`, `limit` is all that is left of that run, and a
    /// trace that ends in a direct branch goes on into the trace the branch
    /// leads to, where the caller would have run it next with nothing else
    /// done in between ([`Cpu::linked_trace`]): the instructions run, and
    /// what the observer hears of them, are the same either way.
    #[inline(always)]
    pub(crate) fn run_trace<B: PhysBus, I: Instrumentation>(
        &mut self,
        bus: &mut B,
        observer: &mut I,
        limit: u64,
        exits: &[u64],
    ) -> Step {
        debug_assert!(
            limit <= self.run_left(),
            "a trace's limit passes the end of its run"
        );
        debug_assert!(
            !exits.is_empty() || limit == self.run_left(),
            "with no exits, a trace's limit is the rest of its run"
        );
        if self.pstate.btype != Btype::None || self.pstate.il || self.step_each {
            return self.step_fetch_check(bus, observer);
        }
        let pc = self.pc;
        let mut observed = Observed { bus, observer };
        let pa = match self.fetch_address(&mut observed, pc) {
            Ok(pa) => pa,
            Err(fault) => return self.take_fault(pc, fault, observed.observer),
        };
        // The fetch's own walk may have reached a device, by tables on one:
        // the caller looks at the machine after this one instruction, which
        // a trace would run, and link, past. An instruction in a trace
        // reaches no device, so no later fetch of the run does.
        if observed.devices_touched() {
            return self.step_uncached(&mut observed, pc, pa);
        }
        let (trace, epoch) = match observed.fetch_source(pa) {
            FetchSource::Stable(epoch) => match self.traces.lookup(pa.get(), epoch) {
                Some(trace) => (Some(trace), epoch),
                None => (
                    self.trace_miss(&mut observed, pa.get(), epoch, exits),
                    epoch,
                ),
            },
            FetchSource::Volatile => (None, CodeEpoch::new(0)),
        };
        let Some(trace) = trace else {
            return self.step_uncached(&mut observed, pc, pa);
        };
        self.trace_break = false;
        let mut slots = trace.start..trace.start + (trace.len as u64).min(limit) as usize;
        let mut pc = pc;
        let mut raised = None;
        // One trace's instructions, then the trace each links to.
        'linked: loop {
            for i in slots {
                let word = self.traces.pool.word(i);
                observed.observer.before_execution(pc, word);
                let flow = match self.exec_op(&mut observed, pc, PoolSlot(i), &mut raised) {
                    Ok(flow) => flow,
                    Err(fault) => return self.take_fault(pc, *fault, observed.observer),
                };
                match self.retire(pc, Some(word), flow, observed.observer) {
                    Done::Next => {}
                    Done::Elsewhere if exits.is_empty() => {
                        debug_assert!(
                            !self.trace_break && !self.timer_dirty && !observed.devices_touched()
                        );
                        match self.linked_trace(i, pc, pa.get(), epoch) {
                            Some(next) => {
                                slots = next.start..next.start + next.len;
                                pc = self.pc;
                                continue 'linked;
                            }
                            None => return Step::Retired,
                        }
                    }
                    done => return done.step(),
                }
                if self.trace_break || self.timer_dirty || observed.devices_touched() {
                    return Step::Retired;
                }
                pc = pc.wrapping_add(4);
            }
            return Step::Retired;
        }
    }

    /// The trace that the taken branch in pool slot `slot`, at `branch_pc`,
    /// which has just ended a trace read from physical address `pa`'s page
    /// of memory of `epoch`, leads to, when it is entered here rather than
    /// by the caller (Bochs cpu.cc `linkTrace`, cpu.h `BX_LINK_TRACE`),
    /// bounded by what is left of the run. `None` returns to the caller,
    /// which finds the same trace, or none, its own way.
    ///
    /// Only a direct branch links (B, BL, B.cond, CBZ/CBNZ, TBZ/TBNZ, as
    /// Bochs links only its direct jumps' traces): it changes neither the
    /// exception level, `PSTATE.IL` nor `PSTATE.BTYPE` (which stays clear,
    /// as a trace needs), and its target is aligned. Only a
    /// target in the branch's own 4 KiB page links: the trace's first
    /// fetch translated that page and checked its execute permission, and
    /// nothing in the trace changed them (a TLB fill, flush or maintenance
    /// instruction ends a trace), so the caller's fetch would find the same
    /// physical page, guarded or not as [`Cpu::in_guarded_page`] already
    /// says; and its memory's source and epoch are the trace's:
    /// RAM's never change, and only a write to a flash bank, a device
    /// access that ends a trace, takes the bank out of read-array mode or
    /// moves its epoch. The
    /// trace is the cache's entry for that address and epoch in this
    /// generation, as the caller's lookup finds it — after a flush, a write
    /// over its code or another trace taking its slot it is not there, and
    /// a trace marked [`POLL`] is never found, so the poll's decision is
    /// made on the caller's miss path. Of the checks the caller makes
    /// between traces: the run's end and a deliverable interrupt are
    /// checked here (an interrupt returns to the caller, to be taken); a
    /// trace ends after any instruction that reaches a device or writes a
    /// timer register, so the next event and the interrupt lines are as the
    /// caller left them; and a run with exit addresses never links. Bochs
    /// keeps the link as a pointer in the trace, valid while its timestamp
    /// is; here the direct-mapped cache is the pointer, and its generation
    /// and epochs the timestamp.
    ///
    /// Out of line, and cold although a loop takes it once a trace, so that
    /// the trace loop's register allocation does not depend on it.
    #[cold]
    #[inline(never)]
    fn linked_trace(
        &mut self,
        slot: usize,
        branch_pc: u64,
        pa: u64,
        epoch: CodeEpoch,
    ) -> Option<Trace> {
        let target = self.pc;
        if !matches!(
            self.traces.pool.op(slot),
            Op::B { .. } | Op::Bl { .. } | Op::BCond { .. } | Op::Cbz { .. } | Op::Tbz { .. }
        ) || (target ^ branch_pc) >> 12 != 0
            || self.run_left() == 0
            || self.interrupt_deliverable()
        {
            return None;
        }
        let trace = self
            .traces
            .lookup((pa & !0xFFF) | (target & 0xFFF), epoch)?;
        Some(Trace {
            start: trace.start,
            len: (trace.len as u64).min(self.run_left()) as usize,
        })
    }

    /// Account for the instruction at `pc` having raised `fault`, in its
    /// fetch or in its execution, as [`Cpu::finish`] does. Out of line, and
    /// cold, so that the trace loop's register allocation does not depend
    /// on it.
    #[cold]
    #[inline(never)]
    fn take_fault<I: Instrumentation>(&mut self, pc: u64, fault: Fault, observer: &mut I) -> Step {
        self.finish(pc, None, Err(fault), observer).step()
    }

    /// Run the instruction at the PC by [`Cpu::step`], with a check of it
    /// due: PSTATE.BTYPE set (a branch through a register led to it, and its
    /// branch target check is due), PSTATE.IL set (an illegal exception
    /// return led to it, and it raises the Illegal Execution state exception
    /// once fetched) or [`Cpu::step_each`] set (Software Step or a
    /// breakpoint watches every instruction). Out of line, and cold, so
    /// that the trace loop's register allocation does not depend on it.
    #[cold]
    #[inline(never)]
    fn step_fetch_check<B: PhysBus, I: Instrumentation>(
        &mut self,
        bus: &mut B,
        observer: &mut I,
    ) -> Step {
        self.step(bus, observer)
    }

    /// Run the instruction at `pc`, at physical address `pa`, by the
    /// per-instruction path: its memory is not cached, or its first word
    /// could not be read into a trace. Out of line, and cold, so that the
    /// trace loop's register allocation does not depend on this second copy
    /// of the decoder and of `exec_op`.
    #[cold]
    #[inline(never)]
    fn step_uncached<B: PhysBus, I: Instrumentation>(
        &mut self,
        observed: &mut Observed<'_, B, I>,
        pc: u64,
        pa: PhysAddr,
    ) -> Step {
        let mut fetched = None;
        let result = self.execute_at(observed, pc, pa, &mut fetched);
        self.finish(pc, fetched, result, observed.observer).step()
    }

    /// Let the next `instructions` (fewer than 2^32) run with nothing
    /// outside the processor looking: the run within which a counter poll
    /// may be fast-forwarded (`poll.rs`). The caller sets it before each run
    /// of traces it counts, in which every instruction retires in one tick
    /// and nothing else moves time, and passes no trace a `limit` beyond it.
    #[inline(always)]
    pub(crate) fn bound_run(&mut self, instructions: u64) {
        debug_assert!(instructions <= u64::from(u32::MAX));
        // Modulo 2^32, as `TraceCache::run_end` keeps it.
        self.traces.run_end = self.icount.wrapping_add(instructions) as u32;
    }

    /// What is left of the run [`Cpu::bound_run`] last set, in
    /// instructions: nothing runs past its end, so the difference of ticks
    /// modulo 2^32 is the difference.
    #[inline]
    fn run_left(&self) -> u64 {
        u64::from(self.traces.run_end.wrapping_sub(self.icount as u32))
    }

    /// The trace starting at `pa`, of `epoch`, that [`TraceCache::lookup`]
    /// does not give: a cached trace marked [`POLL`], or one built now.
    /// Before the read of a poll such a trace begins runs, when the
    /// observer does not hear of every instruction, the poll is
    /// fast-forwarded as far as `poll.rs`'s rule allows, within what is left
    /// of the run [`Cpu::bound_run`] set and past none of `exits`. `None` as
    /// [`Cpu::build_trace`] gives it. Out of line, and cold, so that the
    /// trace loop's register allocation does not depend on it; it takes the
    /// run's end from the processor, not as an argument, for the same
    /// reason.
    #[cold]
    #[inline(never)]
    fn trace_miss<B: PhysBus, I: Instrumentation>(
        &mut self,
        observed: &mut Observed<'_, B, I>,
        pa: u64,
        epoch: CodeEpoch,
        exits: &[u64],
    ) -> Option<Trace> {
        let trace = match self.traces.lookup_poll(pa, epoch) {
            Some(trace) => trace,
            None => self.build_trace(observed, pa, epoch)?,
        };
        let may_skip = match I::EXECUTION {
            Execution::Heard => false,
            Execution::Skippable => true,
        };
        if may_skip && self.traces.marked_poll(pa) {
            if let Op::ReadCounter { word } = *self.traces.pool.op(trace.start) {
                let bound = PollBound {
                    left: self.run_left(),
                    exits,
                };
                self.fast_forward_poll(observed, self.pc, word, bound);
            }
        }
        Some(trace)
    }

    /// Decode the trace starting at `pa`, of `epoch`, into the cache.
    /// `None` when its first word cannot be read, which a stable source
    /// never refuses; the per-instruction path then takes the fetch's
    /// abort.
    #[cold]
    #[inline(never)]
    fn build_trace<B: PhysBus, I: Instrumentation>(
        &mut self,
        bus: &mut Observed<'_, B, I>,
        pa: u64,
        epoch: CodeEpoch,
    ) -> Option<Trace> {
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
            let op = predecode(classify(word, self.imp.features), word);
            self.traces.pool.push(Slot { op, word });
            len += 1;
            if op.ends_trace() {
                break;
            }
        }
        if len == 0 {
            return None;
        }
        let mark = if self.traces.may_poll(start, len) {
            POLL
        } else {
            0
        };
        let entry = Entry {
            pa: pa | mark,
            epoch,
            generation: self.traces.generation,
            at: ((start as u32) << 8) | len as u32,
        };
        self.traces.entries[slot(pa)] = entry;
        let index = self.traces.stamp_index(pa);
        self.traces.stamps[index] |= entry.lines();
        Some(Trace { start, len })
    }

    /// Forget every trace holding an instruction in `span`, which something
    /// has written or is about to write; the running trace ends if any was.
    /// A span of more pages than there are stamps would visit every stamp,
    /// some more than once, and is answered by forgetting every trace.
    pub(crate) fn forget_code(&mut self, span: PhysSpan) {
        let (start, end) = (span.start.get(), span.end.get());
        if span.len() / 0x1000 >= self.traces.stamp_count() as u64 {
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

    /// Give each page of `bytes` of RAM a page write stamp of its own (see
    /// [`TraceCache::cover`]), so that a store to one page never forgets
    /// the code of another; the running trace ends if the traces go.
    #[cfg(feature = "alloc")]
    pub(crate) fn cover_code_memory(&mut self, bytes: u64) {
        self.traces.cover(bytes);
        self.trace_break = true;
    }
}
