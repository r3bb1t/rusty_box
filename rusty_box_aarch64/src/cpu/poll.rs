//! Exact fast-forward of counter polls: loops that wait for time to pass by
//! reading the system counter until it passes a value.
//!
//! Firmware and kernels wait this way, and their delays spend many
//! instructions in such loops (the README's Speed section gives EDK2's
//! share). Each iteration's only effect is the passage of time, so when
//! the processor reaches the counter read of one, and nothing outside it
//! looks at the instructions in between, it moves straight to the end of
//! the last iteration that goes the same way as this one, leaving exactly
//! the state running every instruction would leave.
//!
//! # The rule
//!
//! [`Cpu::walk_poll`] simulates one iteration, from the counter read
//! (`MRS Xt, CNTPCT_EL0` or `CNTVCT_EL0`) back to it, for a given counter
//! value. It keeps three values of its own — `x30`, the counter register
//! `Xt` and NZCV — and reads every other register from the processor. Each
//! instruction is fetched as a permitted TLB hit (or with stage 1 off) from
//! memory whose fetches are stable ([`FetchSource::Stable`]), and must be
//! one of: a NOP; one 64-bit compare (`SUBS XZR` of `Xt` and a register, in
//! either order, or of `Xt` and an immediate); after the compare, a `B.cond`
//! on an ordering (HS, LO, HI, LS, GE, LT, GT, LE); `B`, `BL`; `BR`, `BLR`
//! (`RET` included) through a register other than `Xt`. The iteration must
//! come back to the read within [`MAX_POLL_LEN`] instructions, the read
//! included, with the compare run and `x30` holding what it held before,
//! and run no instruction at an exit address of the caller's.
//!
//! # Why it is exact
//!
//! - An iteration writes nothing but `x30`, the counter register and NZCV.
//!   `x30` is periodic: every accepted iteration ends with the value it
//!   started with (the walk checks it), so a skip leaves it as it is, and
//!   sets the counter register and NZCV to what the last skipped iteration
//!   leaves. An iteration reads no memory and accesses no device, raises no
//!   exception (every fetch is a permitted hit, and no accepted op can
//!   fault), and stores nothing, so its code cannot change under it.
//! - Its path is therefore a function of the counter value it reads alone:
//!   the only flags it tests are its one compare's, Arm ARM
//!   `AddWithCarry(a, NOT(b), 1)` of the counter value and a register the
//!   iteration does not write (or an immediate), and each `B.cond` tests an
//!   ordering of the two (Arm ARM `ConditionHolds`): unsigned, or signed
//!   while the counter is below 2^62, where its signed and unsigned orders
//!   agree. Each such condition, as the counter value grows, changes its
//!   outcome at most once.
//! - The counter is non-decreasing in the tick count (`Cpu::counter_at`,
//!   `CNTPCT = floor(icount * counter_hz / ips)`), and iteration `i` reads
//!   it at tick `icount + i * len`. So when iterations 0 and `i` take the
//!   same path, every `B.cond` on it has one outcome at both ends of an
//!   interval of counter values, and so throughout it: every iteration in
//!   between takes that path too. The search for the first iteration that
//!   goes another way is a binary search, and exact.
//! - Time is the only other thing the iterations change, and the skip
//!   moves `icount` and `retired` by exactly the instructions skipped.
//!   State derived from the counter — the event stream that sets the Event
//!   Register, the timer conditions — is computed from `icount` when it is
//!   read, so it is exact after the skip.
//!
//! # Its bounds
//!
//! A skip happens only when the observer does not hear of every
//! instruction (`Instrumentation::SEES_EXECUTION` is `false`), and only at
//! a read that starts a trace, on the miss path `Cpu::run_trace` takes
//! for such a trace. It is bounded by a [`PollBound`]. Its `left` is what
//! is left of the run the machine set with `Cpu::bound_run`: a burst,
//! which the machine counts to the nearest of the instruction budget, the
//! slice end and the next event — the earliest tick at which a timer output
//! rises (`Cpu::next_timer_deadline`) or a device changes by itself
//! (`VirtMachine::sync_events`). Nothing outside the processor can change
//! before it, so an interrupt is taken at the same instruction either way.
//! Its `exits` are the addresses the machine stops at: an iteration that
//! runs an instruction at one is not accepted, and every skipped iteration
//! runs the instructions the first one does.
//!
//! The skip stops `margin` iterations short of both that limit and the
//! first iteration that goes another way, and those iterations run as
//! usual. They refill the PC history that `Cpu::recent_pcs` and snapshots
//! read — the last [`PC_HISTORY`] instructions — which the skip leaves
//! holding one run of sequential addresses in place of the skipped
//! iterations: each run in the history but the newest holds at least one
//! instruction, so the `margin * jumps >= PC_HISTORY + 2` runs those
//! iterations start cover every entry read, and the machine runs them all
//! before it returns to its host. The instructions the trace that made the
//! read goes on to run after it are one iteration at most, which fits in
//! the margin too.

