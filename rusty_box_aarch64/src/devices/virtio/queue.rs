//! Split virtqueues, from the device's side, over guest RAM.
//!
//! Provenance (R7): the virtio 1.2 specification, section 2.7 ("Split
//! Virtqueues"), as QEMU `hw/virtio/virtio.c` implements it for a device
//! that offers `VIRTIO_RING_F_INDIRECT_DESC` and `VIRTIO_RING_F_EVENT_IDX`:
//! `virtio_queue_set_num`, `virtio_queue_set_rings` and
//! `virtio_init_region_cache` (the rings must lie in guest memory when the
//! driver enables the queue), `virtqueue_split_pop` with
//! `virtqueue_num_heads`, `virtqueue_get_head` and `virtqueue_map_desc` (the
//! checks a chain must pass, each a `virtio_error`), `virtqueue_unpop`,
//! `virtqueue_split_fill` and `virtqueue_split_flush`, and
//! `virtio_split_should_notify` with `vring_need_event`,
//! `virtio_queue_set_notification` (which the block device turns off while
//! it works through its queue, so it publishes its `avail_event` only once
//! it is done) and `virtqueue_detach_element`.
//!
//! A chain is walked again from guest memory each time the device reads or
//! writes it, where QEMU maps its buffers once when it takes it. The two
//! agree for every driver that keeps the specification's rule of not
//! changing descriptors it has made available until the device has used
//! them; they differ only when a driver has the device write over such
//! descriptors itself.

use rusty_box_core::snap::{SnapError, SnapRead, SnapResult, SnapWrite};

use crate::board::RAM_BASE;
use crate::bus::{PhysAddr, Writes};

/// A rule the driver broke, each one of QEMU's `virtio_error` reports. The
/// device stops using its queues until it is reset (virtio 1.2 2.1.2,
/// `DEVICE_NEEDS_RESET`).
///
/// Closed (R5): the errors QEMU's split ring and transport raise for the
/// features these devices offer, and the block device's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueFault {
    /// `QueueNum` larger than the device's queue ("queue size exceeds max
    /// size").
    QueueTooLarge,
    /// A ring the driver enabled lies outside guest memory ("Failed to map
    /// descriptor ring" and its used and available ring siblings).
    UnmappableRing,
    /// More chains taken than the queue holds ("Virtqueue size exceeded").
    SizeExceeded,
    /// An available-ring entry past the descriptor table ("Guest says index
    /// is available").
    BadHead,
    /// A chain taken from a queue whose rings were never mapped ("Region
    /// caches not initialized").
    NoRings,
    /// An indirect table of no entries or of a partial one ("Invalid size
    /// for indirect buffer table").
    BadIndirectSize,
    /// An indirect table outside guest memory ("Cannot map indirect
    /// buffer").
    UnmappableIndirect,
    /// A buffer of no bytes ("zero sized buffers are not allowed").
    ZeroSized,
    /// More than `VIRTQUEUE_MAX_SIZE` buffers in one chain ("too many write
    /// descriptors in indirect table").
    TooManyBuffers,
    /// A buffer outside guest memory ("bogus descriptor or out of
    /// resources").
    UnmappableBuffer,
    /// A device-readable buffer after a device-writable one ("Incorrect
    /// order for descriptors").
    OutOfOrder,
    /// A chain longer than its table ("Looped descriptor").
    Looped,
    /// A `next` past the end of the table ("Desc next is").
    BadNext,
    /// A block request with no device-readable or no device-writable buffer
    /// ("virtio-blk missing headers").
    BlockMissingHeaders,
    /// A block request whose readable buffers are shorter than its header
    /// ("virtio-blk request outhdr too short").
    BlockHeaderShort,
    /// A discard or write-zeroes request without a whole range
    /// ("virtio-blk discard/write_zeroes header too short").
    BlockRangeShort,
    /// A zone report without room for one zone ("in buffer too small for
    /// zone report").
    BlockZoneReportShort,
}

