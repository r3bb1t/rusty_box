//! virtio devices behind the board's virtio-mmio transports.
//!
//! Provenance (R7): the virtio 1.2 specification, sections 2 (device status,
//! feature bits, the configuration space and its generation count) and 4.2
//! (virtio over MMIO, the version 2 register layout), as QEMU implements
//! them:
//!
//! - `hw/virtio/virtio-mmio.c` with `force-legacy=false`: `virtio_mmio_read`
//!   and `virtio_mmio_write` (32-bit register accesses only; the queue
//!   registers latched per queue index and handed to the device when the
//!   driver sets `QueueReady`; `QueueReady` kept for every index below
//!   `VIRTIO_QUEUE_MAX`), `virtio_mmio_soft_reset` and `virtio_mmio_reset`;
//!   its accesses go through QEMU's memory core four bytes at most, so an
//!   eight-byte access is two, low half first;
//! - `hw/virtio/virtio.c`: `virtio_set_status`, `virtio_set_features_ex`
//!   (features the device does not offer are dropped, and `FEATURES_OK`
//!   stays set), `virtio_reset`, `virtio_error` (`DEVICE_NEEDS_RESET` and a
//!   device that ignores its queues until reset), `virtio_notify`,
//!   `virtio_notify_config` and `virtio_notify_vector` (a broken device
//!   leaves its interrupt line where it was), `virtio_queue_notify`, and the
//!   features every device offers (`DEFINE_VIRTIO_COMMON_FEATURES`:
//!   indirect descriptors, event index, notify-on-empty, any-layout and
//!   queue reset, the transport adding `VIRTIO_F_VERSION_1`, and the legacy
//!   ones hidden from `DeviceFeatures`);
//! - `hw/virtio/virtio-config-io.c`: `virtio_config_modern_read*` and
//!   `_write*` (the device's configuration refreshed into a buffer on every
//!   read, a write stored into that buffer and handed to the device; an
//!   access past the configuration reads all ones and writes nothing).
//!
//! A device is plugged into the transports from the last down, the order
//! QEMU's `virt` hands `-device` options their bus (`hw/arm/virt.c`
//! `create_virtio_devices`), and transport `n` raises SPI `16 + n`. A board
//! with devices is QEMU's `virt` given `-global virtio-mmio.force-legacy=false`,
//! which every transport then follows ([`super::virtio_mmio`]): the devices
//! here are virtio 1.0 devices, which Linux's drivers refuse on a legacy
//! transport (`virtinput_probe`, `virtio_gpu_init`).

pub(crate) mod input;
pub(crate) mod queue;

#[cfg(feature = "alloc")]
pub(crate) mod block;
#[cfg(feature = "alloc")]
pub(crate) mod edid;
#[cfg(feature = "alloc")]
pub(crate) mod gpu;

#[cfg(test)]
#[path = "../../../tests/unit/virtio_driver.rs"]
pub(crate) mod test_driver;

#[cfg(test)]
#[path = "../../../tests/unit/virtio_transport.rs"]
mod tests;

use rusty_box_core::snap::{SnapError, SnapRead, SnapResult, SnapWrite};

use crate::bus::{AccessWidth, RegOffset};
use input::Input;
use queue::{Chain, GuestRam, Queue, QueueFault, RingFeatures};

/// "virt", little-endian.
pub const MAGIC: u32 = 0x7472_6976;
/// `Version` of a non-legacy transport.
pub const MODERN_VERSION: u32 = 2;
/// QEMU's vendor ID, "QEMU" little-endian.
pub const VENDOR_QEMU: u32 = 0x554D_4551;
/// `VIRTIO_QUEUE_MAX`: the queue indices `QueueSel` and `QueueNotify`
/// accept.
const VIRTIO_QUEUE_MAX: u32 = 1024;
/// Where the device's configuration starts in a transport's registers.
const CONFIG_OFFSET: u64 = 0x100;
/// The largest configuration a device here has: `struct
/// virtio_input_config`.
const CONFIG_CAPACITY: usize = 136;

