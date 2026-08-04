//! BENCHMARK-ONLY (temporary, campaign instrumentation — mirrors the same
//! patch in the Bochs bench worktree): per-vector interrupt-delivery
//! histogram. Slots 0-255 count LAPIC `acknowledge_int` vectors, 256-511
//! count PIC `iac` vectors. Snapshots are written into the
//! `RUSTY_BOX_BENCH_FILE` sample stream so counts can be read at any icount.
//! One relaxed fetch_add per delivered interrupt; removed when the
//! perf-parity campaign ends.

use core::sync::atomic::{AtomicU64, Ordering};

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static VEC_COUNTS: [AtomicU64; 512] = [ZERO; 512];

/// Record one delivered interrupt. `index` = vector for LAPIC deliveries,
/// `256 + vector` for PIC deliveries.
#[inline]
pub(crate) fn count(index: usize) {
    VEC_COUNTS[index & 0x1ff].fetch_add(1, Ordering::Relaxed);
}

/// Snapshot every non-zero slot as `(index, count)`. Only the
/// `RUSTY_BOX_BENCH_FILE` sampler calls this, and that is std-only.
#[cfg(feature = "std")]
pub(crate) fn snapshot(mut visit: impl FnMut(usize, u64)) {
    for (index, slot) in VEC_COUNTS.iter().enumerate() {
        let value = slot.load(Ordering::Relaxed);
        if value != 0 {
            visit(index, value);
        }
    }
}