impl QueueFault {
    /// A snapshot's code for the fault, never 0.
    pub fn code(self) -> u8 {
        match self {
            Self::QueueTooLarge => 1,
            Self::UnmappableRing => 2,
            Self::SizeExceeded => 3,
            Self::BadHead => 4,
            Self::NoRings => 5,
            Self::BadIndirectSize => 6,
            Self::UnmappableIndirect => 7,
            Self::ZeroSized => 8,
            Self::TooManyBuffers => 9,
            Self::UnmappableBuffer => 10,
            Self::OutOfOrder => 11,
            Self::Looped => 12,
            Self::BadNext => 13,
            Self::BlockMissingHeaders => 14,
            Self::BlockHeaderShort => 15,
            Self::BlockRangeShort => 16,
            Self::BlockZoneReportShort => 17,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::QueueTooLarge,
            2 => Self::UnmappableRing,
            3 => Self::SizeExceeded,
            4 => Self::BadHead,
            5 => Self::NoRings,
            6 => Self::BadIndirectSize,
            7 => Self::UnmappableIndirect,
            8 => Self::ZeroSized,
            9 => Self::TooManyBuffers,
            10 => Self::UnmappableBuffer,
            11 => Self::OutOfOrder,
            12 => Self::Looped,
            13 => Self::BadNext,
            14 => Self::BlockMissingHeaders,
            15 => Self::BlockHeaderShort,
            16 => Self::BlockRangeShort,
            17 => Self::BlockZoneReportShort,
            _ => return None,
        })
    }
}

/// Guest RAM, addressed by guest-physical address, as a device reaches it:
/// what the device writes is noted, for the processor to forget any code it
/// decoded there.
pub struct GuestRam<'a> {
    ram: &'a mut [u8],
    written: Writes,
}

impl<'a> GuestRam<'a> {
    pub fn new(ram: &'a mut [u8]) -> Self {
        Self {
            ram,
            written: Writes::NONE,
        }
    }

    /// The memory the device wrote, or took to write in place.
    pub(crate) fn written(&self) -> Writes {
        self.written
    }

    fn range(&self, addr: u64, len: u64) -> Option<core::ops::Range<usize>> {
        let start = usize::try_from(addr.checked_sub(RAM_BASE)?).ok()?;
        let end = start.checked_add(usize::try_from(len).ok()?)?;
        (end <= self.ram.len()).then_some(start..end)
    }

    /// Whether `len` bytes at `addr` are all guest RAM, the only memory the
    /// board's devices reach (QEMU's `dma_memory_map` also maps the flash
    /// banks and, through its bounce buffer, MMIO; see the README's "Where
    /// the references differ").
    pub fn contains(&self, addr: u64, len: u64) -> bool {
        self.range(addr, len).is_some()
    }

    /// The `len` bytes at `addr`, if they are all RAM.
    #[cfg(feature = "alloc")]
    pub fn bytes(&self, addr: u64, len: u64) -> Option<&[u8]> {
        let range = self.range(addr, len)?;
        Some(&self.ram[range])
    }

    /// The `len` bytes at `addr` to store into, if they are all RAM: a
    /// buffer a device fills in place, as QEMU's block layer fills a request's
    /// mapped buffers.
    #[cfg(feature = "alloc")]
    pub fn bytes_mut(&mut self, addr: u64, len: u64) -> Option<&mut [u8]> {
        let range = self.range(addr, len)?;
        self.written.add(PhysAddr::new(addr), len);
        Some(&mut self.ram[range])
    }

    /// `out.len()` bytes at `addr`, or `None` if they are not all RAM.
    pub fn read(&self, addr: u64, out: &mut [u8]) -> Option<()> {
        let range = self.range(addr, out.len() as u64)?;
        out.copy_from_slice(&self.ram[range]);
        Some(())
    }

    /// Store `data` at `addr`. Every store a device makes is into a range it
    /// checked lies in RAM when it mapped it (a ring when the queue was
    /// enabled, a buffer when its chain was taken), and guest RAM does not
    /// move, so a store always lands; a range that were not all RAM would
    /// not be written at all.
    pub fn store(&mut self, addr: u64, data: &[u8]) {
        if let Some(range) = self.range(addr, data.len() as u64) {
            self.ram[range].copy_from_slice(data);
            self.written.add(PhysAddr::new(addr), data.len() as u64);
        }
    }

    /// Store `data` at `addr`, or `None` if the bytes are not all RAM, in
    /// which case nothing is written: QEMU's `dma_memory_write` failing.
    pub fn try_store(&mut self, addr: u64, data: &[u8]) -> Option<()> {
        let range = self.range(addr, data.len() as u64)?;
        self.ram[range].copy_from_slice(data);
        self.written.add(PhysAddr::new(addr), data.len() as u64);
        Some(())
    }