const VIRTIO_F_NOTIFY_ON_EMPTY: u64 = 1 << 24;
const VIRTIO_F_ANY_LAYOUT: u64 = 1 << 27;
const VIRTIO_RING_F_INDIRECT_DESC: u64 = 1 << 28;
const VIRTIO_RING_F_EVENT_IDX: u64 = 1 << 29;
const VIRTIO_F_BAD_FEATURE: u64 = 1 << 30;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_F_RING_RESET: u64 = 1 << 40;

/// The features QEMU gives every device on a non-legacy virtio-mmio
/// transport, before the device adds its own.
const COMMON_FEATURES: u64 = VIRTIO_RING_F_INDIRECT_DESC
    | VIRTIO_RING_F_EVENT_IDX
    | VIRTIO_F_NOTIFY_ON_EMPTY
    | VIRTIO_F_ANY_LAYOUT
    | VIRTIO_F_RING_RESET
    | VIRTIO_F_VERSION_1;
/// `VIRTIO_LEGACY_FEATURES`: offered but kept out of `DeviceFeatures`.
const LEGACY_FEATURES: u64 = VIRTIO_F_BAD_FEATURE | VIRTIO_F_NOTIFY_ON_EMPTY | VIRTIO_F_ANY_LAYOUT;

pub const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FEATURES_OK: u8 = 0x08;
const STATUS_NEEDS_RESET: u8 = 0x40;

/// `InterruptStatus` bits.
const ISR_VRING: u8 = 1;
const ISR_CONFIG: u8 = 2;

/// The most virtio devices a board carries.
pub const MAX_DEVICES: usize = 4;

/// The most queues a device here has. A device with fewer gives the rest a
/// size of 0, which is how QEMU leaves a queue its device never added.
pub const QUEUES: usize = 2;

/// QEMU's `VirtIODevice` core: the state the transport and the device
/// share.
pub(crate) struct Core {
    status: u8,
    isr: u8,
    queue_sel: u16,
    host_features: u64,
    guest_features: u64,
    generation: u32,
    /// The rule the driver broke, if it broke one: the device ignores its
    /// queues until reset.
    broken: Option<QueueFault>,
    /// The interrupt line as the transport last drove it
    /// (`virtio_mmio_update_irq`).
    irq_level: bool,
}

impl Core {
    fn new(device_features: u64) -> Self {
        Self {
            status: 0,
            isr: 0,
            queue_sel: 0,
            host_features: COMMON_FEATURES | device_features,
            guest_features: 0,
            generation: 0,
            broken: None,
            irq_level: false,
        }
    }

    /// QEMU `virtio_vdev_has_feature`: whether the driver accepted `bit`.
    fn has_feature(&self, bit: u64) -> bool {
        self.guest_features & bit != 0
    }

    fn ring_features(&self) -> RingFeatures {
        RingFeatures {
            event_idx: self.has_feature(VIRTIO_RING_F_EVENT_IDX),
            notify_on_empty: self.has_feature(VIRTIO_F_NOTIFY_ON_EMPTY),
        }
    }

    /// QEMU `virtio_device_disabled`.
    fn disabled(&self) -> bool {
        self.broken.is_some()
    }

    /// QEMU `virtio_notify_vector`: drive the line from `InterruptStatus`,
    /// unless the device is disabled.
    fn update_irq(&mut self) {
        if !self.disabled() {
            self.irq_level = self.isr != 0;
        }
    }

    /// QEMU `virtio_notify_config`: the configuration changed. Nothing
    /// happens before the driver is ready.
    pub(crate) fn notify_config(&mut self) {
        if self.status & STATUS_DRIVER_OK == 0 {
            return;
        }
        self.isr |= ISR_VRING | ISR_CONFIG;
        self.generation = self.generation.wrapping_add(1);
        self.update_irq();
    }

    /// QEMU `virtio_error`.
    fn error(&mut self, fault: QueueFault) {
        if self.has_feature(VIRTIO_F_VERSION_1) {
            self.status |= STATUS_NEEDS_RESET;
            self.notify_config();
        }
        self.broken = Some(fault);
    }
}

/// What a device works with while it runs: guest memory, its queues and
/// the core it shares with its transport (QEMU's `VirtIODevice`, as a
/// device's handlers see it). A queue's errors are QEMU's `virtio_error`:
/// the device is broken, and the call that met the error finds nothing.
pub(crate) struct Vdev<'a, 'r> {
    ram: &'a mut GuestRam<'r>,
    queues: &'a mut [Queue; QUEUES],
    core: &'a mut Core,
}