use super::decode::classify;
use super::dp::add_with_carry;
use super::exec::{bits, cond_holds, Nzcv};
use super::op::{predecode, Op};
use super::sysreg::SysRegKey;
use super::trace::PollBound;
use super::{Cpu, PC_HISTORY};
use crate::bus::{FetchSource, PhysAddr, PhysBus};

/// The longest iteration a poll may have, in instructions, its counter read
/// included.
const MAX_POLL_LEN: u64 = 32;

/// Whether `cond` is an ordering of a compare's operands (Arm ARM
/// `ConditionHolds`: HS, LO, HI, LS, GE, LT, GT, LE), not an equality, a
/// sign or an overflow.
const fn ordering(cond: u8) -> bool {
    matches!(cond, 2 | 3 | 8..=13)
}

/// One iteration of a poll, as [`Cpu::walk_poll`] found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PollCycle {
    /// Instructions, the counter read included.
    len: u64,
    /// Taken branches.
    jumps: u64,
    /// `B.cond`s run.
    conds: u32,
    /// Bit `k` set when the `k`th `B.cond` was taken.
    outcomes: u32,
    /// NZCV after the iteration: its compare's.
    flags: Nzcv,
}

impl PollCycle {
    /// Whether `other` took the same path.
    const fn same_path(&self, other: &Self) -> bool {
        self.len == other.len && self.jumps == other.jumps && self.conds == other.conds && self.outcomes == other.outcomes
    }
}

/// What an op is to [`Cpu::walk_poll`], by its shape alone: the walk
/// accepts no op of a shape [`PollStep::Rejected`] names, whatever the
/// registers hold. A `B.cond` on a condition other than an ordering is
/// one: both this and the walk ask [`ordering`].
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PollStep {
    /// The walk never accepts it.
    Rejected,
    /// The walk may accept it, and then goes on to the next op.
    Straight,
    /// The walk may accept it, and it may branch.
    Branch,
}

impl PollStep {
    /// What `op` is to the walk.
    pub(super) const fn of(op: Op) -> Self {
        match op {
            Op::Nop
            | Op::AddSubShift { sf: true, sub: true, flags: true, rd: 31, amount: 0, .. }
            | Op::AddsImm { sf: true, rd: 31, carry: true, .. } => Self::Straight,
            Op::BCond { cond, .. } if ordering(cond) => Self::Branch,
            Op::B { .. } | Op::Bl { .. } | Op::Br { .. } | Op::Blr { .. } => Self::Branch,
            Op::Class { .. }
            | Op::BCond { .. }
            | Op::AddImm { .. }
            | Op::AddsImm { .. }
            | Op::AddSubShift { .. }
            | Op::LogImm { .. }
            | Op::LogShift { .. }
            | Op::MovImm { .. }
            | Op::Movk { .. }
            | Op::Adr { .. }
            | Op::Adrp { .. }
            | Op::Ubfm { .. }
            | Op::Sbfm { .. }
            | Op::Bfm { .. }
            | Op::CondSel { .. }
            | Op::Madd { .. }
            | Op::Cbz { .. }
            | Op::Tbz { .. }
            | Op::LoadImm { .. }
            | Op::StoreImm { .. }
            | Op::LoadReg { .. }
            | Op::StoreReg { .. }
            | Op::LoadPair { .. }
            | Op::StorePair { .. }
            | Op::FpDp2 { .. }
            | Op::FpDp3 { .. }
            | Op::ReadCounter { .. } => Self::Rejected,
        }
    }
}