    /// Set `len` bytes at `addr` to `byte`, or `None` if they are not all
    /// RAM, in which case nothing is written: QEMU's `dma_memory_set`.
    pub fn fill(&mut self, addr: u64, len: u64, byte: u8) -> Option<()> {
        let range = self.range(addr, len)?;
        self.ram[range].fill(byte);
        self.written.add(PhysAddr::new(addr), len);
        Some(())
    }

    fn read_u16(&self, addr: u64) -> Option<u16> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b)?;
        Some(u16::from_le_bytes(b))
    }
}

/// `VIRTQUEUE_MAX_SIZE`: the largest queue a driver may ask for, and the
/// most buffers one chain may map to.
pub const VIRTQUEUE_MAX_SIZE: u16 = 1024;

const DESC_SIZE: u64 = 16;
const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;
const VRING_DESC_F_INDIRECT: u16 = 4;
const VRING_AVAIL_F_NO_INTERRUPT: u16 = 1;
#[cfg(feature = "alloc")]
const VRING_USED_F_NO_NOTIFY: u16 = 1;

/// One descriptor as the driver wrote it.
#[derive(Debug, Clone, Copy)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

impl Descriptor {
    fn read(ram: &GuestRam<'_>, table: u64, index: u32) -> Option<Self> {
        let mut raw = [0u8; DESC_SIZE as usize];
        ram.read(table.checked_add(DESC_SIZE * u64::from(index))?, &mut raw)?;
        Some(Self {
            addr: u64::from_le_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]),
            len: u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
            flags: u16::from_le_bytes([raw[12], raw[13]]),
            next: u16::from_le_bytes([raw[14], raw[15]]),
        })
    }

    fn device_writes(&self) -> bool {
        self.flags & VRING_DESC_F_WRITE != 0
    }
}

/// The ring features a driver accepted, as the queue code consults them
/// (QEMU `virtio_vdev_has_feature`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RingFeatures {
    /// `VIRTIO_RING_F_EVENT_IDX`.
    pub event_idx: bool,
    /// `VIRTIO_F_NOTIFY_ON_EMPTY`.
    pub notify_on_empty: bool,
}

/// A descriptor chain the device took from the available ring: where its
/// descriptors are, for walking its buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chain {
    /// The head descriptor's index, which identifies the chain in the used
    /// ring.
    pub head: u16,
    /// The descriptor table the chain is in: the queue's, or an indirect one.
    table: u64,
    /// Entries in `table`.
    table_len: u32,
    /// The chain's first descriptor in `table`.
    first: u32,
    /// How many device-readable and device-writable buffers it has (QEMU
    /// `out_num` and `in_num`).
    pub readable: u32,
    pub writable: u32,
    /// The total length of each (QEMU `iov_size` of `out_sg` and `in_sg`).
    pub readable_len: u64,
    pub writable_len: u64,
}

/// Where a walk of a chain has got to.
struct Walk {
    next: Option<u32>,
    steps: u32,
}

/// One buffer of a chain, as the driver described it.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buffer {
    pub addr: u64,
    pub len: u32,
    pub device_writes: bool,
}

/// A walk over a chain's buffers in order. It holds no borrow of guest
/// memory between steps, so a device can fill one buffer before it looks at
/// the next.
#[cfg(feature = "alloc")]
pub struct Buffers {
    chain: Chain,
    walk: Walk,
}

#[cfg(feature = "alloc")]
impl Buffers {
    /// The next buffer, or `None` past the last.
    pub fn next_buffer(&mut self, ram: &GuestRam<'_>) -> Option<Buffer> {
        let desc = self.chain.step(ram, &mut self.walk)?;
        Some(Buffer {
            addr: desc.addr,
            len: desc.len,
            device_writes: desc.device_writes(),
        })
    }
}

impl Chain {
    fn walk(&self) -> Walk {
        Walk {
            next: Some(self.first),
            steps: 0,
        }
    }

    /// Its buffers, one at a time.
    #[cfg(feature = "alloc")]
    pub fn buffers(&self) -> Buffers {
        Buffers {
            chain: *self,
            walk: self.walk(),
        }
    }