impl Vdev<'_, '_> {
    /// QEMU `virtio_queue_ready`.
    #[cfg(feature = "alloc")]
    pub(crate) fn queue_ready(&self, q: usize) -> bool {
        self.queues[q].is_ready()
    }

    /// How many chains queue `q` has ready for the device: how many pops
    /// would find one.
    pub(crate) fn available(&mut self, q: usize) -> u16 {
        if self.core.disabled() {
            return 0;
        }
        self.queues[q].available(self.ram)
    }

    /// QEMU `virtqueue_pop`.
    pub(crate) fn pop(&mut self, q: usize) -> Option<Chain> {
        if self.core.disabled() {
            return None;
        }
        let features = self.core.ring_features();
        match self.queues[q].pop(self.ram, features) {
            Ok(chain) => chain,
            Err(fault) => {
                self.core.error(fault);
                None
            }
        }
    }

    /// QEMU `virtqueue_unpop`.
    pub(crate) fn unpop(&mut self, q: usize) {
        self.queues[q].unpop();
    }

    /// QEMU `virtqueue_detach_element`.
    #[cfg(feature = "alloc")]
    pub(crate) fn detach(&mut self, q: usize) {
        self.queues[q].detach();
    }

    /// QEMU `virtio_queue_set_notification`.
    #[cfg(feature = "alloc")]
    pub(crate) fn set_notification(&mut self, q: usize, enable: bool) {
        let features = self.core.ring_features();
        self.queues[q].set_notification(self.ram, features, enable);
    }

    /// QEMU `virtio_error`: the driver broke a rule the device checks.
    #[cfg(feature = "alloc")]
    pub(crate) fn error(&mut self, fault: QueueFault) {
        self.core.error(fault);
    }

    /// QEMU `virtqueue_push`.
    pub(crate) fn push(&mut self, q: usize, chain: &Chain, len: u32) {
        let disabled = self.core.disabled();
        self.queues[q].push(self.ram, chain.head, len, disabled);
    }

    /// QEMU `virtio_notify`: interrupt the driver if it wants to hear about
    /// what queue `q` used.
    pub(crate) fn notify(&mut self, q: usize) {
        let features = self.core.ring_features();
        let disabled = self.core.disabled();
        if self.queues[q].should_notify(self.ram, features, disabled) {
            self.core.isr |= ISR_VRING;
            self.core.update_irq();
        }
    }

    /// The device-readable bytes of `chain`, from `offset`.
    pub(crate) fn read(&self, chain: &Chain, offset: usize, out: &mut [u8]) -> usize {
        chain.read(self.ram, offset, out)
    }

    /// Fill the device-writable bytes of `chain`, from `offset`.
    pub(crate) fn write(&mut self, chain: &Chain, offset: usize, data: &[u8]) -> usize {
        chain.write(self.ram, offset, data)
    }

    /// Guest memory, for buffers a device keeps beyond a chain (a GPU
    /// resource's backing).
    #[cfg(feature = "alloc")]
    pub(crate) fn ram(&self) -> &GuestRam<'_> {
        self.ram
    }
}

/// What is plugged into a transport.
///
/// Closed (R5): every device this board models.
pub(crate) enum Device {
    Input(Input),
    #[cfg(feature = "alloc")]
    Gpu(gpu::Gpu),
    #[cfg(feature = "alloc")]
    Block(block::Block),
}

impl Device {
    /// The virtio device ID (virtio 1.2 section 5).
    fn id(&self) -> u32 {
        match self {
            Self::Input(_) => 18,
            #[cfg(feature = "alloc")]
            Self::Gpu(_) => 16,
            #[cfg(feature = "alloc")]
            Self::Block(_) => 2,
        }
    }

    /// The size QEMU gives each queue when it adds it (`virtio_add_queue`).
    fn queue_sizes(&self) -> [u16; QUEUES] {
        match self {
            Self::Input(_) => [input::QUEUE_SIZE, input::QUEUE_SIZE],
            #[cfg(feature = "alloc")]
            Self::Gpu(_) => [gpu::CONTROL_QUEUE_SIZE, gpu::CURSOR_QUEUE_SIZE],
            #[cfg(feature = "alloc")]
            Self::Block(_) => [block::QUEUE_SIZE, 0],
        }
    }

