//! Checked, static physical mappings for small AArch64 boards.

use core::convert::Infallible;
use core::fmt;
use core::marker::PhantomData;

use rusty_box_core::{VmClock, VmInstant};
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{
    Emit, ExecutionRequest, InterruptOutput, IrqMmioModel, MmioWrite, Subscriptions, publish_events,
};

use crate::bus::{
    AccessWidth, BusFault, CodeEpoch, FetchSource, PhysAddr, PhysBus, load_le, store_le,
};
use crate::machine::custom::{BoardInputs, MachineBoard, ProcessorOutputs};

/// A nonempty physical interval with a representable exclusive end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressRange {
    base: PhysAddr,
    end: u64,
}

impl AddressRange {
    pub fn new(base: PhysAddr, len: u64) -> Result<Self, MapError> {
        let end = base
            .get()
            .checked_add(len)
            .filter(|&end| end > base.get())
            .ok_or(MapError::InvalidRange)?;
        Ok(Self { base, end })
    }

    #[inline]
    fn len(self) -> u64 {
        self.end - self.base.get()
    }

    #[inline]
    fn overlaps(self, other: Self) -> bool {
        self.base.get() < other.end && other.base.get() < self.end
    }

    #[inline]
    fn offset(self, address: PhysAddr) -> Option<u64> {
        address
            .offset_in(self.base.get(), self.len())
            .map(|offset| offset.get())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// A range is empty, unrepresentable, or cannot hold its access width.
    InvalidRange,
    Overlap,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRange => "invalid physical range",
            Self::Overlap => "physical mappings overlap",
        })
    }
}

impl core::error::Error for MapError {}

/// Validation refused a mapping before taking any resources from the caller.
pub struct MappingRefused<R> {
    pub reason: MapError,
    pub resources: R,
}

impl<R> fmt::Debug for MappingRefused<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(f)
    }
}

impl<R> fmt::Display for MappingRefused<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(f)
    }
}

impl<R> core::error::Error for MappingRefused<R> {}

/// The inputs to a device mount. Recover these to retry with a new range.
pub struct MountParts<D> {
    pub range: AddressRange,
    pub width: AccessWidth,
    pub device: D,
}

/// Borrowed RAM or ROM, with its length checked against the physical map.
pub struct MemoryRegion<B> {
    range: AddressRange,
    bytes: B,
}

pub type Ram<'a> = MemoryRegion<&'a mut [u8]>;
pub type Rom<'a> = MemoryRegion<&'a [u8]>;

impl<B: AsRef<[u8]>> MemoryRegion<B> {
    pub fn new(base: PhysAddr, bytes: B) -> Result<Self, MapError> {
        let range = AddressRange::new(base, bytes.as_ref().len() as u64)?;
        Ok(Self { range, bytes })
    }

    #[inline]
    fn read(&self, address: PhysAddr, width: AccessWidth) -> Option<u64> {
        load_le(
            self.bytes.as_ref(),
            self.range.offset(address)? as usize,
            width,
        )
    }
}

/// A scalar interrupt-capable model with clock and reset behavior. It has no
/// DMA or machine access. A timed model consumes due work in `advance` and
/// reports its next strictly future deadline.
///
/// ```compile_fail,E0599
/// use rusty_box_devices::composition::InterruptOutput;
/// fn attempt_dma<I: InterruptOutput>(irq: &mut I) {
///     irq.write(0x1000, &[42]);
/// }
/// ```
pub trait ClockedDevice: IrqMmioModel {
    fn advance<I: InterruptOutput>(&mut self, _clock: VmClock, _irq: &mut I) -> Option<VmInstant> {
        None
    }

    fn reset(&mut self, clock: VmClock);
}

type Event<D> = <<D as IrqMmioModel>::Events as IntoIterator>::Item;

#[derive(Default)]
struct Line(bool);

impl InterruptOutput for Line {
    fn set_level(&mut self, asserted: bool) {
        self.0 = asserted;
    }
}