    /// The next descriptor of a walk, bounded by the table's length.
    fn step(&self, ram: &GuestRam<'_>, walk: &mut Walk) -> Option<Descriptor> {
        let index = walk.next?;
        if walk.steps == self.table_len {
            return None;
        }
        walk.steps += 1;
        let desc = Descriptor::read(ram, self.table, index)?;
        walk.next = (desc.flags & VRING_DESC_F_NEXT != 0 && u32::from(desc.next) < self.table_len)
            .then_some(u32::from(desc.next));
        Some(desc)
    }

    /// QEMU `iov_to_buf` over the device-readable buffers: copy what lies
    /// `offset` bytes in into `out`, returning how many bytes there were.
    pub fn read(&self, ram: &GuestRam<'_>, offset: usize, out: &mut [u8]) -> usize {
        let mut skip = offset as u64;
        let mut done = 0;
        let mut walk = self.walk();
        while done < out.len() {
            let Some(desc) = self.step(ram, &mut walk) else {
                break;
            };
            if desc.device_writes() {
                break;
            }
            let len = u64::from(desc.len);
            if skip >= len {
                skip -= len;
                continue;
            }
            let n = ((len - skip) as usize).min(out.len() - done);
            let Some(at) = desc.addr.checked_add(skip) else {
                break;
            };
            if ram.read(at, &mut out[done..done + n]).is_none() {
                break;
            }
            done += n;
            skip = 0;
        }
        done
    }

    /// QEMU `iov_from_buf` over the device-writable buffers: store `data`
    /// starting `offset` bytes in, returning how many bytes fitted.
    pub fn write(&self, ram: &mut GuestRam<'_>, offset: usize, data: &[u8]) -> usize {
        let mut skip = offset as u64;
        let mut done = 0;
        let mut walk = self.walk();
        while done < data.len() {
            let Some(desc) = self.step(ram, &mut walk) else {
                break;
            };
            if !desc.device_writes() {
                continue;
            }
            let len = u64::from(desc.len);
            if skip >= len {
                skip -= len;
                continue;
            }
            let n = ((len - skip) as usize).min(data.len() - done);
            let Some(at) = desc.addr.checked_add(skip) else {
                break;
            };
            ram.store(at, &data[done..done + n]);
            done += n;
            skip = 0;
        }
        done
    }
}

/// One virtqueue's device-side state (QEMU `VirtQueue`).
#[derive(Debug, Clone, Copy)]
pub struct Queue {
    /// `vring.num_default`: the size the device gave the queue, which
    /// `QueueNumMax` reports and a driver may not exceed. This is QEMU
    /// 11.1.2's `virt-11.1` machine; `hw_compat_11_0` overrides every
    /// virtio-mmio queue to 1024 on `virt-11.0` and older.
    num_default: u16,
    /// `vring.num`.
    num: u16,
    /// `vring.desc`, `vring.avail`, `vring.used`: the rings, as the driver
    /// last enabled the queue with.
    desc: u64,
    avail: u64,
    used: u64,
    /// The rings lie in guest memory (QEMU's region caches exist).
    mapped: bool,
    last_avail_idx: u16,
    /// The available index as the device last read it.
    shadow_avail_idx: u16,
    used_idx: u16,
    signalled_used: u16,
    signalled_used_valid: bool,
    /// Chains taken and not yet returned.
    inuse: u16,
    /// Whether the device wants the driver's notifications (QEMU
    /// `vq->notification`). Off only while a device works through the
    /// queue, so between guest accesses it is always on.
    notification: bool,
}

impl Queue {
    /// A queue of `size` entries, as QEMU's `virtio_add_queue` makes it; a
    /// size of 0 is a queue the device does not have.
    pub const fn new(size: u16) -> Self {
        Self {
            num_default: size,
            num: size,
            desc: 0,
            avail: 0,
            used: 0,
            mapped: false,
            last_avail_idx: 0,
            shadow_avail_idx: 0,
            used_idx: 0,
            signalled_used: 0,
            signalled_used_valid: false,
            inuse: 0,
            notification: true,
        }
    }

    /// `QueueNumMax`.
    pub fn num_default(&self) -> u16 {
        self.num_default
    }

    /// QEMU `__virtio_queue_reset`.
    pub fn reset(&mut self) {
        *self = Self::new(self.num_default);
    }

    /// QEMU `virtio_queue_ready`: the driver has given the queue an
    /// available ring.
    #[cfg(feature = "alloc")]
    pub fn is_ready(&self) -> bool {
        self.avail != 0
    }