/// A poll as the walk follows it: its counter read, and where the caller
/// stops.
#[derive(Debug, Clone, Copy)]
struct Poll<'x> {
    /// The address of the counter read.
    start: u64,
    /// The register it reads into, `Xt`: neither 30 nor 31.
    rt: u32,
    /// The caller's exit addresses ([`PollBound::exits`]).
    exits: &'x [u64],
}

impl Cpu {
    /// The op at `pc`, when `pc` is none of `exits`, and a fetch there now
    /// would raise nothing and read memory whose fetches are stable.
    fn poll_op<B: PhysBus>(&self, bus: &mut B, pc: u64, exits: &[u64]) -> Option<Op> {
        if exits.contains(&pc) {
            return None;
        }
        let pa = PhysAddr::new(self.fetch_probe(pc)?);
        match bus.fetch_source(pa) {
            FetchSource::Stable(_) => {}
            FetchSource::Volatile => return None,
        }
        let word = bus.fetch(pa, self.clock()).ok()?;
        Some(predecode(classify(word), word))
    }

    /// General register `n` (31 reading as zero) as an iteration of a poll
    /// reads it: `rt` holds `value`, `x30` what the iteration last wrote to
    /// it, every other register what the processor holds.
    fn poll_reg(&self, n: u8, rt: u32, value: u64, x30: u64) -> u64 {
        let n = u32::from(n);
        if n == rt {
            value
        } else if n == 30 {
            x30
        } else {
            self.x_or_zr(n)
        }
    }

    /// One iteration of `poll` whose counter read reads `value`, if it is
    /// one the module's rule accepts.
    fn walk_poll<B: PhysBus>(&self, bus: &mut B, poll: Poll<'_>, value: u64) -> Option<PollCycle> {
        let Poll { start, rt, exits } = poll;
        match self.poll_op(bus, start, exits)? {
            Op::ReadCounter { word } if bits(word, 4, 0) == rt => {}
            _ => return None,
        }
        let mut x30 = self.x[30];
        let mut compared: Option<Nzcv> = None;
        let mut len = 1;
        let mut jumps = 0;
        let mut conds = 0u32;
        let mut outcomes = 0u32;
        let mut pc = start.wrapping_add(4);
        loop {
            if len == MAX_POLL_LEN {
                return None;
            }
            let op = self.poll_op(bus, pc, exits)?;
            len += 1;
            let next = match op {
                Op::Nop => pc.wrapping_add(4),
                Op::AddSubShift { sf: true, sub: true, flags: true, rd: 31, rn, rm, amount: 0, .. }
                    if compared.is_none() && (u32::from(rn) == rt) != (u32::from(rm) == rt) =>
                {
                    let a = self.poll_reg(rn, rt, value, x30);
                    let b = self.poll_reg(rm, rt, value, x30);
                    compared = Some(add_with_carry(a, !b, true, true).nzcv());
                    pc.wrapping_add(4)
                }
                Op::AddsImm { sf: true, rd: 31, rn, carry: true, y } if compared.is_none() && u32::from(rn) == rt => {
                    compared = Some(add_with_carry(value, y, true, true).nzcv());
                    pc.wrapping_add(4)
                }
                Op::BCond { cond, offset } if ordering(cond) => {
                    let taken = cond_holds(compared?, u32::from(cond));
                    outcomes |= u32::from(taken) << conds;
                    conds += 1;
                    if taken {
                        jumps += 1;
                        self.branch_addr(pc.wrapping_add(offset))
                    } else {
                        pc.wrapping_add(4)
                    }
                }
                Op::B { offset } => {
                    jumps += 1;
                    self.branch_addr(pc.wrapping_add(offset))
                }
                Op::Bl { offset } => {
                    x30 = pc.wrapping_add(4);
                    jumps += 1;
                    self.branch_addr(pc.wrapping_add(offset))
                }
                Op::Br { rn } if u32::from(rn) != rt => {
                    jumps += 1;
                    self.branch_addr(self.poll_reg(rn, rt, value, x30))
                }
                Op::Blr { rn } if u32::from(rn) != rt => {
                    let target = self.poll_reg(rn, rt, value, x30);
                    x30 = pc.wrapping_add(4);
                    jumps += 1;
                    self.branch_addr(target)
                }
                Op::Class { .. }
                | Op::AddImm { .. }
                | Op::AddsImm { .. }
                | Op::AddSubShift { .. }
                | Op::LogImm { .. }
                | Op::LogShift { .. }
                | Op::MovImm { .. }
                | Op::Movk { .. }
                | Op::Adr { .. }
                | Op::Adrp { .. }
                | Op::Ubfm { .. }
                | Op::Sbfm { .. }
                | Op::Bfm { .. }
                | Op::CondSel { .. }
                | Op::Madd { .. }
                | Op::BCond { .. }
                | Op::Cbz { .. }
                | Op::Tbz { .. }
                | Op::Br { .. }
                | Op::Blr { .. }
                | Op::LoadImm { .. }
                | Op::StoreImm { .. }
                | Op::LoadReg { .. }
                | Op::StoreReg { .. }
                | Op::LoadPair { .. }
                | Op::StorePair { .. }
                | Op::FpDp2 { .. }
                | Op::FpDp3 { .. }
                | Op::ReadCounter { .. } => return None,
            };
            if next == start {
                let flags = compared?;
                return (jumps > 0 && x30 == self.x[30]).then_some(PollCycle {
                    len,
                    jumps,
                    conds,
                    outcomes,
                    flags,
                });
            }
            pc = next;
        }
    }