    /// The device's own feature bits (its `get_features`).
    fn features(&self) -> u64 {
        match self {
            Self::Input(_) => 0,
            #[cfg(feature = "alloc")]
            Self::Gpu(_) => gpu::FEATURES,
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.features(),
        }
    }

    /// Bytes of configuration space (`virtio_init`'s `config_size`).
    fn config_len(&self) -> usize {
        match self {
            Self::Input(d) => d.config_len(),
            #[cfg(feature = "alloc")]
            Self::Gpu(_) => gpu::CONFIG_LEN,
            #[cfg(feature = "alloc")]
            Self::Block(_) => block::CONFIG_LEN,
        }
    }

    fn get_config(&self, config: &mut [u8]) {
        match self {
            Self::Input(d) => d.get_config(config),
            #[cfg(feature = "alloc")]
            Self::Gpu(d) => d.get_config(config),
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.get_config(config),
        }
    }

    fn set_config(&mut self, config: &[u8], core: &mut Core) {
        match self {
            Self::Input(d) => d.set_config(config, core),
            #[cfg(feature = "alloc")]
            Self::Gpu(d) => d.set_config(config),
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.set_config(config),
        }
    }

    /// The device's `set_status`, before the core takes `status`. Only the
    /// block device reads the features the driver accepted.
    #[cfg_attr(not(feature = "alloc"), allow(unused_variables))]
    fn set_status(&mut self, status: u8, core: &Core) {
        match self {
            Self::Input(d) => d.set_status(status),
            #[cfg(feature = "alloc")]
            Self::Gpu(_) => {}
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.set_status(status, core),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Input(d) => d.reset(),
            #[cfg(feature = "alloc")]
            Self::Gpu(d) => d.reset(),
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.reset(),
        }
    }

    /// The driver notified queue `q` (the queue's `handle_output`).
    fn handle_output(&mut self, vdev: &mut Vdev<'_, '_>, q: usize) {
        match self {
            Self::Input(d) => d.handle_output(vdev, q),
            #[cfg(feature = "alloc")]
            Self::Gpu(d) => d.handle_output(vdev, q),
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.handle_output(vdev, q),
        }
    }

    fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        match self {
            Self::Input(d) => d.save(w),
            #[cfg(feature = "alloc")]
            Self::Gpu(d) => d.save(w),
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.save(w),
        }
    }

    fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        match self {
            Self::Input(d) => d.restore(r),
            #[cfg(feature = "alloc")]
            Self::Gpu(d) => d.restore(r),
            #[cfg(feature = "alloc")]
            Self::Block(d) => d.restore(r),
        }
    }
}

/// The queue registers the driver writes, latched until it sets
/// `QueueReady` (QEMU `VirtIOMMIOQueue`).
#[derive(Debug, Clone, Copy, Default)]
struct ProxyQueue {
    num: u16,
    desc: [u32; 2],
    avail: [u32; 2],
    used: [u32; 2],
}

impl ProxyQueue {
    fn address(halves: [u32; 2]) -> u64 {
        (u64::from(halves[1]) << 32) | u64::from(halves[0])
    }
}

/// A non-legacy virtio-mmio transport with a device plugged in.
pub(crate) struct Transport {
    /// Which of the board's transports this is.
    slot: u8,
    pub(crate) device: Device,
    core: Core,
    queues: [Queue; QUEUES],
    proxy: [ProxyQueue; QUEUES],
    /// `QueueReady` for every queue index, a bit each.
    enabled: [u64; (VIRTIO_QUEUE_MAX / 64) as usize],
    host_features_sel: u32,
    guest_features_sel: u32,
    guest_features: [u32; 2],
    /// The configuration space as the device last filled it, with what the
    /// driver wrote since (QEMU `vdev->config`).
    config: [u8; CONFIG_CAPACITY],
}

