//! What a thread waits a device deadline on.
//!
//! ## Why not a condition variable
//!
//! **Measured on this host.** `SleepConditionVariableSRW` — what backs Rust's
//! `Condvar::wait_timeout` — returns after the system's ~15.6 ms timer tick
//! whatever it asks for. The request does not appear in the answer; the tick
//! does:
//!
//! | asked | condvar mean | worst | high-resolution timer | worst |
//! |-------|--------------|-------|------------------------|-------|
//! | 500 µs| —            | —     | 0.752 ms               | 1.328 |
//! | 1 ms  | 12.172 ms    | 15.945| 1.327 ms               | 1.595 |
//! | 5 ms  | 9.675 ms     | 19.712| 5.313 ms               | 5.501 |
//! | 10 ms | 17.177 ms    | 29.563| 10.355 ms              | 10.877|
//!
//! Every device deadline on a PC is nearer than the default tick — a PIT at
//! 1 kHz is 1 ms away, the 8042's serial delay 150 µs — so a thread that waited
//! on a condition variable would deliver their pulses in BURSTS one tick apart.
//! That is the timing failure the slice model had, and a guest calibrating its
//! timers notices it immediately.
//!
//! ## Why not `timeBeginPeriod`
//!
//! It is what QEMU does (`os-win32.c os_setup_early_signal_handling`, an
//! unconditional `timeBeginPeriod(mm_tc.wPeriodMin)` at startup with an
//! `atexit` to undo it), and it was tried here and measured to work — 1 ms
//! waits landed at 1.053 ms. It is still the wrong instrument for this crate,
//! for three reasons:
//!
//! - It raises a setting for the whole PROCESS. QEMU may do that because QEMU
//!   is the application; this is a library a front end embeds, and a library
//!   has no business changing how every other timer in its host behaves.
//! - It floors at one millisecond, so the 8042's 150 µs deadline cannot be
//!   expressed through it at all.
//! - Since Windows 10 2004 the grant is per-process, and Windows 11 revokes it
//!   for a process whose window is occluded or minimised unless it opts out
//!   with `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION` — so a
//!   GUI-hosted machine could silently lose its timing when the user looks
//!   away.
//!
//! A high-resolution waitable timer has none of those properties and measured
//! at least as well. QEMU has no such path anywhere in its Windows layer.
//!
//! ## Provenance (R7)
//!
//! Bochs has no counterpart at all: it advances device time from its own CPU
//! loop — `BX_TICKN(delta)` in `cpu/cpu.cc BX_SYNC_TIME_IF_SINGLE_PROCESSOR`,
//! where `delta` is instructions retired — so it never waits on a host clock.
//! VirtualBox waits on the emulation thread and compensates empirically
//! (`vmR3HaltOldDoHalt` spins below 50 µs and subtracts a running average of
//! measured oversleep); it has no device thread to serve.

use std::time::Duration;

use crate::sys;

pub use crate::sys::DeadlineWake;

/// A deadline a thread can wait on, and a doorbell for changing its mind.
///
/// Owns its host handles and releases them on drop, which is what keeps the
/// seam's `close_deadline` obligation off every caller (R1).
#[derive(Debug)]
pub struct DeadlineTimer {
    /// `None` when the platform would not create one — an older Windows, or a
    /// host with no timer service. The machine still runs: [`Self::wait`]
    /// falls back to a condition variable, whose accuracy is the table above's
    /// left half. Reported by [`Self::is_high_resolution`] so a caller that
    /// cares can say so rather than guess.
    raw: Option<sys::RawDeadline>,
    /// The fallback's state: a generation the doorbell bumps, so a ring that
    /// lands between two waits is not lost.
    rung: std::sync::Mutex<u64>,
    doorbell: std::sync::Condvar,
}

impl DeadlineTimer {
    /// Create one, falling back to a condition variable if the platform
    /// refuses.
    ///
    /// Never fails the caller: a machine whose deadlines are served at the
    /// system's default tick is slower to deliver device pulses, not wrong.
    #[must_use]
    pub fn new() -> Self {
        let raw = match sys::create_deadline() {
            Ok(raw) => Some(raw),
            Err(error) => {
                tracing::warn!(
                    "no high-resolution waitable timer on this host, so device deadlines are \
                     served at the system's default ~15.6 ms tick: {error}"
                );
                None
            }
        };
        Self {
            raw,
            rung: std::sync::Mutex::new(0),
            doorbell: std::sync::Condvar::new(),
        }
    }

