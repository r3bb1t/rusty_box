//! A virtio driver for the device models' unit tests: guest RAM, the
//! transport's registers, and split rings laid out the way Linux's
//! `virtio_mmio` and `virtio_ring` lay them out. It drives a [`Transport`]
//! the way a guest does, through its registers and its memory, so the tests
//! built on it check what a guest would see.

use std::vec;
use std::vec::Vec;

use rusty_box_core::snap::{SnapResult, SnapWrite};

use crate::board::RAM_BASE;
use crate::bus::{AccessWidth, RegOffset};

use super::queue::GuestRam;
use super::{Device, Transport, Vdev};

// Register offsets (virtio 1.2 4.2.2).
pub const MAGIC_VALUE: u64 = 0x000;
pub const VERSION: u64 = 0x004;
pub const DEVICE_ID: u64 = 0x008;
pub const VENDOR_ID: u64 = 0x00C;
pub const DEVICE_FEATURES: u64 = 0x010;
pub const DEVICE_FEATURES_SEL: u64 = 0x014;
pub const DRIVER_FEATURES: u64 = 0x020;
pub const DRIVER_FEATURES_SEL: u64 = 0x024;
pub const QUEUE_SEL: u64 = 0x030;
pub const QUEUE_NUM_MAX: u64 = 0x034;
pub const QUEUE_NUM: u64 = 0x038;
pub const QUEUE_READY: u64 = 0x044;
pub const QUEUE_NOTIFY: u64 = 0x050;
pub const INTERRUPT_STATUS: u64 = 0x060;
pub const INTERRUPT_ACK: u64 = 0x064;
pub const STATUS: u64 = 0x070;
pub const QUEUE_DESC_LOW: u64 = 0x080;
pub const QUEUE_DESC_HIGH: u64 = 0x084;
pub const QUEUE_DRIVER_LOW: u64 = 0x090;
pub const QUEUE_DRIVER_HIGH: u64 = 0x094;
pub const QUEUE_DEVICE_LOW: u64 = 0x0A0;
pub const QUEUE_DEVICE_HIGH: u64 = 0x0A4;
pub const SHM_LEN_LOW: u64 = 0x0B0;
pub const SHM_LEN_HIGH: u64 = 0x0B4;
pub const SHM_BASE_LOW: u64 = 0x0B8;
pub const CONFIG_GENERATION: u64 = 0x0FC;
pub const CONFIG: u64 = 0x100;

// Status bits.
pub const ACKNOWLEDGE: u32 = 1;
pub const DRIVER: u32 = 2;
pub const DRIVER_OK: u32 = 4;
pub const FEATURES_OK: u32 = 8;
pub const NEEDS_RESET: u32 = 0x40;

// Feature bits.
pub const F_INDIRECT_DESC: u64 = 1 << 28;
pub const F_EVENT_IDX: u64 = 1 << 29;
pub const F_VERSION_1: u64 = 1 << 32;

// Descriptor flags.
pub const DESC_NEXT: u16 = 1;
pub const DESC_WRITE: u16 = 2;
pub const DESC_INDIRECT: u16 = 4;

/// Guest RAM the tests give the device.
const RAM_SIZE: usize = 4 << 20;
/// Where the driver's buffers start, above the rings.
const BUFFERS: u64 = RAM_BASE + 0x10_0000;

/// One buffer of a chain the driver makes available.
#[derive(Debug, Clone, Copy)]
pub struct Buf {
    pub addr: u64,
    pub len: u32,
    pub device_writes: bool,
}

impl Buf {
    pub fn readable(addr: u64, len: u32) -> Self {
        Self {
            addr,
            len,
            device_writes: false,
        }
    }

    pub fn writable(addr: u64, len: u32) -> Self {
        Self {
            addr,
            len,
            device_writes: true,
        }
    }
}

/// The driver's side of one virtqueue.
#[derive(Debug, Clone, Copy, Default)]
struct DriverQueue {
    size: u16,
    desc: u64,
    avail: u64,
    used: u64,
    next_desc: u16,
    avail_idx: u16,
    last_used: u16,
}

/// A guest with RAM, a transport and the driver's view of its queues.
pub struct Driver {
    pub ram: Vec<u8>,
    pub transport: Transport,
    queues: [DriverQueue; 2],
    next_buffer: u64,
}

impl Driver {
    pub fn new(device: Device) -> Self {
        Self {
            ram: vec![0; RAM_SIZE],
            transport: Transport::new(31, device),
            queues: [DriverQueue::default(); 2],
            next_buffer: BUFFERS,
        }
    }

    pub fn read_width(&mut self, off: u64, width: AccessWidth) -> u64 {
        self.transport.read(RegOffset::new(off), width)
    }

    pub fn read(&mut self, off: u64) -> u32 {
        self.read_width(off, AccessWidth::Word) as u32
    }

    pub fn write_width(&mut self, off: u64, width: AccessWidth, value: u64) {
        let mut ram = GuestRam::new(&mut self.ram);
        self.transport.write(&mut ram, RegOffset::new(off), width, value);
    }