impl Transport {
    pub(crate) fn new(slot: u8, device: Device) -> Self {
        let [a, b] = device.queue_sizes();
        Self {
            slot,
            core: Core::new(device.features()),
            device,
            queues: [Queue::new(a), Queue::new(b)],
            proxy: [ProxyQueue::default(); QUEUES],
            enabled: [0; (VIRTIO_QUEUE_MAX / 64) as usize],
            host_features_sel: 0,
            guest_features_sel: 0,
            guest_features: [0; 2],
            config: [0; CONFIG_CAPACITY],
        }
    }

    pub(crate) fn slot(&self) -> u8 {
        self.slot
    }

    /// Which input device this is, if it is one.
    pub(crate) fn input_kind(&self) -> Option<input::InputKind> {
        match &self.device {
            Device::Input(i) => Some(i.kind()),
            #[cfg(feature = "alloc")]
            Device::Gpu(_) | Device::Block(_) => None,
        }
    }

    /// The interrupt line.
    pub(crate) fn irq(&self) -> bool {
        self.core.irq_level
    }

    /// Run `f` on the device with what it works with: guest memory, its
    /// queues and the shared core.
    pub(crate) fn with_device<T, F: FnOnce(&mut Device, &mut Vdev<'_, '_>) -> T>(&mut self, ram: &mut GuestRam<'_>, f: F) -> T {
        let mut vdev = Vdev {
            ram,
            queues: &mut self.queues,
            core: &mut self.core,
        };
        f(&mut self.device, &mut vdev)
    }

    /// The host changed the device's configuration (QEMU
    /// `virtio_notify_config`).
    #[cfg(feature = "alloc")]
    pub(crate) fn notify_config(&mut self) {
        self.core.notify_config();
    }

    /// QEMU `virtio_reset`, as a driver's write of 0 to `Status` or a
    /// system reset runs it.
    fn reset_device(&mut self) {
        self.device.set_status(0, &self.core);
        self.device.reset();
        self.core.broken = None;
        self.core.guest_features = 0;
        self.core.queue_sel = 0;
        self.core.status = 0;
        self.core.isr = 0;
        self.core.update_irq();
        for q in &mut self.queues {
            q.reset();
        }
    }

    /// QEMU `virtio_mmio_soft_reset`.
    fn soft_reset(&mut self) {
        self.reset_device();
        self.enabled = [0; (VIRTIO_QUEUE_MAX / 64) as usize];
    }

    /// QEMU `virtio_mmio_reset`: the system reset.
    pub(crate) fn reset(&mut self) {
        self.soft_reset();
        self.host_features_sel = 0;
        self.guest_features_sel = 0;
        self.guest_features = [0; 2];
        self.proxy = [ProxyQueue::default(); QUEUES];
    }

    fn selected(&self) -> Option<usize> {
        let q = usize::from(self.core.queue_sel);
        (q < QUEUES).then_some(q)
    }

    fn queue_enabled(&self, index: u16) -> bool {
        self.enabled[usize::from(index / 64)] & (1 << (index % 64)) != 0
    }

    fn set_queue_enabled(&mut self, index: u16, on: bool) {
        let word = &mut self.enabled[usize::from(index / 64)];
        if on {
            *word |= 1 << (index % 64);
        } else {
            *word &= !(1 << (index % 64));
        }
    }

