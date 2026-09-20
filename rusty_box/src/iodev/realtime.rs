//! Host time for the devices that keep it under `clock: sync=realtime` —
//! Bochs `bx_virt_timer.time_usec(true)`, the realtime virtual clock pit.cc,
//! acpi.cc and vgacore.cc read when `BXPN_CLOCK_SYNC` names realtime.

/// A device clock on host time: it read `base_usec` at the host instant
/// `anchor`, and counts on from there.
///
/// A reading at an instant rather than the instant it read zero, so resuming
/// at any saved reading only ever counts forward from now: the instant a
/// long-running machine's clock read zero can lie before anything the host's
/// clock can represent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RealtimeClock {
    anchor: std::time::Instant,
    base_usec: u64,
}

impl RealtimeClock {
    /// A clock that reads `usec` now.
    pub(crate) fn reading(usec: u64) -> Self {
        Self {
            anchor: std::time::Instant::now(),
            base_usec: usec,
        }
    }

    /// The reading now, in microseconds.
    pub(crate) fn usec(&self) -> u64 {
        let elapsed = u64::try_from(self.anchor.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.base_usec.saturating_add(elapsed)
    }
}