    /// The driver has given the queue a descriptor table, which QEMU's
    /// `virtio_queue_notify` requires before it runs the device.
    pub fn has_descriptors(&self) -> bool {
        self.desc != 0
    }

    /// QEMU `virtio_queue_set_num`: a size the queue cannot take is ignored,
    /// one larger than the device's is an error.
    pub fn set_num(&mut self, num: i32) -> Result<(), QueueFault> {
        if (num != 0) != (self.num != 0) || num > i32::from(VIRTQUEUE_MAX_SIZE) || num < 0 {
            return Ok(());
        }
        if num > i32::from(self.num_default) {
            return Err(QueueFault::QueueTooLarge);
        }
        self.num = num as u16;
        Ok(())
    }

    /// QEMU `virtio_queue_set_rings`.
    pub fn set_rings(
        &mut self,
        ram: &GuestRam<'_>,
        desc: u64,
        avail: u64,
        used: u64,
        features: RingFeatures,
    ) -> Result<(), QueueFault> {
        if self.num == 0 {
            return Ok(());
        }
        self.desc = desc;
        self.avail = avail;
        self.used = used;
        self.map_rings(ram, features)
    }

    /// QEMU `virtio_init_region_cache`: map the rings at their current size,
    /// or none while there is no descriptor table.
    pub fn map_rings(
        &mut self,
        ram: &GuestRam<'_>,
        features: RingFeatures,
    ) -> Result<(), QueueFault> {
        self.mapped = false;
        if self.desc == 0 {
            return Ok(());
        }
        let num = u64::from(self.num);
        let event = if features.event_idx { 2 } else { 0 };
        let mapped = ram.contains(self.desc, DESC_SIZE * num)
            && ram.contains(self.used, 4 + 8 * num + event)
            && ram.contains(self.avail, 4 + 2 * num + event);
        if !mapped {
            return Err(QueueFault::UnmappableRing);
        }
        self.mapped = true;
        Ok(())
    }

    /// A 16-bit field of the available ring, 0 while it is not mapped.
    fn avail_u16(&self, ram: &GuestRam<'_>, offset: u64) -> u16 {
        if !self.mapped {
            return 0;
        }
        ram.read_u16(self.avail + offset).unwrap_or(0)
    }

    /// Store a 16-bit field of the used ring, if it is mapped.
    fn set_used_u16(&self, ram: &mut GuestRam<'_>, offset: u64, value: u16) {
        if self.mapped {
            ram.store(self.used + offset, &value.to_le_bytes());
        }
    }

    /// QEMU `vring_avail_idx`.
    fn read_avail_idx(&mut self, ram: &GuestRam<'_>) -> u16 {
        if !self.mapped {
            return 0;
        }
        self.shadow_avail_idx = self.avail_u16(ram, 2);
        self.shadow_avail_idx
    }

    /// How many chains the driver has made available that the device has not
    /// taken, the available index read afresh: how many successive pops find
    /// the queue not empty, since QEMU's `virtio_queue_split_empty` reads
    /// the index again whenever the device has caught up with the last
    /// read. An index further ahead than the queue holds is counted as it
    /// stands; QEMU's pop does not check it, and the chains past the queue's
    /// size fail as theirs do (`SizeExceeded`, `BadHead`).
    pub fn available(&mut self, ram: &GuestRam<'_>) -> u16 {
        if self.avail == 0 {
            return 0;
        }
        self.read_avail_idx(ram).wrapping_sub(self.last_avail_idx)
    }

    /// QEMU `virtio_queue_split_empty`.
    fn is_empty(&mut self, ram: &GuestRam<'_>, disabled: bool) -> bool {
        if disabled || self.avail == 0 {
            return true;
        }
        if self.shadow_avail_idx != self.last_avail_idx {
            return false;
        }
        self.read_avail_idx(ram) == self.last_avail_idx
    }