    /// Whether deadlines are served by the platform's high-resolution timer.
    #[must_use]
    pub fn is_high_resolution(&self) -> bool {
        self.raw.is_some()
    }

    /// Wait until `after` has passed or someone rings, whichever comes first.
    ///
    /// Answers which it was, so a caller can tell "what I waited for is due"
    /// from "someone changed my mind" without inspecting shared state twice.
    pub fn wait(&self, after: Duration) -> DeadlineWake {
        match self.raw {
            Some(raw) => self.wait_on_platform(raw, after),
            None => self.wait_on_condvar(after),
        }
    }

    /// Wake a waiter now.
    ///
    /// Rings whichever mechanism the waiter is on, so a caller never has to
    /// know which was available.
    pub fn ring(&self) {
        {
            let mut rung = self
                .rung
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *rung = rung.saturating_add(1);
        }
        self.doorbell.notify_all();
        if let Some(raw) = self.raw {
            match sys::ring_deadline(raw) {
                Ok(()) => {}
                Err(error) => tracing::error!("a device deadline's doorbell would not ring: {error}"),
            }
        }
    }

    fn wait_on_platform(&self, raw: sys::RawDeadline, after: Duration) -> DeadlineWake {
        let nanos = u64::try_from(after.as_nanos()).unwrap_or(u64::MAX);
        match sys::arm_deadline(raw, nanos) {
            Ok(()) => {}
            Err(error) => {
                tracing::error!("a device deadline would not arm, waiting without it: {error}");
                return self.wait_on_condvar(after);
            }
        }
        match sys::wait_deadline(raw) {
            Ok(wake) => wake,
            Err(error) => {
                tracing::error!("a device deadline's wait failed: {error}");
                DeadlineWake::Rung
            }
        }
    }

    /// The fallback. Watches the ring generation rather than a flag, so a ring
    /// that arrives between two waits is still seen by the next one.
    fn wait_on_condvar(&self, after: Duration) -> DeadlineWake {
        let rung = self
            .rung
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = *rung;
        let (rung, timed_out) = self
            .doorbell
            .wait_timeout_while(rung, after, |now| *now == before)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(rung);
        if timed_out.timed_out() {
            DeadlineWake::Deadline
        } else {
            DeadlineWake::Rung
        }
    }
}

impl Default for DeadlineTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DeadlineTimer {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            // SAFETY: `raw` is this value's own pair, taken here so it cannot
            // be closed twice, and no waiter can be inside a call on it — a
            // `&mut self` drop means nobody else holds a reference.
            #[expect(
                unsafe_code,
                reason = "UNSAFETY: discharging the seam's close-once obligation, which this \
                          type owns because it owns the handles"
            )]
            unsafe {
                sys::close_deadline(raw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A wait for a deadline ends at the deadline, and says so.
    ///
    /// The number is the point: at the system's default resolution this wait
    /// would return in ~15.6 ms whatever it asked for, which is what makes
    /// every sub-tick device deadline undeliverable.
    #[test]
    fn a_deadline_wait_ends_near_the_deadline_it_asked_for() {
        let timer = DeadlineTimer::new();
        if !timer.is_high_resolution() {
            return; // the fallback's accuracy is the tick's, by construction
        }
        let began = Instant::now();
        let wake = timer.wait(Duration::from_millis(2));
        let waited = began.elapsed();
        assert_eq!(wake, DeadlineWake::Deadline);
        assert!(
            waited < Duration::from_millis(10),
            "a 2 ms deadline must not wait a whole 15.6 ms system tick: {waited:?}"
        );
    }

    /// A ring ends the wait early and is distinguishable from the deadline.
    ///
    /// Both halves matter: a waiter that could not tell them apart would
    /// service a deadline that had not arrived.
    #[test]
    fn a_ring_ends_the_wait_before_the_deadline_and_says_which() {
        let timer = std::sync::Arc::new(DeadlineTimer::new());
        let ringer = std::sync::Arc::clone(&timer);
        let hand = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            ringer.ring();
        });
        let began = Instant::now();
        let wake = timer.wait(Duration::from_secs(30));
        let waited = began.elapsed();
        hand.join().expect("the ringing thread");
        assert_eq!(wake, DeadlineWake::Rung, "the doorbell ended it, not the deadline");
        assert!(
            waited < Duration::from_secs(5),
            "the ring must not wait out a 30 s deadline: {waited:?}"
        );
    }
}
