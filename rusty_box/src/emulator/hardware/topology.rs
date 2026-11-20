//! Fixed, page-aligned windows on the software PC's memory bus.

use core::num::NonZeroU16;

use rusty_box_devices::composition::{MmioEvent, publish_events};

use super::{
    BxPhyAddress, ComposedIrqs, Emit, EventEndpoint, IsaIrq, MmioModel, MmioWrite, PcDevice,
    PcDeviceAttachRefusal, PcDma, PcInterrupt, PcMmioDevice, Reaction, SubscriberError,
    Subscriptions, WindowOffset,
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

    /// Place `next` after this window. A refusal returns both original windows,
    /// including their models, routing, and captured subscriptions.
    pub fn then<N: PcMmioDevice>(
        self,
        next: MmioWindow<N>,
    ) -> Result<MmioWindow<WindowPair<D, N>>, MmioJoinError<D, N>> {
        if self.lines & next.lines != 0 {
            return Err(MmioJoinError {
                reason: MmioTopologyError::DuplicateIrq,
                left: self,
                right: next,
            });
        }
        let Some(size) = self.size.checked_add(next.size) else {
            return Err(MmioJoinError {
                reason: MmioTopologyError::SizeOverflow,
                left: self,
                right: next,
            });
        };
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

    fn map_device<N>(self, map: impl FnOnce(D) -> N) -> MmioWindow<N> {
        MmioWindow {
            size: self.size,
            lines: self.lines,
            device: map(self.device),
        }
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

impl<D: MmioModel + Send> MmioWindow<IrqWindow<EventEndpoint<D>>> {
    /// Put a shared device model on one page with the selected PC IRQ output.
    pub fn with_model(line: IsaIrq, model: D) -> Self {
        MmioWindow::with_irq(NonZeroU16::MIN, line, EventEndpoint::new(model))
    }
}

impl<D: PcMmioDevice> MmioWindow<IrqWindow<D>> {
    /// Set the page count for this leaf before composing it into a bus.
    pub fn with_pages(mut self, pages: NonZeroU16) -> Self {
        self.size = u64::from(pages.get()) * 4096;
        self
    }

    /// Change this leaf's board wiring without replacing its model or handlers.
    pub fn with_irq_line(mut self, line: IsaIrq) -> Self {
        self.lines = line.mask();
        self.device.line = line;
        self
    }
}

impl<D, S> MmioWindow<IrqWindow<EventEndpoint<D, S>>>
where
    D: MmioModel + Send,
    S: for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
{
    /// Observe notifications from this instance.
    pub fn on<F: FnMut(&MmioEvent<D>) + Send>(
        self,
        handler: F,
    ) -> MmioWindow<
        IrqWindow<
            EventEndpoint<
                D,
                impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
            >,
        >,
    > {
        self.map_endpoint(|endpoint| endpoint.on(handler))
    }

    /// Observe notifications and optionally request a stop or report a failure.
    pub fn try_on<F: FnMut(&MmioEvent<D>) -> Result<Reaction, SubscriberError> + Send>(
        self,
        handler: F,
    ) -> MmioWindow<
        IrqWindow<
            EventEndpoint<
                D,
                impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
            >,
        >,
    > {
        self.map_endpoint(|endpoint| endpoint.try_on(handler))
    }

    /// Lend this leaf's IRQ output to a subscriber for one notification.
    pub fn on_with_irq<F>(
        self,
        handler: F,
    ) -> MmioWindow<
        IrqWindow<
            EventEndpoint<
                D,
                impl for<'a> Emit<MmioEvent<D>, PcInterrupt<'a>, Error = SubscriberError> + Send,
            >,
        >,
    >
    where
        F: for<'a> FnMut(&MmioEvent<D>, &mut PcInterrupt<'a>) -> Result<Reaction, SubscriberError>
            + Send,
    {
        self.map_endpoint(|endpoint| endpoint.on_with_irq(handler))
    }

    fn map_endpoint<N>(
        self,
        map: impl FnOnce(EventEndpoint<D, S>) -> EventEndpoint<D, N>,
    ) -> MmioWindow<IrqWindow<EventEndpoint<D, N>>> {
        self.map_device(|IrqWindow { device, line }| IrqWindow {
            device: map(device),
            line,
        })
    }
}

impl<D, R, W, E> MmioWindow<RegisterWindow<D, R, W, Subscriptions<E::Item, SubscriberError>>>
where
    D: Send,
    R: FnMut(&mut D, WindowOffset, usize) -> u64 + Send,
    W: FnMut(&mut D, MmioWrite) -> E + Send,
    E: IntoIterator,
{
    /// Put ordinary model functions on one page. Handlers can be added with
    /// `on` or `try_on` when the model publishes events.
    pub fn registers(model: D, read: R, write: W) -> Self {
        register_window(NonZeroU16::MIN, model, read, write, Subscriptions::new())
    }
}

impl<D, R, W, E, S> MmioWindow<RegisterWindow<D, R, W, S>>
where
    D: Send,
    R: FnMut(&mut D, WindowOffset, usize) -> u64 + Send,
    W: FnMut(&mut D, MmioWrite) -> E + Send,
    E: IntoIterator,
    S: Emit<E::Item, Error = SubscriberError> + Send,
{
    /// Set the page count for this leaf before composing it into a bus.
    pub fn with_pages(mut self, pages: NonZeroU16) -> Self {
        self.size = u64::from(pages.get()) * 4096;
        self
    }

    /// Observe notifications from this instance.
    pub fn on<F: FnMut(&E::Item) + Send>(
        self,
        handler: F,
    ) -> MmioWindow<RegisterWindow<D, R, W, impl Emit<E::Item, Error = SubscriberError> + Send>>
    {
        self.map_subscribers(|previous| Subscriptions::new().forward(previous).on(handler))
    }

    /// Observe notifications and optionally request a stop or report a failure.
    pub fn try_on<F: FnMut(&E::Item) -> Result<Reaction, SubscriberError> + Send>(
        self,
        handler: F,
    ) -> MmioWindow<RegisterWindow<D, R, W, impl Emit<E::Item, Error = SubscriberError> + Send>>
    {
        self.map_subscribers(|previous| Subscriptions::new().forward(previous).try_on(handler))
    }

    fn map_subscribers<N>(
        self,
        map: impl FnOnce(S) -> N,
    ) -> MmioWindow<RegisterWindow<D, R, W, N>> {
        self.map_device(|device| RegisterWindow {
            model: device.model,
            read: device.read,
            write: device.write,
            subscribers: map(device.subscribers),
        })
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

/// A rejected join and the two windows that can be used to try again.
#[derive(thiserror::Error)]
#[error("{reason}")]
pub struct MmioJoinError<L, R> {
    pub reason: MmioTopologyError,
    pub left: MmioWindow<L>,
    pub right: MmioWindow<R>,
}

impl<L, R> core::fmt::Debug for MmioJoinError<L, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.reason.fmt(f)
    }
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
