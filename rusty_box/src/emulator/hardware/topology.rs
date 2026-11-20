//! Fixed, page-aligned windows on the software PC's memory bus.

use core::num::NonZeroU16;

use rusty_box_devices::composition::publish_events;

use super::{
    BxPhyAddress, ComposedIrqs, Emit, IsaIrq, MmioWrite, PcDevice, PcDeviceAttachRefusal, PcDma,
    PcInterrupt, PcMmioDevice, Reaction, SubscriberError, WindowOffset,
};
use crate::Emulator;
use crate::cpu::Instrumentation;

/// An owned subtree of a PC memory aperture. Each child's offsets start at zero.
///
/// Compose adjacent child windows with [`Self::then`]. Children and handlers
/// are concrete Rust values. Mounting the root uses one registry entry, with
/// an optional box or caller-supplied stable storage.
pub struct MmioWindow<D> {
    size: u64,
    lines: u16,
    device: D,
}

impl<D: PcMmioDevice> MmioWindow<D> {
    /// Wire an endpoint to one board-selected output.
    pub fn with_irq(pages: NonZeroU16, line: IsaIrq, device: D) -> MmioWindow<IrqWindow<D>> {
        MmioWindow {
            size: u64::from(pages.get()) * 4096,
            lines: line.mask(),
            device: IrqWindow { device, line },
        }
    }

    /// Place `next` after this window. Duplicate interrupt lines are rejected.
    pub fn then<N: PcMmioDevice>(
        self,
        next: MmioWindow<N>,
    ) -> Result<MmioWindow<WindowPair<D, N>>, MmioTopologyError> {
        if self.lines & next.lines != 0 {
            return Err(MmioTopologyError::DuplicateIrq);
        }
        let size = self
            .size
            .checked_add(next.size)
            .ok_or(MmioTopologyError::SizeOverflow)?;
        Ok(MmioWindow {
            size,
            lines: self.lines | next.lines,
            device: WindowPair {
                split: self.size,
                left: self.device,
                right: next.device,
            },
        })
    }

    /// Install the complete aperture. A refusal returns the typed subtree.
    #[cfg(feature = "alloc")]
    pub fn install<T: Instrumentation>(
        self,
        machine: &mut Emulator<T>,
        base: BxPhyAddress,
    ) -> Result<(), MmioBusRefused<Self>>
    where
        D: 'static,
    {
        let slot = match machine.mmio_bus_slot(base, self.size, self.lines) {
            Ok(slot) => slot,
            Err(reason) => {
                return Err(MmioBusRefused {
                    reason,
                    window: self,
                });
            }
        };
        machine.insert_mmio_bus(
            slot,
            base,
            self.size,
            self.lines,
            PcDevice::owned(self.device),
        );
        Ok(())
    }

    /// Install using stable caller storage, without allocating.
    pub fn install_borrowed<T: Instrumentation>(
        &'static mut self,
        machine: &mut Emulator<T>,
        base: BxPhyAddress,
    ) -> Result<(), MmioBusRefused<&'static mut Self>> {
        let slot = match machine.mmio_bus_slot(base, self.size, self.lines) {
            Ok(slot) => slot,
            Err(reason) => {
                return Err(MmioBusRefused {
                    reason,
                    window: self,
                });
            }
        };
        machine.insert_mmio_bus(
            slot,
            base,
            self.size,
            self.lines,
            PcDevice::Borrowed(&mut self.device),
        );
        Ok(())
    }
}

/// Make a register-only window from ordinary methods and a local subscriber
/// route. Both functions see only device state and the local register access.
pub fn register_window<D, R, W, E, S>(
    pages: NonZeroU16,
    model: D,
    read: R,
    write: W,
    subscribers: S,
) -> MmioWindow<RegisterWindow<D, R, W, S>>
where
    D: Send,
    R: FnMut(&mut D, WindowOffset, usize) -> u64 + Send,
    W: FnMut(&mut D, MmioWrite) -> E + Send,
    E: IntoIterator,
    S: Emit<E::Item, Error = SubscriberError> + Send,
{
    MmioWindow {
        size: u64::from(pages.get()) * 4096,
        lines: 0,
        device: RegisterWindow {
            model,
            read,
            write,
            subscribers,
        },
    }
}