    pub fn write(&mut self, off: u64, value: u32) {
        self.write_width(off, AccessWidth::Word, u64::from(value));
    }

    /// Both words of `DeviceFeatures`.
    pub fn device_features(&mut self) -> u64 {
        self.write(DEVICE_FEATURES_SEL, 0);
        let low = self.read(DEVICE_FEATURES);
        self.write(DEVICE_FEATURES_SEL, 1);
        let high = self.read(DEVICE_FEATURES);
        (u64::from(high) << 32) | u64::from(low)
    }

    /// Offer `features` as the driver's, both words.
    pub fn accept(&mut self, features: u64) {
        self.write(DRIVER_FEATURES_SEL, 0);
        self.write(DRIVER_FEATURES, features as u32);
        self.write(DRIVER_FEATURES_SEL, 1);
        self.write(DRIVER_FEATURES, (features >> 32) as u32);
    }

    /// Linux's `virtio_dev_probe` up to `FEATURES_OK`: reset, acknowledge,
    /// accept `features`.
    pub fn negotiate(&mut self, features: u64) {
        self.write(STATUS, 0);
        self.write(STATUS, ACKNOWLEDGE);
        self.write(STATUS, ACKNOWLEDGE | DRIVER);
        self.accept(features);
        self.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK);
    }

    /// Linux's `vm_setup_vq` for queue `q` at its largest size, rings at a
    /// region of their own.
    pub fn setup_queue(&mut self, q: usize) {
        self.write(QUEUE_SEL, q as u32);
        let size = self.read(QUEUE_NUM_MAX) as u16;
        let area = RAM_BASE + 0x1_0000 * (q as u64 + 1);
        let (desc, avail, used) = (area, area + 0x4000, area + 0x5000);
        self.queues[q] = DriverQueue {
            size,
            desc,
            avail,
            used,
            ..DriverQueue::default()
        };
        self.write(QUEUE_NUM, u32::from(size));
        self.write(QUEUE_DESC_LOW, desc as u32);
        self.write(QUEUE_DESC_HIGH, (desc >> 32) as u32);
        self.write(QUEUE_DRIVER_LOW, avail as u32);
        self.write(QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
        self.write(QUEUE_DEVICE_LOW, used as u32);
        self.write(QUEUE_DEVICE_HIGH, (used >> 32) as u32);
        self.write(QUEUE_READY, 1);
    }

    /// The whole of Linux's setup: negotiate `features`, set up both
    /// queues, `DRIVER_OK`.
    pub fn start(&mut self, features: u64) {
        self.negotiate(features);
        self.setup_queue(0);
        self.setup_queue(1);
        self.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
    }

    /// `len` bytes of fresh guest memory.
    pub fn alloc(&mut self, len: u64) -> u64 {
        let addr = self.next_buffer;
        self.next_buffer += len.next_multiple_of(16);
        addr
    }

    /// A fresh buffer holding `data`.
    pub fn alloc_with(&mut self, data: &[u8]) -> u64 {
        let addr = self.alloc(data.len() as u64);
        self.put(addr, data);
        addr
    }

    pub fn put(&mut self, addr: u64, data: &[u8]) {
        let at = (addr - RAM_BASE) as usize;
        self.ram[at..at + data.len()].copy_from_slice(data);
    }

    pub fn mem(&self, addr: u64, len: usize) -> &[u8] {
        let at = (addr - RAM_BASE) as usize;
        &self.ram[at..at + len]
    }

    pub fn put_u16(&mut self, addr: u64, value: u16) {
        self.put(addr, &value.to_le_bytes());
    }

    pub fn get_u16(&self, addr: u64) -> u16 {
        let b = self.mem(addr, 2);
        u16::from_le_bytes([b[0], b[1]])
    }

    /// Write descriptor `index` of queue `q`'s table.
    pub fn put_desc(&mut self, q: usize, index: u16, addr: u64, len: u32, flags: u16, next: u16) {
        let at = self.queues[q].desc + 16 * u64::from(index);
        Self::put_desc_at(self, at, addr, len, flags, next);
    }

    fn put_desc_at(&mut self, at: u64, addr: u64, len: u32, flags: u16, next: u16) {
        let mut raw = [0u8; 16];
        raw[..8].copy_from_slice(&addr.to_le_bytes());
        raw[8..12].copy_from_slice(&len.to_le_bytes());
        raw[12..14].copy_from_slice(&flags.to_le_bytes());
        raw[14..].copy_from_slice(&next.to_le_bytes());
        self.put(at, &raw);
    }

    /// Put `head` in queue `q`'s available ring and publish it.
    pub fn publish(&mut self, q: usize, head: u16) {
        let dq = self.queues[q];
        self.put_u16(dq.avail + 4 + 2 * u64::from(dq.avail_idx % dq.size), head);
        self.queues[q].avail_idx = dq.avail_idx.wrapping_add(1);
        self.put_u16(dq.avail + 2, self.queues[q].avail_idx);
    }

    /// Make `buffers` available on queue `q` as one chain of direct
    /// descriptors; returns its head.
    pub fn offer(&mut self, q: usize, buffers: &[Buf]) -> u16 {
        let size = self.queues[q].size;
        let head = self.queues[q].next_desc;
        for (i, b) in buffers.iter().enumerate() {
            let index = (head + i as u16) % size;
            let last = i + 1 == buffers.len();
            let flags = if last { 0 } else { DESC_NEXT } | if b.device_writes { DESC_WRITE } else { 0 };
            self.put_desc(q, index, b.addr, b.len, flags, (index + 1) % size);
        }
        self.queues[q].next_desc = (head + buffers.len() as u16) % size;
        self.publish(q, head);
        head
    }

    /// Make `buffers` available on queue `q` through one indirect
    /// descriptor; returns its head.
    pub fn offer_indirect(&mut self, q: usize, buffers: &[Buf]) -> u16 {
        let table = self.alloc(16 * buffers.len() as u64);
        for (i, b) in buffers.iter().enumerate() {
            let last = i + 1 == buffers.len();
            let flags = if last { 0 } else { DESC_NEXT } | if b.device_writes { DESC_WRITE } else { 0 };
            self.put_desc_at(table + 16 * i as u64, b.addr, b.len, flags, i as u16 + 1);
        }
        let size = self.queues[q].size;
        let head = self.queues[q].next_desc;
        self.put_desc(q, head, table, 16 * buffers.len() as u32, DESC_INDIRECT, 0);
        self.queues[q].next_desc = (head + 1) % size;
        self.publish(q, head);
        head
    }

    pub fn notify(&mut self, q: usize) {
        self.write(QUEUE_NOTIFY, q as u32);
    }

    /// The used-ring entries (`id`, `len`) queue `q` added since the last
    /// call.
    pub fn used(&mut self, q: usize) -> Vec<(u32, u32)> {
        let dq = self.queues[q];
        let idx = self.get_u16(dq.used + 2);
        let mut out = Vec::new();
        let mut i = dq.last_used;
        while i != idx {
            let e = self.mem(dq.used + 4 + 8 * u64::from(i % dq.size), 8);
            out.push((
                u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
            ));
            i = i.wrapping_add(1);
        }
        self.queues[q].last_used = idx;
        out
    }

    /// The available ring's flags.
    pub fn set_avail_flags(&mut self, q: usize, flags: u16) {
        let avail = self.queues[q].avail;
        self.put_u16(avail, flags);
    }

    /// `used_event`: the used index the driver wants an interrupt at.
    pub fn set_used_event(&mut self, q: usize, idx: u16) {
        let dq = self.queues[q];
        self.put_u16(dq.avail + 4 + 2 * u64::from(dq.size), idx);
    }

    /// `avail_event`: where the device has got to in the available ring.
    pub fn avail_event(&self, q: usize) -> u16 {
        let dq = self.queues[q];
        self.get_u16(dq.used + 4 + 8 * u64::from(dq.size))
    }

    /// Queue `q`'s available-ring address, for a test that writes it.
    pub fn avail_ring(&self, q: usize) -> u64 {
        self.queues[q].avail
    }

    /// The transport's interrupt line.
    pub fn irq(&self) -> bool {
        self.transport.irq()
    }

    /// Read and acknowledge `InterruptStatus`, as Linux's `vm_interrupt`.
    pub fn take_interrupt(&mut self) -> u32 {
        let isr = self.read(INTERRUPT_STATUS);
        self.write(INTERRUPT_ACK, isr);
        isr
    }

    /// Run `f` on the device as the host does, between guest accesses.
    pub fn host<T, F: FnOnce(&mut Device, &mut Vdev<'_, '_>) -> T>(&mut self, f: F) -> T {
        let mut ram = GuestRam::new(&mut self.ram);
        self.transport.with_device(&mut ram, f)
    }

    /// The transport's state, as a snapshot writes it.
    pub fn save(&self) -> Vec<u8> {
        let mut out = Bytes(Vec::new());
        match self.transport.save(&mut out) {
            Ok(()) => out.0,
            Err(e) => panic!("saving a transport: {e:?}"),
        }
    }

    /// This guest again, its transport restored from `saved` onto `device`
    /// and its memory and driver copied.
    pub fn restored(&self, device: Device, saved: &[u8]) -> Driver {
        let mut transport = Transport::new(31, device);
        let mut r: &[u8] = saved;
        if let Err(e) = transport.restore(&mut r) {
            panic!("restoring a transport: {e:?}");
        }
        assert!(r.is_empty(), "the restore read the whole snapshot");
        Driver {
            ram: self.ram.clone(),
            transport,
            queues: self.queues,
            next_buffer: self.next_buffer,
        }
    }
}

/// A snapshot written to memory.
struct Bytes(Vec<u8>);

impl SnapWrite for Bytes {
    fn write_bytes(&mut self, bytes: &[u8]) -> SnapResult {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}
