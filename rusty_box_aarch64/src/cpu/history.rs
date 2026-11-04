//! Where the processor executed, for post-mortem diagnostics.
//!
//! Between branches execution is sequential, so the record keeps one entry
//! per run of sequential instructions — the address the run started at and
//! how many instructions had retired before it — instead of one per
//! instruction. A straight-line instruction costs nothing to record; a
//! taken branch, an exception or interrupt entry, an exception return, a
//! reset and a host's write of the PC each start a run.
//! [`PcHistory::recent`] expands the runs back into the addresses of the
//! last [`PC_HISTORY`] instructions retired.

use super::PC_HISTORY;

/// Runs kept: a power of two above `PC_HISTORY`. Every run but the newest
/// holds at least one instruction (an empty newest run is replaced, not
/// pushed past), so `PC_HISTORY + 1` runs always cover `PC_HISTORY`
/// instructions.
const RUNS: usize = 128;
const _: () = assert!(RUNS.is_power_of_two() && RUNS > PC_HISTORY);

/// Instructions retired at `start`, `start + 4`, ... from the moment
/// `first` instructions had retired until the next run began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    start: u64,
    first: u64,
}

/// The runs, newest last.
#[derive(Debug, Clone)]
pub(crate) struct PcHistory {
    runs: [Run; RUNS],
    /// Runs begun since the record began, never zero: the newest is at
    /// `(count - 1) % RUNS`.
    count: usize,
}

impl PcHistory {
    /// A record whose only run starts at `pc` with `retired` instructions
    /// retired: nothing executed yet.
    pub(crate) const fn new(pc: u64, retired: u64) -> Self {
        let run = Run {
            start: pc,
            first: retired,
        };
        Self {
            runs: [run; RUNS],
            count: 1,
        }
    }

    /// The record of `pcs` (oldest first) as the last instructions retired
    /// before `retired`, with execution continuing at `pc`. `pcs` is at most
    /// `retired` long, as a record of real execution is.
    pub(crate) fn from_addresses(pcs: &[u64], retired: u64, pc: u64) -> Self {
        let mut history = Self::new(pc, retired);
        let earliest = retired - pcs.len() as u64;
        for (k, &addr) in pcs.iter().enumerate() {
            history.runs[k % RUNS] = Run {
                start: addr,
                first: earliest + k as u64,
            };
        }
        history.runs[pcs.len() % RUNS] = Run {
            start: pc,
            first: retired,
        };
        history.count = pcs.len() + 1;
        history
    }

    /// Execution continues out of sequence at `pc`, after `retired`
    /// instructions.
    #[inline]
    pub(crate) fn jump(&mut self, pc: u64, retired: u64) {
        let newest = &mut self.runs[(self.count - 1) % RUNS];
        if newest.first == retired {
            // Nothing ran at the newest run's start: it is replaced.
            newest.start = pc;
            return;
        }
        self.runs[self.count % RUNS] = Run {
            start: pc,
            first: retired,
        };
        self.count += 1;
    }

    /// The addresses of the last [`PC_HISTORY`] instructions retired (fewer
    /// if fewer have), oldest first, `retired` being the count retired now.
    pub(crate) fn recent(&self, retired: u64) -> RecentPcs {
        let mut pcs = [0u64; PC_HISTORY];
        let mut taken = 0usize;
        let mut end = retired;
        for k in 0..self.count.min(RUNS) {
            let run = self.runs[(self.count - 1 - k) % RUNS];
            let len = end - run.first;
            let take = (PC_HISTORY - taken).min(usize::try_from(len).unwrap_or(usize::MAX));
            for j in 0..take {
                // The run's newest instruction first, filling from the back.
                let index = len - 1 - j as u64;
                pcs[PC_HISTORY - 1 - taken - j] = run.start.wrapping_add(4 * index);
            }
            taken += take;
            end = run.first;
            if taken == PC_HISTORY {
                break;
            }
        }
        RecentPcs {
            pcs,
            next: PC_HISTORY - taken,
        }
    }
}

/// The addresses of the instructions a processor retired last, oldest
/// first: [`Cpu::recent_pcs`](super::Cpu::recent_pcs).
#[derive(Debug, Clone)]
pub struct RecentPcs {
    pcs: [u64; PC_HISTORY],
    next: usize,
}

impl Iterator for RecentPcs {
    type Item = u64;

    #[inline]
    fn next(&mut self) -> Option<u64> {
        let pc = *self.pcs.get(self.next)?;
        self.next += 1;
        Some(pc)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = PC_HISTORY - self.next;
        (left, Some(left))
    }
}

impl ExactSizeIterator for RecentPcs {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::vec::Vec;

    /// What the record must equal: every retired address, kept one by one.
    struct Naive {
        pcs: VecDeque<u64>,
    }

    impl Naive {
        fn retire(&mut self, pc: u64) {
            if self.pcs.len() == PC_HISTORY {
                self.pcs.pop_front();
            }
            self.pcs.push_back(pc);
        }
    }

    /// Random straight-line runs, branches, exceptions taken in place of an
    /// instruction (which retire and then jump), interrupts and host PC
    /// writes taken before anything ran at the new address: the runs expand
    /// to exactly the addresses an instruction-by-instruction record holds,
    /// at every point.
    #[test]
    fn runs_expand_to_the_addresses_retired_one_by_one() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut pc = 0x4000_0000u64;
        let mut retired = 0u64;
        let mut history = PcHistory::new(pc, retired);
        let mut naive = Naive {
            pcs: VecDeque::new(),
        };
        for _ in 0..20_000 {
            match rand() % 8 {
                // Sequential instructions, the last of them wrapping the
                // address space now and then.
                0..=3 => {
                    for _ in 0..rand() % 6 {
                        naive.retire(pc);
                        retired += 1;
                        pc = pc.wrapping_add(4);
                    }
                }
                // A taken branch, or an exception: the instruction retires,
                // then execution continues elsewhere.
                4 | 5 => {
                    naive.retire(pc);
                    retired += 1;
                    pc = rand() & !3;
                    history.jump(pc, retired);
                }
                // An interrupt or a host write of the PC: nothing retires.
                _ => {
                    pc = if rand() % 4 == 0 {
                        u64::MAX - 7
                    } else {
                        rand() & !3
                    };
                    history.jump(pc, retired);
                }
            }
            let got: Vec<u64> = history.recent(retired).collect();
            assert_eq!(got, naive.pcs.iter().copied().collect::<Vec<_>>());
        }
    }

    /// A record rebuilt from its own addresses expands to the same
    /// addresses, now and after more execution.
    #[test]
    fn a_record_rebuilt_from_its_addresses_continues_as_the_original() {
        let mut history = PcHistory::new(0x1000, 10);
        let mut retired = 10;
        for target in [0x2000u64, 0x3000, 0x2000] {
            retired += 7;
            history.jump(target, retired);
        }
        let pcs: Vec<u64> = history.recent(retired).collect();
        let mut rebuilt = PcHistory::from_addresses(&pcs, retired, 0x2000);
        assert_eq!(rebuilt.recent(retired).collect::<Vec<_>>(), pcs);
        retired += 3;
        history.jump(0x5000, retired);
        rebuilt.jump(0x5000, retired);
        assert_eq!(
            rebuilt.recent(retired).collect::<Vec<_>>(),
            history.recent(retired).collect::<Vec<_>>()
        );
    }
}