    /// QEMU `virtqueue_split_pop`: take the next chain the driver made
    /// available, after checking every descriptor of it. The caller has
    /// checked that the device is not disabled.
    pub fn pop(
        &mut self,
        ram: &mut GuestRam<'_>,
        features: RingFeatures,
    ) -> Result<Option<Chain>, QueueFault> {
        if self.num == 0 || self.is_empty(ram, false) {
            return Ok(None);
        }
        if self.inuse >= self.num {
            return Err(QueueFault::SizeExceeded);
        }
        let head = self.avail_u16(ram, 4 + 2 * u64::from(self.last_avail_idx % self.num));
        self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
        if head >= self.num {
            return Err(QueueFault::BadHead);
        }
        if features.event_idx {
            self.set_avail_event(ram, self.last_avail_idx);
        }
        if !self.mapped {
            return Err(QueueFault::NoRings);
        }

        let mut table = self.desc;
        let mut max = u32::from(self.num);
        let mut index = u32::from(head);
        let mut desc = Descriptor::read(ram, table, index).ok_or(QueueFault::UnmappableRing)?;
        if desc.flags & VRING_DESC_F_INDIRECT != 0 {
            if desc.len == 0 || u64::from(desc.len) % DESC_SIZE != 0 {
                return Err(QueueFault::BadIndirectSize);
            }
            if !ram.contains(desc.addr, u64::from(desc.len)) {
                return Err(QueueFault::UnmappableIndirect);
            }
            table = desc.addr;
            max = desc.len / DESC_SIZE as u32;
            index = 0;
            desc = Descriptor::read(ram, table, index).ok_or(QueueFault::UnmappableIndirect)?;
        }
        let first = index;

        let max_buffers = u32::from(VIRTQUEUE_MAX_SIZE);
        let (mut readable, mut writable, mut entries) = (0u32, 0u32, 0u32);
        let (mut readable_len, mut writable_len) = (0u64, 0u64);
        loop {
            if desc.device_writes() {
                Self::map_buffer(ram, &desc, readable + writable, max_buffers)?;
                writable += 1;
                writable_len += u64::from(desc.len);
            } else {
                if writable != 0 {
                    return Err(QueueFault::OutOfOrder);
                }
                Self::map_buffer(ram, &desc, readable, max_buffers)?;
                readable += 1;
                readable_len += u64::from(desc.len);
            }
            entries += 1;
            if entries > max {
                return Err(QueueFault::Looped);
            }
            if desc.flags & VRING_DESC_F_NEXT == 0 {
                break;
            }
            if u32::from(desc.next) >= max {
                return Err(QueueFault::BadNext);
            }
            index = u32::from(desc.next);
            desc = Descriptor::read(ram, table, index).ok_or(QueueFault::UnmappableBuffer)?;
        }
        self.inuse += 1;
        Ok(Some(Chain {
            head,
            table,
            table_len: max,
            first,
            readable,
            writable,
            readable_len,
            writable_len,
        }))
    }

    /// QEMU `vring_set_avail_event`: tell the driver where the device has
    /// got to, unless it has turned notifications off.
    fn set_avail_event(&self, ram: &mut GuestRam<'_>, idx: u16) {
        if self.notification {
            // used->ring[num].
            self.set_used_u16(ram, 4 + 8 * u64::from(self.num), idx);
        }
    }

    /// QEMU `virtio_queue_set_notification` for a split ring: with event
    /// indices, publish the available index just read (once notifications
    /// are on again); without, set or clear `VRING_USED_F_NO_NOTIFY`.
    #[cfg(feature = "alloc")]
    pub fn set_notification(
        &mut self,
        ram: &mut GuestRam<'_>,
        features: RingFeatures,
        enable: bool,
    ) {
        self.notification = enable;
        if self.desc == 0 {
            return;
        }
        if features.event_idx {
            let idx = self.read_avail_idx(ram);
            self.set_avail_event(ram, idx);
        } else if self.mapped {
            let flags = ram.read_u16(self.used).unwrap_or(0);
            let flags = if enable {
                flags & !VRING_USED_F_NO_NOTIFY
            } else {
                flags | VRING_USED_F_NO_NOTIFY
            };
            self.set_used_u16(ram, 0, flags);
        }
    }

    /// QEMU `virtqueue_detach_element`: forget a chain taken from the queue
    /// without returning it.
    #[cfg(feature = "alloc")]
    pub fn detach(&mut self) {
        self.inuse = self.inuse.wrapping_sub(1);
    }

    /// QEMU `virtqueue_map_desc` for one buffer, `mapped` buffers already
    /// mapped of at most `limit`.
    fn map_buffer(
        ram: &GuestRam<'_>,
        desc: &Descriptor,
        mapped: u32,
        limit: u32,
    ) -> Result<(), QueueFault> {
        if desc.len == 0 {
            return Err(QueueFault::ZeroSized);
        }
        if mapped == limit {
            return Err(QueueFault::TooManyBuffers);
        }
        if !ram.contains(desc.addr, u64::from(desc.len)) {
            return Err(QueueFault::UnmappableBuffer);
        }
        Ok(())
    }

