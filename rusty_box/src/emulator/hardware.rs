//! Optional hardware on the software PC's scalar MMIO route.
//!
//! The board erases the placed endpoint once. Device code behind it can use
//! ordinary functions, generic capabilities and statically composed handlers.

use rusty_box_devices::api::IrqLine;
pub use rusty_box_devices::api::WindowOffset;
pub use rusty_box_devices::composition::{
    DmaWrite, Emit, InterruptOutput, IrqMmioModel, MmioModel, MmioWrite, Reaction, Subscriptions,
};

use crate::config::BxPhyAddress;
use crate::cpu::apic::BxLocalApic;
use crate::iodev::irq::IrqFabric;
use crate::memory::BxMemC;
use crate::memory::mmio::{DmaExclusions, MmioAttachRefusal};

mod event_endpoint;
pub use event_endpoint::EventEndpoint;
mod topology;
pub use topology::{
    DetachedMmioBus, IrqWindow, MmioBusAttachRefusal, MmioBusRefused, MmioTopologyError,
    MmioWindow, RegisterWindow, WindowPair, register_window,
};

/// An allocation-free diagnostic chosen by the subscriber at the PC boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct SubscriberError(pub &'static str);

/// A subscriber failed during a guest MMIO write. The host receives this
/// after the interpreter returns and the scheduler services committed effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("MMIO subscriber at {address:#x}: {source}")]
pub struct MmioSubscriberError {
    pub address: BxPhyAddress,
    pub source: SubscriberError,
}

/// An endpoint on a single-CPU software PC, with offsets local to its window.
pub trait PcMmioDevice: Send {
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64;

    /// Service the write, then deliver any notifications before returning.
    /// A stop requests a return at the next interpreter boundary. The write
    /// has taken effect and will not be replayed by that stop. An error also
    /// returns to the host at that boundary and preserves completed effects.
    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut PcDma<'_>,
        irq: &mut PcInterrupt<'_>,
    ) -> Result<Reaction, SubscriberError>;
}

/// Optional MMIO write observer. It runs after the device and its subscribers.
pub struct ObserveWrites<D, F> {
    device: D,
    observe: F,
}

impl<D, F: FnMut(MmioWrite, Result<Reaction, SubscriberError>)> ObserveWrites<D, F> {
    pub fn new(device: D, observe: F) -> Self {
        Self { device, observe }
    }
}

impl<D: PcMmioDevice, F: FnMut(MmioWrite, Result<Reaction, SubscriberError>) + Send> PcMmioDevice
    for ObserveWrites<D, F>
{
    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        self.device.read(offset, len)
    }

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut PcDma<'_>,
        irq: &mut PcInterrupt<'_>,
    ) -> Result<Reaction, SubscriberError> {
        let result = self.device.write(access, dma, irq);
        (self.observe)(access, result);
        result
    }
}

/// A typed endpoint and its placement, before mounting it on the PC.
/// The endpoint can be layered while its concrete type is still available.
pub struct PcAttachment<D> {
    page: BxPhyAddress,
    line: IsaIrq,
    device: D,
}

impl<D: PcMmioDevice> PcAttachment<D> {
    pub fn new(page: BxPhyAddress, line: IsaIrq, device: D) -> Self {
        Self { page, line, device }
    }

    pub fn observe_writes<F: FnMut(MmioWrite, Result<Reaction, SubscriberError>) + Send>(
        self,
        observe: F,
    ) -> PcAttachment<ObserveWrites<D, F>> {
        PcAttachment::new(
            self.page,
            self.line,
            ObserveWrites::new(self.device, observe),
        )
    }

    /// Separate placement from the typed endpoint, for example when placing
    /// the endpoint in caller-owned stable storage.
    pub fn into_parts(self) -> (BxPhyAddress, IsaIrq, D) {
        (self.page, self.line, self.device)
    }

    /// Mount with owned storage. Allocation-free boards can supply stable
    /// storage via [`PcDevice::Borrowed`] and `attach_composed_mmio`.
    #[cfg(feature = "alloc")]
    pub fn install<T: crate::cpu::Instrumentation>(
        self,
        machine: &mut crate::Emulator<T>,
    ) -> Result<(), PcAttachmentRefused<D>>
    where
        D: 'static,
    {
        let slot = match machine.composed_mmio_slot(self.page, self.line) {
            Ok(slot) => slot,
            Err(reason) => {
                return Err(PcAttachmentRefused {
                    reason,
                    attachment: self,
                });
            }
        };
        machine.insert_composed_mmio(slot, self.page, self.line, PcDevice::owned(self.device));
        Ok(())
    }
}

/// An owned mount was refused before allocation or type erasure.
#[derive(thiserror::Error)]
#[error("{reason}")]
pub struct PcAttachmentRefused<D> {
    pub reason: PcDeviceAttachRefusal,
    pub attachment: PcAttachment<D>,
}

impl<D> core::fmt::Debug for PcAttachmentRefused<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.reason.fmt(f)
    }
}

