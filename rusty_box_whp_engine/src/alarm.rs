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
//! inside the run, which is the whole of what this needs: one thread waiting
//! on a deadline, whose only act is to ask the processor to come back.
//!
//! ## Why the wait has two phases
//!
//! The waiting is slept in bulk and spun at the end, because a slept wait
//! cannot end a slice on time — the host's timer tick is 15.6 ms and a slice
//! is measured in microseconds. [`SPIN_MARGIN`] carries the measurement and
//! the cost.
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_box_whp::Canceller;

/// How much of a wait is spun rather than slept.
///
/// A timed wait on a condition variable cannot return sooner than the host's
/// timer tick. Measured here: asked for 50 microseconds it returns after
/// 15,467 on average — 309 times the request, and the same 15.6 ms tick
/// whatever it is asked for. Raising the period to a millisecond brings that
/// to 1,692 microseconds, still 34 times over. Spinning the same 50
/// microseconds lands in 59.9.
///
/// So the wait has two phases: everything beyond this margin is slept, and
/// the margin itself is spun. A slice shorter than the margin — which is
/// most of them, since a machine asks for the time to its next device
/// deadline — is spun whole and ends when it was asked to.
///
/// The cost is a core busy for as long as the guest is on the hardware, and
/// it buys the only thing that makes a hardware slice honest: a guest whose
/// timer pulses arrive at their deadline rather than in bursts at slice
/// boundaries. Linux notices the difference during interrupt setup and
/// refuses to boot without it.
const SPIN_MARGIN: Duration = Duration::from_millis(2);

/// A wait longer than [`SPIN_MARGIN`] still rides the host's own timer tick
/// for the part of it that is slept, and so can end late by as much as that
/// tick. The margin is what a slice is measured against, and a slice this
/// engine hands the hardware is shorter than the margin, so it is spun whole
/// and ends when it was asked to. Only a budget larger than the margin — a
/// guest that has halted, and is not waiting on this — takes the slept path.

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
    /// Bumped by every change to `at`.
    ///
    /// The spinning phase of a wait holds no lock — it cannot, because the
    /// thread arming the next slice would then block behind a thread that is
    /// deliberately busy. So it watches this instead, and a slice that ends
    /// early is noticed within one turn of the loop rather than at its
    /// deadline.
    generation: AtomicU64,
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
            generation: AtomicU64::new(0),
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
        // Published after the state it describes, and read before a spinning
        // watcher acts on the deadline it was holding, so the deadline of a
        // slice that has already ended cannot interrupt the one after it.
        self.shared.generation.fetch_add(1, Ordering::Release);
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

/// Wait until a deadline passes, then ask the processor to come back.
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

        if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if let Some(sleepable) = remaining.checked_sub(SPIN_MARGIN) {
                let (guard, _timed_out) = match shared.changed.wait_timeout(at, sleepable) {
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

            // Inside the margin, so the rest is spun. The lock is released
            // first: this thread is about to be deliberately busy, and the
            // thread that ends the slice must not queue behind it.
            let generation = shared.generation.load(Ordering::Acquire);
            drop(at);
            while Instant::now() < deadline
                && shared.generation.load(Ordering::Acquire) == generation
            {
                std::hint::spin_loop();
            }
            at = match shared.at.lock() {
                Ok(at) => at,
                Err(poisoned) => poisoned.into_inner(),
            };
            // Re-read for the same reason the slept phase does, and for one
            // more: the spin ends on a changed generation as well as on the
            // deadline, and only the state says which happened.
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