/// A model and one naturally aligned scalar access width in a physical range.
pub struct MappedDevice<D: ClockedDevice, S = Subscriptions<Event<D>>> {
    range: AddressRange,
    width: AccessWidth,
    device: D,
    irq: Line,
    sink: S,
    stop: ExecutionRequest,
}

impl<D: ClockedDevice> MappedDevice<D> {
    pub fn new(
        range: AddressRange,
        width: AccessWidth,
        device: D,
    ) -> Result<Self, MappingRefused<MountParts<D>>> {
        if !range.base.get().is_multiple_of(width.bytes())
            || !range.len().is_multiple_of(width.bytes())
        {
            return Err(MappingRefused {
                reason: MapError::InvalidRange,
                resources: MountParts {
                    range,
                    width,
                    device,
                },
            });
        }
        Ok(Self {
            range,
            width,
            device,
            irq: Line::default(),
            sink: Subscriptions::new(),
            stop: ExecutionRequest::None,
        })
    }
}

impl<D: ClockedDevice, S> MappedDevice<D, S> {
    /// Replace the sink before constructing the board. Delivery lends only
    /// the event and can request a stop after the register write.
    pub fn with_events<T>(self, sink: T) -> MappedDevice<D, T>
    where
        T: Emit<Event<D>, Error = Infallible>,
    {
        MappedDevice {
            range: self.range,
            width: self.width,
            device: self.device,
            irq: self.irq,
            sink,
            stop: self.stop,
        }
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    /// Exclusive access to this particular mounted model.
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    #[inline]
    fn offset(
        &self,
        address: PhysAddr,
        width: AccessWidth,
    ) -> Option<Result<WindowOffset, BusFault>> {
        let offset = self.range.offset(address)?;
        Some(
            if self.width == width && offset.is_multiple_of(width.bytes()) {
                Ok(WindowOffset(offset))
            } else {
                Err(BusFault::Unmapped)
            },
        )
    }
}

/// The statically typed pair of two nonoverlapping device subtrees.
pub struct DevicePair<A, B> {
    first: A,
    second: B,
}

pub struct DevicePairParts<A, B> {
    pub first: A,
    pub second: B,
}

impl<A: DeviceMap, B: DeviceMap> DevicePair<A, B> {
    pub fn new(first: A, second: B) -> Result<Self, MappingRefused<DevicePairParts<A, B>>> {
        let mut overlaps = false;
        first.ranges(&mut |range| {
            overlaps |= second.overlaps(range);
        });
        if overlaps {
            return Err(MappingRefused {
                reason: MapError::Overlap,
                resources: DevicePairParts { first, second },
            });
        }
        Ok(Self { first, second })
    }

    pub fn first(&self) -> &A {
        &self.first
    }

    pub fn second(&self) -> &B {
        &self.second
    }
}

/// A short host loan of the devices on one board. Only model state is
/// mutable through it. The checked intervals cannot be replaced.
///
/// ```compile_fail
/// use rusty_box_aarch64::{Aarch64Machine, NoEl2};
/// use rusty_box_aarch64::board::address_space::{AddressSpace, ClockedDevice, MappedDevice, Mounted};
/// fn retain<'a, 'mem, D: ClockedDevice>(
///     machine: &'a mut Aarch64Machine<AddressSpace<'mem, MappedDevice<D>>, NoEl2>,
/// ) -> &'a mut D {
///     machine.with_devices_mut(|devices| devices.select::<Mounted>())
/// }
/// ```
pub struct DevicesMut<'a, D>(&'a mut D);

impl<'a, D> DevicesMut<'a, D> {
    /// Borrow the concrete model at a named slot in this tree.
    pub fn get<K: DeviceSlot<D>>(&mut self) -> &mut K::Model {
        K::project(DevicesMut(self.0))
    }

    /// Consume the view and retain its lifetime while selecting a model.
    pub fn select<K: DeviceSlot<D>>(self) -> &'a mut K::Model {
        K::project(self)
    }
}