    pub(crate) fn read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        if width == AccessWidth::Dword {
            let low = self.read_word(offset.get(), AccessWidth::Word);
            let high = self.read_word(offset.get() + 4, AccessWidth::Word);
            return low | (high << 32);
        }
        self.read_word(offset.get(), width)
    }

    fn read_word(&mut self, off: u64, width: AccessWidth) -> u64 {
        if off >= CONFIG_OFFSET {
            return self.config_read((off - CONFIG_OFFSET) as usize, width);
        }
        if width != AccessWidth::Word {
            return 0;
        }
        u64::from(match off {
            0x000 => MAGIC,
            0x004 => MODERN_VERSION,
            0x008 => self.device.id(),
            0x00C => VENDOR_QEMU,
            0x010 => ((self.core.host_features & !LEGACY_FEATURES) >> (32 * self.host_features_sel)) as u32,
            0x034 => self.selected().map_or(0, |q| u32::from(self.queues[q].num_default())),
            0x044 => u32::from(self.queue_enabled(self.core.queue_sel)),
            0x060 => u32::from(self.core.isr),
            0x070 => u32::from(self.core.status),
            // No shared memory regions: their length reads as all ones.
            0x0B0 | 0x0B4 => u32::MAX,
            0x0FC => self.core.generation,
            _ => 0,
        })
    }

    /// QEMU `virtio_config_modern_read*`.
    fn config_read(&mut self, addr: usize, width: AccessWidth) -> u64 {
        let size = width.bytes() as usize;
        let len = self.device.config_len();
        if addr + size > len {
            return u64::from(u32::MAX) & width.value_mask();
        }
        self.device.get_config(&mut self.config[..len]);
        let mut value = 0u64;
        for (i, b) in self.config[addr..addr + size].iter().enumerate() {
            value |= u64::from(*b) << (8 * i);
        }
        value
    }

    /// QEMU `virtio_config_modern_write*`.
    fn config_write(&mut self, addr: usize, width: AccessWidth, value: u64) {
        let size = width.bytes() as usize;
        let len = self.device.config_len();
        if addr + size > len {
            return;
        }
        for (i, b) in self.config[addr..addr + size].iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        self.device.set_config(&self.config[..len], &mut self.core);
    }

    pub(crate) fn write(&mut self, ram: &mut GuestRam<'_>, offset: RegOffset, width: AccessWidth, value: u64) {
        if width == AccessWidth::Dword {
            self.write_word(ram, offset.get(), AccessWidth::Word, value & 0xFFFF_FFFF);
            self.write_word(ram, offset.get() + 4, AccessWidth::Word, value >> 32);
            return;
        }
        self.write_word(ram, offset.get(), width, value);
    }

    fn write_word(&mut self, ram: &mut GuestRam<'_>, off: u64, width: AccessWidth, value: u64) {
        if off >= CONFIG_OFFSET {
            self.config_write((off - CONFIG_OFFSET) as usize, width, value);
            return;
        }
        if width != AccessWidth::Word {
            return;
        }
        let v = value as u32;
        match off {
            0x014 => self.host_features_sel = u32::from(v != 0),
            0x020 => self.guest_features[self.guest_features_sel as usize] = v,
            0x024 => self.guest_features_sel = u32::from(v != 0),
            0x030 => {
                if v < VIRTIO_QUEUE_MAX {
                    self.core.queue_sel = v as u16;
                }
            }
            0x038 => {
                if let Some(q) = self.selected() {
                    if let Err(fault) = self.queues[q].set_num(v as i32) {
                        self.core.error(fault);
                    }
                    if let Err(fault) = self.queues[q].map_rings(ram, self.core.ring_features()) {
                        self.core.error(fault);
                    }
                    self.proxy[q].num = v as u16;
                }
            }
            0x044 => self.write_queue_ready(ram, v),
            0x050 => self.queue_notify(ram, v as u16),
            0x064 => {
                self.core.isr &= !(v as u8);
                self.core.update_irq();
            }
            0x070 => self.write_status(ram, v as u8),
            0x080 | 0x084 | 0x090 | 0x094 | 0x0A0 | 0x0A4 => {
                if let Some(q) = self.selected() {
                    let half = usize::from(off & 4 != 0);
                    let proxy = &mut self.proxy[q];
                    match off & !4 {
                        0x080 => proxy.desc[half] = v,
                        0x090 => proxy.avail[half] = v,
                        _ => proxy.used[half] = v,
                    }
                }
            }
            _ => {}
        }
    }

    /// `QueueReady`: setting it hands the latched size and rings to the
    /// queue.
    fn write_queue_ready(&mut self, ram: &GuestRam<'_>, v: u32) {
        let index = self.core.queue_sel;
        if v == 0 {
            self.set_queue_enabled(index, false);
            return;
        }
        if let Some(q) = self.selected() {
            let proxy = self.proxy[q];
            if let Err(fault) = self.queues[q].set_num(i32::from(proxy.num)) {
                self.core.error(fault);
            }
            let (desc, avail, used) = (
                ProxyQueue::address(proxy.desc),
                ProxyQueue::address(proxy.avail),
                ProxyQueue::address(proxy.used),
            );
            if let Err(fault) = self.queues[q].set_rings(ram, desc, avail, used, self.core.ring_features()) {
                self.core.error(fault);
            }
        }
        self.set_queue_enabled(index, true);
    }

    /// QEMU `virtio_queue_notify`: run the device for queue `index`.
    fn queue_notify(&mut self, ram: &mut GuestRam<'_>, index: u16) {
        let q = usize::from(index);
        if q >= QUEUES || !self.queues[q].has_descriptors() || self.core.disabled() {
            return;
        }
        self.with_device(ram, |device, vdev| device.handle_output(vdev, q));
    }

    /// `Status`: QEMU `virtio_set_features` when the driver sets
    /// `FEATURES_OK`, then `virtio_set_status`, and a reset when the status
    /// is 0.
    fn write_status(&mut self, ram: &GuestRam<'_>, status: u8) {
        if status & STATUS_FEATURES_OK != 0 {
            self.set_features(ram, (u64::from(self.guest_features[1]) << 32) | u64::from(self.guest_features[0]));
        }
        self.device.set_status(status, &self.core);
        self.core.status = status;
        if self.core.status == 0 {
            self.soft_reset();
        }
    }

    /// QEMU `virtio_set_features_ex`: keep what the device offers of what
    /// the driver accepted, once.
    fn set_features(&mut self, ram: &GuestRam<'_>, features: u64) {
        if self.core.status & STATUS_FEATURES_OK != 0 {
            return;
        }
        self.core.guest_features = features & self.core.host_features;
        if self.core.has_feature(VIRTIO_RING_F_EVENT_IDX) {
            let ring = self.core.ring_features();
            for i in 0..QUEUES {
                if let Err(fault) = self.queues[i].map_rings(ram, ring) {
                    self.core.error(fault);
                }
            }
        }
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for q in &self.queues {
            q.save(w)?;
        }
        for p in &self.proxy {
            w.write_u16(p.num)?;
            for v in p.desc.iter().chain(&p.avail).chain(&p.used) {
                w.write_u32(*v)?;
            }
        }
        for word in &self.enabled {
            w.write_u64(*word)?;
        }
        w.write_u8(self.core.status)?;
        w.write_u8(self.core.isr)?;
        w.write_u16(self.core.queue_sel)?;
        w.write_u64(self.core.guest_features)?;
        w.write_u32(self.core.generation)?;
        w.write_u8(self.core.broken.map_or(0, QueueFault::code))?;
        w.write_bool(self.core.irq_level)?;
        w.write_u32(self.host_features_sel)?;
        w.write_u32(self.guest_features_sel)?;
        w.write_u32(self.guest_features[0])?;
        w.write_u32(self.guest_features[1])?;
        w.write_bytes(&self.config)?;
        self.device.save(w)
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        for q in &mut self.queues {
            q.restore(r)?;
        }
        for p in &mut self.proxy {
            p.num = r.read_u16()?;
            for v in p.desc.iter_mut().chain(&mut p.avail).chain(&mut p.used) {
                *v = r.read_u32()?;
            }
        }
        for word in &mut self.enabled {
            *word = r.read_u64()?;
        }
        self.core.status = r.read_u8()?;
        self.core.isr = r.read_u8()?;
        self.core.queue_sel = r.read_u16()?;
        if u32::from(self.core.queue_sel) >= VIRTIO_QUEUE_MAX {
            return Err(SnapError::Invalid("virtio queue selector past the last queue index"));
        }
        let guest_features = r.read_u64()?;
        if guest_features & !self.core.host_features != 0 {
            return Err(SnapError::Invalid("virtio driver features the device does not offer"));
        }
        self.core.guest_features = guest_features;
        self.core.generation = r.read_u32()?;
        self.core.broken = match r.read_u8()? {
            0 => None,
            code => Some(QueueFault::from_code(code).ok_or(SnapError::Invalid("virtio device broken by an unknown fault"))?),
        };
        self.core.irq_level = r.read_bool()?;
        self.host_features_sel = r.read_u32()?;
        self.guest_features_sel = r.read_u32()?;
        if self.host_features_sel > 1 || self.guest_features_sel > 1 {
            return Err(SnapError::Invalid("virtio feature selector past the second word"));
        }
        self.guest_features[0] = r.read_u32()?;
        self.guest_features[1] = r.read_u32()?;
        r.read_bytes(&mut self.config)?;
        self.device.restore(r)
    }
}
