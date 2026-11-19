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

/// A scalar register model that may return an owned notification after a
/// write. A model with several notifications can use its own enum. The
/// model borrows DMA and IRQ only for the duration of the operation.
pub trait MmioModel {
    type Event;

    fn read(&mut self, offset: WindowOffset, len: usize) -> u64;

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut impl DmaWrite,
        irq: &mut impl InterruptOutput,
    ) -> Option<Self::Event>;
}

/// Publish a model's notification after its write returns. The publisher
/// lends subscribers the IRQ output while the model and DMA remain separate.
/// A subscriber failure leaves the model's completed operation in effect.
pub fn write_and_publish<D: MmioModel, I: InterruptOutput, S: Emit<D::Event, I>>(
    model: &mut D,
    access: MmioWrite,
    dma: &mut impl DmaWrite,
    irq: &mut I,
    subscribers: &mut S,
) -> Result<Reaction, S::Error> {
    match model.write(access, dma, irq) {
        Some(event) => subscribers.emit(&event, irq),
        None => Ok(Reaction::Continue),
    }
}
