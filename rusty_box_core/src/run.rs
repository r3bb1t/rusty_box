//! How a host bounds a run and how far the run got — the words every machine
//! answers in, whatever processor it emulates.
//!
//! A machine's reasons for stopping are its own (an x86 machine can triple
//! fault, an AArch64 one can be switched off over PSCI), so each machine names
//! those in its own closed enum. The budget and the progress are not: an
//! instruction is an instruction and a tick is a tick on every architecture,
//! and code that paces a guest should not have to know which one it paces.

/// What bounds one run.
///
/// The two units a machine can hold itself to without consulting a clock it
/// does not own. A host deadline is not one of them: `std::time::Instant` is
/// not available on every target these machines run on, and a front end that
/// wants to bound a frame already has a clock of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RunBudget {
    /// Guest instructions the boot processor may retire.
    ///
    /// Counted on the boot processor because that is the one a caller set the
    /// program counter on. An engine that does not count instructions cannot
    /// honour this, and says so rather than running unbounded.
    Instructions(u64),
    /// Ticks of guest time to let pass.
    ///
    /// The unit a processor that waits for an interrupt still advances in,
    /// and the one a machine's next timer deadline is measured in — so "run
    /// until the next device deadline" is expressible without guessing how
    /// many instructions that is.
    Ticks(u64),
}

/// How far a run got, in the unit the machine can actually answer in.
///
/// A uniprocessor on an engine that counts instructions answers in
/// instructions: one processor retires them, and its count is the machine's
/// own. A multiprocessor, or an engine that runs the guest on the host's own
/// processor, cannot, and reports the guest time its stretch took.
///
/// Naming the two apart is doctrine R4: a count of instructions and a span of
/// time are different things, and adding one to the other is refused rather
/// than silently summed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Progress {
    /// Guest instructions retired.
    Instructions(u64),
    /// Ticks of guest time that passed.
    Ticks(u64),
}

/// Two progress readings in different units, which do not add.
///
/// A machine reports one unit for its whole life, so meeting this means two
/// readings from different machines, or a machine with a defect no total
/// could describe. Both readings are handed back for the caller to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MixedProgress {
    /// The running total.
    pub before: Progress,
    /// The reading that would have been added to it.
    pub later: Progress,
}

impl Progress {
    /// The instructions retired, or `None` when the machine measured its
    /// progress as time instead.
    #[inline]
    #[must_use]
    pub const fn instructions(self) -> Option<u64> {
        match self {
            Self::Instructions(count) => Some(count),
            Self::Ticks(_) => None,
        }
    }

    /// The guest time that passed, or `None` when the machine measured its
    /// progress as instructions instead.
    #[inline]
    #[must_use]
    pub const fn ticks(self) -> Option<u64> {
        match self {
            Self::Ticks(count) => Some(count),
            Self::Instructions(_) => None,
        }
    }

    /// Whether the guest failed to advance at all.
    ///
    /// One of the two questions both units answer the same way, and the one a
    /// caller asks to find out whether stepping again is worth it.
    #[inline]
    #[must_use]
    pub const fn stalled(self) -> bool {
        match self {
            Self::Instructions(count) | Self::Ticks(count) => count == 0,
        }
    }

    /// Add a later run's progress to this one, if both are in the same unit.
    ///
    /// # Errors
    ///
    /// [`MixedProgress`] when one reading is a count and the other a span of
    /// time.
    #[inline]
    pub const fn checked_add(self, later: Self) -> Result<Self, MixedProgress> {
        match (self, later) {
            (Self::Instructions(a), Self::Instructions(b)) => Ok(Self::Instructions(a.saturating_add(b))),
            (Self::Ticks(a), Self::Ticks(b)) => Ok(Self::Ticks(a.saturating_add(b))),
            (before, later) => Err(MixedProgress { before, later }),
        }
    }

    /// The number, with the unit deliberately discarded.
    ///
    /// For pacing, where the unit genuinely does not matter: a front end
    /// bounding how much it does per frame wants to know how much happened,
    /// not what kind. Anything that will compare against an instruction count
    /// or a deadline must ask [`Self::instructions`] or [`Self::ticks`] and
    /// handle the `None`: discarding the unit there compares a span of time
    /// with a count.
    #[inline]
    #[must_use]
    pub const fn count(self) -> u64 {
        match self {
            Self::Instructions(count) | Self::Ticks(count) => count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_in_one_unit_adds_and_saturates() {
        assert_eq!(
            Progress::Instructions(3).checked_add(Progress::Instructions(4)),
            Ok(Progress::Instructions(7))
        );
        assert_eq!(
            Progress::Ticks(u64::MAX).checked_add(Progress::Ticks(1)),
            Ok(Progress::Ticks(u64::MAX))
        );
    }

    #[test]
    fn a_count_and_a_duration_do_not_add() {
        let mixed = Progress::Instructions(1).checked_add(Progress::Ticks(2));
        assert_eq!(
            mixed,
            Err(MixedProgress {
                before: Progress::Instructions(1),
                later: Progress::Ticks(2),
            })
        );
    }

    #[test]
    fn each_unit_answers_only_for_itself() {
        assert_eq!(Progress::Instructions(5).instructions(), Some(5));
        assert_eq!(Progress::Instructions(5).ticks(), None);
        assert_eq!(Progress::Ticks(5).ticks(), Some(5));
        assert!(Progress::Ticks(0).stalled());
        assert!(!Progress::Instructions(1).stalled());
        assert_eq!(Progress::Ticks(9).count(), 9);
    }
}