    /// Iteration `i` from now of `poll`, iterations being `len`
    /// instructions long: the walk with the counter it reads.
    fn poll_iteration<B: PhysBus>(&self, bus: &mut B, poll: Poll<'_>, len: u64, i: u64) -> Option<PollCycle> {
        self.walk_poll(bus, poll, self.counter_at(self.icount + i * len))
    }

    /// Before the counter read `word` at `pc` executes: if it is the read
    /// of a poll the module's rule covers, skip every whole iteration from
    /// it on that the rule shows goes as this one does, short of the margin
    /// and of `bound.left`, none of whose instructions is at one of
    /// `bound.exits`. Out of line, and cold, reached only from the miss
    /// path of a trace that may begin a poll (`TraceCache::may_poll`), so
    /// that neither the trace loop nor the lookup of any other trace
    /// depends on it.
    #[cold]
    #[inline(never)]
    pub(super) fn fast_forward_poll<B: PhysBus>(&mut self, bus: &mut B, pc: u64, word: u32, bound: PollBound<'_>) {
        // XZR discards the value; x30 is the register a call writes, which
        // the walk keeps apart from the counter register.
        let rt = bits(word, 4, 0);
        if rt >= 30 {
            return;
        }
        // The read must not trap: CNTPCT_EL0 (op2 1) or CNTVCT_EL0 (op2 2).
        if self.sysreg_read(SysRegKey::new(3, 3, 14, 0, bits(word, 7, 5))).is_err() {
            return;
        }
        let poll = Poll {
            start: pc,
            rt,
            exits: bound.exits,
        };
        let Some(cycle) = self.walk_poll(bus, poll, self.counter()) else {
            return;
        };
        let len = cycle.len;
        // Iterations run as usual at the end, to refill the PC history.
        let margin = (PC_HISTORY as u64 + 2).div_ceil(cycle.jumps) + 1;
        let fit = bound.left / len;
        if fit <= margin {
            return;
        }
        // Below 2^62 the signed orderings of the counter are monotone too.
        match self.icount.checked_add(fit * len) {
            Some(end) if self.counter_at(end) < 1 << 62 => {}
            Some(_) | None => return,
        }
        let same = |cpu: &Self, bus: &mut B, i: u64| {
            cpu.poll_iteration(bus, poll, len, i).is_some_and(|c| c.same_path(&cycle))
        };
        // The first iteration in 1..=fit that goes another way, or `fit`.
        let first_other = if same(self, bus, fit) {
            fit
        } else {
            // Iteration lo - 1 goes this way, iteration hi does not.
            let (mut lo, mut hi) = (1, fit);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if same(self, bus, mid) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        let skip = first_other.saturating_sub(margin);
        if skip == 0 {
            return;
        }
        let Some(last) = self.poll_iteration(bus, poll, len, skip - 1) else {
            return;
        };
        self.x[rt as usize] = self.counter_at(self.icount + (skip - 1) * len);
        self.pstate.set_flags(last.flags);
        self.icount += skip * len;
        self.retired += skip * len;
    }
}
