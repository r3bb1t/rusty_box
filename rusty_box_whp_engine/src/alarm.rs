//! Getting the processor back from a guest that has stopped asking for
//! anything.
//!
//! Every other way this engine regains control is something the guest did — a
//! port access, an access outside the map, a halt. A guest in a loop that
//! touches none of those does not come back at all, and `WHvRunVirtualProcessor`
//! is a blocking call: the deadline the machine gave the slice is checked
//! between exits, and between exits is exactly where such a guest never is.
//!
//! So the deadline needs a voice of its own. The platform documents
//! `WHvCancelRunVirtualProcessor` as callable from a thread other than the one
//! inside the run, which is the whole of what this needs: one thread, asleep
//! until a deadline, whose only act is to ask the processor to come back.
//!
//! ## Why a thread and not a timer callback
//!
//! There is no callback to hang this on. The machine's own timer wheel runs on
//! the thread that is currently inside the run, so it cannot fire while the
//! run is in progress — that is the problem, not the solution.
//!
//! One thread for the engine's whole life, not one per slice: arming is a lock
//! and a notify, and a thread per slice would cost more to create than the
//! slice it was guarding.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use rusty_box_whp::Canceller;

/// A thread that interrupts a running processor when its slice runs out.
///
/// Dropping it stops the thread and waits for it, which is why it must be
/// dropped BEFORE the partition it cancels: a canceller names a handle the
/// platform reclaims when the partition goes, and the field order of whatever
/// owns both is what guarantees the ordering.
pub(crate) struct Alarm {
    shared: Arc<Shared>,
    /// `None` only while the alarm is being dropped.
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    at: Mutex<State>,
    changed: Condvar,
}

/// What the sleeping thread is waiting for.
///
/// A state rather than two booleans (R2): the thread is either idle, waiting
/// for a deadline, or being shut down, and no pair of flags can spell a fourth
/// thing.
enum State {
    /// No slice is running; nothing to interrupt.
    Idle,
    /// A slice is running and must be interrupted at this instant.
    Armed(Instant),
    /// The engine is going away.
    Finished,
}

impl Alarm {
    /// Start the thread that watches for `canceller`'s processor overrunning.
    pub(crate) fn watching(canceller: Canceller) -> Self {
        let shared = Arc::new(Shared {
            at: Mutex::new(State::Idle),
            changed: Condvar::new(),
        });
        let mine = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("whp slice alarm".into())
            .spawn(move || watch(&mine, canceller))
            .ok();
        Self { shared, thread }
    }

    /// Interrupt the running processor if it is still running at `deadline`.
    pub(crate) fn arm(&self, deadline: Instant) {
        self.set(State::Armed(deadline));
    }

    /// The slice is over; stop watching for it.
    ///
    /// Called whatever ended the slice, including an error, so a deadline
    /// belonging to a slice that has finished cannot interrupt the next one.
    pub(crate) fn disarm(&self) {
        self.set(State::Idle);
    }

    fn set(&self, state: State) {
        // A poisoned lock means the watching thread panicked between taking
        // the lock and releasing it, and the state behind it is a plain enum
        // that no panic can leave half-written. Recovering is therefore
        // strictly better than propagating a panic into a running guest.
        match self.shared.at.lock() {
            Ok(mut at) => *at = state,
            Err(poisoned) => *poisoned.into_inner() = state,
        }
        self.shared.changed.notify_one();
    }
}

impl Drop for Alarm {
    fn drop(&mut self) {
        self.set(State::Finished);
        if let Some(thread) = self.thread.take() {
            // Joined rather than detached: the canceller this thread holds
            // names a partition that is about to be destroyed, and a cancel
            // arriving after that names a handle the platform has reclaimed.
            let _joined = thread.join();
        }
    }
}

/// Sleep until a deadline passes, then ask the processor to come back.
fn watch(shared: &Shared, canceller: Canceller) {
    let mut at = match shared.at.lock() {
        Ok(at) => at,
        Err(poisoned) => poisoned.into_inner(),
    };
    loop {
        let deadline = match *at {
            State::Finished => return,
            State::Idle => {
                at = match shared.changed.wait(at) {
                    Ok(at) => at,
                    Err(poisoned) => poisoned.into_inner(),
                };
                continue;
            }
            State::Armed(deadline) => deadline,
        };

        let now = Instant::now();
        if now < deadline {
            let (guard, _timed_out) = match shared
                .changed
                .wait_timeout(at, deadline.saturating_duration_since(now))
            {
                Ok(pair) => pair,
                Err(poisoned) => poisoned.into_inner(),
            };
            at = guard;
            // Re-read rather than assume the wait timed out: the slice may
            // have finished and armed a later deadline while this thread was
            // asleep, and cancelling then would interrupt a slice that had
            // barely started.
            continue;
        }

        // The deadline has passed and the slice has not disarmed, so the guest
        // is still inside the run. Go idle first: the cancel is sticky, and
        // firing twice for one deadline would cost the next slice an exit.
        *at = State::Idle;
        drop(at);
        if let Err(error) = canceller.cancel() {
            tracing::error!("could not interrupt an overrunning slice: {error}");
        }
        at = match shared.at.lock() {
            Ok(at) => at,
            Err(poisoned) => poisoned.into_inner(),
        };
    }
}