/// A typed position in a static device tree. Name a path with a type alias
/// such as `type SensorSlot = First<Mounted>`.
///
/// ```compile_fail,E0277
/// use rusty_box_aarch64::board::address_space::{DevicesMut, First, Mounted};
/// fn missing_slot(mut devices: DevicesMut<'_, ()>) {
///     devices.get::<First<Mounted>>();
/// }
/// ```
pub trait DeviceSlot<D> {
    type Model: ClockedDevice;

    fn project(devices: DevicesMut<'_, D>) -> &mut Self::Model;
}

/// Select a mounted model at the current position.
pub struct Mounted;
/// Select a slot within the first child of a pair.
pub struct First<K>(PhantomData<fn() -> K>);
/// Select a slot within the second child of a pair.
pub struct Second<K>(PhantomData<fn() -> K>);

impl<D: ClockedDevice, S> DeviceSlot<MappedDevice<D, S>> for Mounted {
    type Model = D;

    fn project(devices: DevicesMut<'_, MappedDevice<D, S>>) -> &mut D {
        devices.device()
    }
}

impl<A: DeviceMap, B: DeviceMap, K: DeviceSlot<A>> DeviceSlot<DevicePair<A, B>> for First<K> {
    type Model = K::Model;

    fn project(devices: DevicesMut<'_, DevicePair<A, B>>) -> &mut Self::Model {
        devices.first().select::<K>()
    }
}

impl<A: DeviceMap, B: DeviceMap, K: DeviceSlot<B>> DeviceSlot<DevicePair<A, B>> for Second<K> {
    type Model = K::Model;

    fn project(devices: DevicesMut<'_, DevicePair<A, B>>) -> &mut Self::Model {
        devices.second().select::<K>()
    }
}

impl<'a, A: DeviceMap, B: DeviceMap> DevicesMut<'a, DevicePair<A, B>> {
    pub fn first(self) -> DevicesMut<'a, A> {
        DevicesMut(&mut self.0.first)
    }

    pub fn second(self) -> DevicesMut<'a, B> {
        DevicesMut(&mut self.0.second)
    }
}

impl<'a, D: ClockedDevice, S> DevicesMut<'a, MappedDevice<D, S>> {
    pub fn device(self) -> &'a mut D {
        self.0.device_mut()
    }
}

/// A sealed tree of checked MMIO mappings. Use `()`, `MappedDevice` or
/// nested `DevicePair`s. Each address decodes at most one member.
pub trait DeviceMap: sealed::Regions {}

impl<T: sealed::Regions> DeviceMap for T {}

use sealed::{Regions, Signals};

mod sealed {
    use super::*;

    #[derive(Default)]
    pub struct Signals {
        pub(super) irq: bool,
        pub(super) deadline: Option<VmInstant>,
        pub(super) stop: bool,
    }

    pub trait Regions {
        fn ranges<F: FnMut(AddressRange)>(&self, visit: &mut F);
        fn read(&mut self, address: PhysAddr, width: AccessWidth) -> Option<Result<u64, BusFault>>;
        fn write(
            &mut self,
            address: PhysAddr,
            width: AccessWidth,
            value: u64,
        ) -> Option<Result<(), BusFault>>;
        fn advance(&mut self, clock: VmClock, signals: &mut Signals);
        fn reset(&mut self, clock: VmClock);

        fn overlaps(&self, target: AddressRange) -> bool {
            let mut found = false;
            self.ranges(&mut |range| found |= range.overlaps(target));
            found
        }
    }
}

impl Regions for () {
    fn ranges<F: FnMut(AddressRange)>(&self, _: &mut F) {}

    fn read(&mut self, _: PhysAddr, _: AccessWidth) -> Option<Result<u64, BusFault>> {
        None
    }

    fn write(&mut self, _: PhysAddr, _: AccessWidth, _: u64) -> Option<Result<(), BusFault>> {
        None
    }