    /// QEMU `virtqueue_unpop`: put the chain taken last back, for the next
    /// `pop` to take again.
    pub fn unpop(&mut self) {
        self.last_avail_idx = self.last_avail_idx.wrapping_sub(1);
        self.inuse = self.inuse.wrapping_sub(1);
    }

    /// QEMU `virtqueue_push` (`virtqueue_split_fill` and
    /// `virtqueue_split_flush` of one chain): hand `head` back with `len`
    /// bytes of it written. A disabled device writes nothing.
    pub fn push(&mut self, ram: &mut GuestRam<'_>, head: u16, len: u32, disabled: bool) {
        if !disabled && self.used != 0 && self.mapped {
            let slot = u64::from(self.used_idx % self.num);
            let mut elem = [0u8; 8];
            elem[..4].copy_from_slice(&u32::from(head).to_le_bytes());
            elem[4..].copy_from_slice(&len.to_le_bytes());
            ram.store(self.used + 4 + 8 * slot, &elem);
        }
        if disabled {
            self.inuse = self.inuse.wrapping_sub(1);
            return;
        }
        if self.used == 0 {
            return;
        }
        let old = self.used_idx;
        let new = old.wrapping_add(1);
        self.set_used_u16(ram, 2, new);
        self.used_idx = new;
        self.inuse = self.inuse.wrapping_sub(1);
        if i32::from(new.wrapping_sub(self.signalled_used) as i16)
            < i32::from(new.wrapping_sub(old))
        {
            self.signalled_used_valid = false;
        }
    }

    /// QEMU `virtio_split_should_notify`: whether the driver wants an
    /// interrupt for what the device has used.
    pub fn should_notify(
        &mut self,
        ram: &GuestRam<'_>,
        features: RingFeatures,
        disabled: bool,
    ) -> bool {
        if features.notify_on_empty && self.inuse == 0 && self.is_empty(ram, disabled) {
            return true;
        }
        if !features.event_idx {
            return self.avail_u16(ram, 0) & VRING_AVAIL_F_NO_INTERRUPT == 0;
        }
        let valid = self.signalled_used_valid;
        self.signalled_used_valid = true;
        let old = self.signalled_used;
        let new = self.used_idx;
        self.signalled_used = new;
        // vring_get_used_event: avail->ring[num].
        let used_event = self.avail_u16(ram, 4 + 2 * u64::from(self.num));
        !valid || vring_need_event(used_event, new, old)
    }

    pub fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u16(self.num)?;
        w.write_u64(self.desc)?;
        w.write_u64(self.avail)?;
        w.write_u64(self.used)?;
        w.write_bool(self.mapped)?;
        for v in [
            self.last_avail_idx,
            self.shadow_avail_idx,
            self.used_idx,
            self.signalled_used,
            self.inuse,
        ] {
            w.write_u16(v)?;
        }
        w.write_bool(self.signalled_used_valid)
    }

    pub fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let num = r.read_u16()?;
        // A queue the device has is never sized to nothing, and one it does
        // not have never to anything else.
        if (num == 0) != (self.num_default == 0) || num > self.num_default {
            return Err(SnapError::Invalid(
                "virtqueue size the device does not allow",
            ));
        }
        self.num = num;
        self.desc = r.read_u64()?;
        self.avail = r.read_u64()?;
        self.used = r.read_u64()?;
        self.mapped = r.read_bool()?;
        self.last_avail_idx = r.read_u16()?;
        self.shadow_avail_idx = r.read_u16()?;
        self.used_idx = r.read_u16()?;
        self.signalled_used = r.read_u16()?;
        self.inuse = r.read_u16()?;
        if self.inuse > self.num {
            return Err(SnapError::Invalid(
                "virtqueue with more chains in use than it holds",
            ));
        }
        self.signalled_used_valid = r.read_bool()?;
        Ok(())
    }
}

/// `vring_need_event` (Linux `include/uapi/linux/virtio_ring.h`): whether
/// moving the used index from `old` to `new` passed `event_idx`.
fn vring_need_event(event_idx: u16, new: u16, old: u16) -> bool {
    new.wrapping_sub(event_idx).wrapping_sub(1) < new.wrapping_sub(old)
}