/// Storage supplied by the board for an MMIO endpoint. Detaching returns it.
pub enum PcDevice {
    Borrowed(&'static mut dyn PcMmioDevice),
    #[cfg(feature = "alloc")]
    Owned(alloc::boxed::Box<dyn PcMmioDevice>),
}

impl PcDevice {
    #[cfg(feature = "alloc")]
    pub fn owned(device: impl PcMmioDevice + 'static) -> Self {
        Self::Owned(alloc::boxed::Box::new(device))
    }

    pub fn endpoint(&mut self) -> &mut dyn PcMmioDevice {
        match self {
            Self::Borrowed(device) => *device,
            #[cfg(feature = "alloc")]
            Self::Owned(device) => device.as_mut(),
        }
    }
}

/// A board-selected ISA line. IRQ2 is the PIC cascade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IsaIrq(IrqLine);

impl IsaIrq {
    pub const fn new(line: u8) -> Option<Self> {
        if line < 16 && line != 2 {
            Some(Self(IrqLine(line)))
        } else {
            None
        }
    }

    fn mask(self) -> u16 {
        1 << self.0.0
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ComposedIrqs {
    Endpoint(IsaIrq),
    Bus(u16),
}

impl ComposedIrqs {
    pub(crate) fn mask(self) -> u16 {
        match self {
            Self::Endpoint(line) => line.mask(),
            Self::Bus(mask) => mask,
        }
    }

    pub(crate) fn lines(self) -> impl Iterator<Item = IrqLine> {
        let mask = self.mask();
        (0..16)
            .filter(move |line| mask & (1 << line) != 0)
            .map(IrqLine)
    }

    pub(crate) fn is_bus(self) -> bool {
        matches!(self, Self::Bus(_))
    }
}

impl From<IsaIrq> for IrqLine {
    fn from(line: IsaIrq) -> Self {
        line.0
    }
}

/// Rejected placement, with the endpoint storage returned to its owner.
#[derive(thiserror::Error)]
#[error("{reason}")]
pub struct PcDeviceAttachError {
    pub reason: PcDeviceAttachRefusal,
    pub device: PcDevice,
}

impl core::fmt::Debug for PcDeviceAttachError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.reason.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PcDeviceAttachRefusal {
    #[error(transparent)]
    Address(#[from] MmioAttachRefusal),
    #[error("the PC already uses this ISA line")]
    BoardIrqInUse,
    #[error("another composed endpoint is already driving this ISA line")]
    IrqInUse,
}

/// A short DMA loan limited to one ordinary RAM page per write.
pub struct PcDma<'a> {
    pub(crate) memory: &'a mut BxMemC,
    pub(crate) exclusions: DmaExclusions<'a>,
    pub(crate) lapic: &'a BxLocalApic,
    pub(crate) wrote: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PcDmaError {
    #[error("DMA write must fit in one unaliased PC RAM page below 3 GiB")]
    Range,
    #[error("DMA destination page belongs to an MMIO mapping")]
    Mapped,
    #[error("DMA destination RAM could not be made resident")]
    Backing,
}

impl DmaWrite for PcDma<'_> {
    type Error = PcDmaError;

    fn write(&mut self, address: BxPhyAddress, bytes: &[u8]) -> Result<(), Self::Error> {
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > 4096 - address.page_offset() as usize
            || (0xA0000..0x100000).contains(&address.get())
            || address.get() >= 0xC000_0000
            || self.memory.a20_addr(address) != address
        {
            return Err(PcDmaError::Range);
        }
        let page = address.page_base();
        if self.exclusions.overlaps(page)
            || self.memory.mmio.overlaps_page(page)
            || self.lapic.is_selected(page)
        {
            return Err(PcDmaError::Mapped);
        }
        self.memory
            .write_dma_page(address, bytes)
            .map_err(|err| match err {
                crate::Error::Memory(crate::memory::MemoryError::RamImageOutOfRange) => {
                    PcDmaError::Range
                }
                _ => PcDmaError::Backing,
            })?;
        self.wrote = true;
        Ok(())
    }
}

/// A short loan of a single IRQ output, wired by the board.
pub struct PcInterrupt<'a> {
    pub(crate) irq: &'a mut IrqFabric,
    pub(crate) routes: ComposedIrqs,
    pub(crate) driven: bool,
}

impl PcInterrupt<'_> {
    fn with_line(
        &mut self,
        line: IsaIrq,
        operation: impl FnOnce(&mut PcInterrupt<'_>) -> Result<Reaction, SubscriberError>,
    ) -> Result<Reaction, SubscriberError> {
        if self.routes.mask() & line.mask() == 0 {
            return Err(SubscriberError("MMIO window IRQ is outside this route"));
        }
        let mut output = PcInterrupt {
            irq: self.irq,
            routes: ComposedIrqs::Endpoint(line),
            driven: false,
        };
        let result = operation(&mut output);
        self.driven |= output.driven;
        result
    }
}

impl InterruptOutput for PcInterrupt<'_> {
    fn set_level(&mut self, asserted: bool) {
        let ComposedIrqs::Endpoint(line) = self.routes else {
            unreachable!("bus routing projects a single interrupt output to each leaf");
        };
        self.irq.set_composed_level(line.into(), asserted);
        self.driven = true;
    }
}