    fn advance(&mut self, _: VmClock, _: &mut Signals) {}

    fn reset(&mut self, _: VmClock) {}
}

impl<D, S> Regions for MappedDevice<D, S>
where
    D: ClockedDevice,
    S: Emit<Event<D>, Error = Infallible>,
{
    fn ranges<F: FnMut(AddressRange)>(&self, visit: &mut F) {
        visit(self.range);
    }

    #[inline]
    fn read(&mut self, address: PhysAddr, width: AccessWidth) -> Option<Result<u64, BusFault>> {
        Some(
            self.offset(address, width)?.map(|offset| {
                self.device.read(offset, width.bytes() as usize) & width.value_mask()
            }),
        )
    }

    #[inline]
    fn write(
        &mut self,
        address: PhysAddr,
        width: AccessWidth,
        value: u64,
    ) -> Option<Result<(), BusFault>> {
        Some(self.offset(address, width)?.map(|offset| {
            let write = MmioWrite {
                offset,
                len: width.bytes() as usize,
                value: value & width.value_mask(),
            };
            let Ok(request) = publish_events(
                self.device.write(write, &mut self.irq),
                &mut self.sink,
                &mut (),
            );
            if request == ExecutionRequest::Stop {
                self.stop = request;
            }
        }))
    }

    fn advance(&mut self, clock: VmClock, signals: &mut Signals) {
        if let Some(at) = self.device.advance(clock, &mut self.irq) {
            signals.deadline = Some(signals.deadline.map_or(at, |earlier| earlier.min(at)));
        }
        signals.irq |= self.irq.0;
        signals.stop |= core::mem::take(&mut self.stop) == ExecutionRequest::Stop;
    }

    fn reset(&mut self, clock: VmClock) {
        self.device.reset(clock);
        self.irq = Line::default();
        self.stop = ExecutionRequest::None;
    }
}

impl<A: DeviceMap, B: DeviceMap> Regions for DevicePair<A, B> {
    fn ranges<F: FnMut(AddressRange)>(&self, visit: &mut F) {
        self.first.ranges(visit);
        self.second.ranges(visit);
    }

    #[inline]
    fn read(&mut self, address: PhysAddr, width: AccessWidth) -> Option<Result<u64, BusFault>> {
        self.first
            .read(address, width)
            .or_else(|| self.second.read(address, width))
    }

    #[inline]
    fn write(
        &mut self,
        address: PhysAddr,
        width: AccessWidth,
        value: u64,
    ) -> Option<Result<(), BusFault>> {
        self.first
            .write(address, width, value)
            .or_else(|| self.second.write(address, width, value))
    }

    fn advance(&mut self, clock: VmClock, signals: &mut Signals) {
        self.first.advance(clock, signals);
        self.second.advance(clock, signals);
    }

    fn reset(&mut self, clock: VmClock) {
        self.first.reset(clock);
        self.second.reset(clock);
    }
}

/// One borrowed RAM bank, optional immutable ROM, and a statically composed
/// device tree. Device IRQs are level-ORed onto the processor's physical IRQ.
/// Other interrupt controllers can implement `MachineBoard` separately.
pub struct AddressSpace<'mem, D> {
    ram: Ram<'mem>,
    rom: Option<Rom<'mem>>,
    devices: D,
    touched: bool,
}

pub struct AddressSpaceParts<'mem, D> {
    pub ram: Ram<'mem>,
    pub rom: Option<Rom<'mem>>,
    pub devices: D,
}

