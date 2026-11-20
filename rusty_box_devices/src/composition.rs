//! Narrow capabilities for independently implemented devices.

pub use rusty_box_core::events::{Emit, Reaction, Subscriptions, publish_events};

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

/// A scalar register model that can use DMA and drive an interrupt line.
/// A write returns an owned notification sequence. An optional event or a
/// fixed array can be used without allocation. Commit device effects in
/// the write itself, before subscriber delivery begins.
pub trait MmioModel {
    type Events: IntoIterator;

    fn read(&mut self, offset: WindowOffset, len: usize) -> u64;

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut impl DmaWrite,
        irq: &mut impl InterruptOutput,
    ) -> Self::Events;
}

/// The notification type delivered by a register model.
pub type MmioEvent<D> = <<D as MmioModel>::Events as IntoIterator>::Item;

/// A register model that needs one interrupt output. Implementing this
/// trait gives the model a DMA-free implementation of [`MmioModel`].
pub trait IrqMmioModel {
    type Events: IntoIterator;

    fn read(&mut self, offset: WindowOffset, len: usize) -> u64;

    fn write(&mut self, access: MmioWrite, irq: &mut impl InterruptOutput) -> Self::Events;
}

impl<D: IrqMmioModel> MmioModel for D {
    type Events = D::Events;

    #[inline]
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        IrqMmioModel::read(self, offset, len)
    }

    #[inline]
    fn write(
        &mut self,
        access: MmioWrite,
        _: &mut impl DmaWrite,
        irq: &mut impl InterruptOutput,
    ) -> Self::Events {
        IrqMmioModel::write(self, access, irq)
    }
}

/// Publish a model's notifications after its write returns. The publisher
/// lends subscribers the IRQ output while the model and DMA remain separate.
/// A subscriber failure leaves the model's completed operation in effect.
pub fn write_and_publish<D: MmioModel, I: InterruptOutput, S: Emit<MmioEvent<D>, I>>(
    model: &mut D,
    access: MmioWrite,
    dma: &mut impl DmaWrite,
    irq: &mut I,
    subscribers: &mut S,
) -> Result<Reaction, S::Error> {
    publish_events(model.write(access, dma, irq), subscribers, irq)
}