/// Storage for a register-only leaf returned by [`register_window`].
pub struct RegisterWindow<D, R, W, S> {
    model: D,
    read: R,
    write: W,
    subscribers: S,
}

impl<D, R, W, E, S> PcMmioDevice for RegisterWindow<D, R, W, S>
where
    D: Send,
    R: FnMut(&mut D, WindowOffset, usize) -> u64 + Send,
    W: FnMut(&mut D, MmioWrite) -> E + Send,
    E: IntoIterator,
    S: Emit<E::Item, Error = SubscriberError> + Send,
{
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        (self.read)(&mut self.model, offset, len)
    }

    fn write(
        &mut self,
        access: MmioWrite,
        _: &mut PcDma<'_>,
        _: &mut PcInterrupt<'_>,
    ) -> Result<Reaction, SubscriberError> {
        publish_events(
            (self.write)(&mut self.model, access),
            &mut self.subscribers,
            &mut (),
        )
    }
}

/// Storage for a leaf returned by [`MmioWindow::with_irq`].
pub struct IrqWindow<D> {
    device: D,
    line: IsaIrq,
}

impl<D: PcMmioDevice> PcMmioDevice for IrqWindow<D> {
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        self.device.read(offset, len)
    }

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut PcDma<'_>,
        irqs: &mut PcInterrupt<'_>,
    ) -> Result<Reaction, SubscriberError> {
        irqs.with_line(self.line, |irq| self.device.write(access, dma, irq))
    }
}

/// Storage for adjacent child windows returned by [`MmioWindow::then`].
pub struct WindowPair<L, R> {
    split: u64,
    left: L,
    right: R,
}

impl<L: PcMmioDevice, R: PcMmioDevice> PcMmioDevice for WindowPair<L, R> {
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        if offset.get() < self.split {
            self.left.read(offset, len)
        } else {
            self.right
                .read(WindowOffset(offset.get() - self.split), len)
        }
    }

    fn write(
        &mut self,
        mut access: MmioWrite,
        dma: &mut PcDma<'_>,
        irqs: &mut PcInterrupt<'_>,
    ) -> Result<Reaction, SubscriberError> {
        if access.offset.get() < self.split {
            self.left.write(access, dma, irqs)
        } else {
            access.offset = WindowOffset(access.offset.get() - self.split);
            self.right.write(access, dma, irqs)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MmioTopologyError {
    #[error("two windows drive the same ISA line")]
    DuplicateIrq,
    #[error("MMIO aperture size overflows the address space")]
    SizeOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MmioBusAttachRefusal {
    #[error(transparent)]
    Placement(#[from] PcDeviceAttachRefusal),
    #[error("MMIO aperture wraps the physical address space")]
    AddressOverflow,
}

/// An installation failed before changing mappings or interrupt ownership.
#[derive(thiserror::Error)]
#[error("{reason}")]
pub struct MmioBusRefused<W> {
    pub reason: MmioBusAttachRefusal,
    pub window: W,
}

impl<W> core::fmt::Debug for MmioBusRefused<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.reason.fmt(f)
    }
}

/// A detached root retains its devices, subscriptions and board IRQ wiring.
pub struct DetachedMmioBus {
    pub(crate) size: u64,
    pub(crate) irqs: ComposedIrqs,
    pub(crate) device: PcDevice,
}

impl DetachedMmioBus {
    /// Place the subtree again, possibly at another physical address.
    pub fn install<T: Instrumentation>(
        self,
        machine: &mut Emulator<T>,
        base: BxPhyAddress,
    ) -> Result<(), MmioBusRefused<Self>> {
        let lines = self.irqs.mask();
        let slot = match machine.mmio_bus_slot(base, self.size, lines) {
            Ok(slot) => slot,
            Err(reason) => {
                return Err(MmioBusRefused {
                    reason,
                    window: self,
                });
            }
        };
        machine.insert_mmio_bus(slot, base, self.size, lines, self.device);
        Ok(())
    }
}