impl<'mem, D: DeviceMap> AddressSpace<'mem, D> {
    pub fn new(
        ram: Ram<'mem>,
        rom: Option<Rom<'mem>>,
        devices: D,
    ) -> Result<Self, MappingRefused<AddressSpaceParts<'mem, D>>> {
        if devices.overlaps(ram.range)
            || rom
                .as_ref()
                .is_some_and(|rom| ram.range.overlaps(rom.range) || devices.overlaps(rom.range))
        {
            return Err(MappingRefused {
                reason: MapError::Overlap,
                resources: AddressSpaceParts { ram, rom, devices },
            });
        }
        Ok(Self {
            ram,
            rom,
            devices,
            touched: false,
        })
    }

    pub fn devices(&self) -> &D {
        &self.devices
    }

    /// The static tree stays in its validated placement. Mounts expose their
    /// original concrete model through `device_mut`.
    pub fn devices_mut(&mut self) -> DevicesMut<'_, D> {
        DevicesMut(&mut self.devices)
    }

    pub fn ram(&self) -> &[u8] {
        self.ram.bytes
    }

    /// Write bytes to RAM between runs, borrowing the board through its
    /// machine so the processor invalidates old code and translations.
    pub fn write_ram(&mut self, address: PhysAddr, data: &[u8]) -> Result<(), BusFault> {
        let start = self.ram.range.offset(address).ok_or(BusFault::Unmapped)? as usize;
        let end = start.checked_add(data.len()).ok_or(BusFault::Unmapped)?;
        self.ram
            .bytes
            .get_mut(start..end)
            .ok_or(BusFault::Unmapped)?
            .copy_from_slice(data);
        Ok(())
    }

    #[inline]
    fn memory_read(&self, address: PhysAddr, width: AccessWidth) -> Option<u64> {
        self.ram
            .read(address, width)
            .or_else(|| self.rom.as_ref()?.read(address, width))
    }
}

impl<D: DeviceMap> PhysBus for AddressSpace<'_, D> {
    #[inline]
    fn read(&mut self, addr: PhysAddr, width: AccessWidth, _: VmClock) -> Result<u64, BusFault> {
        if let Some(value) = self.memory_read(addr, width) {
            return Ok(value);
        }
        self.touched = true;
        self.devices
            .read(addr, width)
            .unwrap_or(Err(BusFault::Unmapped))
    }

    #[inline]
    fn write(
        &mut self,
        addr: PhysAddr,
        width: AccessWidth,
        value: u64,
        _: VmClock,
    ) -> Result<(), BusFault> {
        if let Some(offset) = self.ram.range.offset(addr) {
            return store_le(self.ram.bytes, offset as usize, width, value)
                .ok_or(BusFault::Unmapped);
        }
        self.touched = true;
        self.devices
            .write(addr, width, value)
            .unwrap_or(Err(BusFault::Unmapped))
    }

    #[inline]
    fn fetch(&mut self, addr: PhysAddr, _: VmClock) -> Result<u32, BusFault> {
        self.memory_read(addr, AccessWidth::Word)
            .map(|word| word as u32)
            .ok_or(BusFault::Unmapped)
    }

    #[inline]
    fn read_descriptor(&mut self, addr: PhysAddr, _: VmClock) -> Result<u64, BusFault> {
        self.memory_read(addr, AccessWidth::Dword)
            .ok_or(BusFault::Unmapped)
    }

    #[inline]
    fn fetch_source(&self, addr: PhysAddr) -> FetchSource {
        if self.ram.range.offset(addr).is_some()
            || self
                .rom
                .as_ref()
                .is_some_and(|rom| rom.range.offset(addr).is_some())
        {
            FetchSource::Stable(CodeEpoch::new(0))
        } else {
            FetchSource::Volatile
        }
    }

    #[inline]
    fn devices_touched(&self) -> bool {
        self.touched
    }
}

impl<D: DeviceMap> MachineBoard for AddressSpace<'_, D> {
    fn synchronize(&mut self, clock: VmClock, _: ProcessorOutputs) -> BoardInputs {
        self.touched = false;
        let mut signals = Signals::default();
        self.devices.advance(clock, &mut signals);
        BoardInputs {
            irq: signals.irq,
            deadline: signals.deadline,
            stop: signals.stop,
            ..BoardInputs::IDLE
        }
    }

    fn reset(&mut self, clock: VmClock) {
        self.devices.reset(clock);
        self.touched = false;
    }
}
