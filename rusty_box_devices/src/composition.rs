//! Narrow capabilities for independently implemented devices.

pub use rusty_box_core::events::{Emit, Reaction, Subscriptions};

use crate::api::{BxPhyAddress, WindowOffset};

/// Write bytes into guest physical RAM.
///
/// A successful call writes all bytes. A failed call writes none. A backend
/// may refuse apertures, unsupported widths, or unavailable backing. The
/// implementation is responsible for instruction-cache coherence.
pub trait DmaWrite {
    type Error;

    fn write(&mut self, address: BxPhyAddress, bytes: &[u8]) -> Result<(), Self::Error>;
}

/// Drive one interrupt output. The board chooses how and where it is routed.
pub trait InterruptOutput {
    fn set_level(&mut self, asserted: bool);
}

/// One scalar little-endian write relative to a mapped device window.
/// A bus may split an access into several smaller writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioWrite {
    pub offset: WindowOffset,
    pub len: usize,
    pub value: u64,
}
