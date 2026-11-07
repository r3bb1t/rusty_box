//! virtio-blk: QEMU's `virtio-blk-device`.
//!
//! Provenance (R7): the virtio 1.2 specification, section 5.2, as QEMU
//! implements it in `hw/block/virtio-blk.c` for a raw image given with every
//! default (`-drive file=…,format=raw,if=none,id=hd0 -device
//! virtio-blk-device,drive=hd0`: one queue of 256 entries, a write-back
//! cache, discard requests ignored, no serial number, not zoned):
//!
//! - `virtio_blk_get_features` and `virtio_blk_device_realize`: the
//!   features offered (`VIRTIO_BLK_F_RO` too over a drive given
//!   `readonly=on`, whose writes QEMU's block layer refuses);
//! - `virtio_blk_update_config`: the capacity, `seg_max` two less than the
//!   queue, 512-byte logical and physical blocks (`blkconf_blocksizes`,
//!   which finds no block sizes to probe in a regular file), the cache
//!   mode, one queue, and one range per discard and write-zeroes request;
//! - `virtio_blk_set_config` and `virtio_blk_reset`: the driver chooses the
//!   cache mode through `writeback`; a reset brings back the write-back
//!   cache;
//! - `virtio_blk_handle_vq` and `virtio_blk_handle_request`: the headers
//!   every request must have (a `virtio_error` when it has not), reads and
//!   writes checked by `virtio_blk_sect_range_ok`, a write-through cache
//!   syncing each write, flushes, `VIRTIO_BLK_T_GET_ID`, and discard and
//!   write-zeroes (`virtio_blk_handle_discard_write_zeroes`);
//! - `virtio_blk_req_complete`: the status in the last byte of the last
//!   device-writable buffer.
//!
//! Where it follows the specification over QEMU (README, "Where the
//! references differ"):
//!
//! - Every device-writable byte is written, those a request leaves zeroed,
//!   so the used length that counts them all is true (2.7.8.2: "The device
//!   MUST write at least len bytes ... beginning at the first
//!   device-writable buffer") and reaches the status byte; QEMU counts
//!   every device-writable byte and leaves those a failed or unsupported
//!   request did not write as they were.
//! - The cache mode is set when the driver sets `FEATURES_OK` (5.2.5.2:
//!   with `VIRTIO_BLK_F_CONFIG_WCE` but not `VIRTIO_BLK_F_FLUSH`
//!   negotiated, `writeback` is initialised to 0; without either, writes
//!   are stable), and `writeback` changes it only once
//!   `VIRTIO_BLK_F_CONFIG_WCE` is negotiated (5.2.6.1); QEMU's
//!   `virtio_blk_set_status` sets it at `DRIVER_OK` and only without
//!   `VIRTIO_BLK_F_CONFIG_WCE`.
//! - The configuration runs to the end of `unused1`, which reads 0
//!   (5.2.5.2: "The device MUST initialize padding bytes unused0 and
//!   unused1 to 0"); QEMU's ends after `write_zeroes_may_unmap`, past
//!   which the transport reads all ones.
//! - A discard or write-zeroes range with a flag the request does not take
//!   is `VIRTIO_BLK_S_UNSUPP` whether or not its sectors lie on the disk
//!   (5.2.6.2); QEMU checks the sectors first.
//! - `VIRTIO_BLK_T_GET_ID` writes the identifier NUL-padded to 20 bytes
//!   (5.2.6); QEMU writes it with one NUL.
//! - The request types are 1.2's (5.2.6) exactly: the legacy SCSI command,
//!   the legacy flush and barrier variants and the zone commands 1.2 does
//!   not have are each `VIRTIO_BLK_S_UNSUPP`, as any type the device does
//!   not support is; QEMU also takes SCSI requests apart, masks the barrier
//!   and output bits off the type, and answers zone commands as a device
//!   that is not zoned.
//!
//! Where it differs from QEMU otherwise:
//!
//! - The device offers no legacy geometry (`VIRTIO_BLK_F_GEOMETRY`, optional
//!   in 5.2.3), so `geometry` reads 0, valid only with the feature (5.2.4);
//!   QEMU offers the cylinders, heads and sectors `hd_geometry_guess`
//!   (`hw/block/hd-geometry.c`) makes up from the image.
//! - A request has no length limit of the device's own: one moves as many
//!   sectors as lie in the disk, and `max_discard_sectors` and
//!   `max_write_zeroes_sectors` are the most their le32 fields hold; QEMU
//!   refuses a request of more than `BDRV_REQUEST_MAX_SECTORS` (2³¹ − 1
//!   bytes, in sectors) and reports that as both limits.
//! - A request completes before the driver's notification returns, or its
//!   `Status` write of `DRIVER_OK`, which takes what the queue already holds
//!   (`super`'s `Transport::write_status`). QEMU gives reads, writes and
//!   flushes to its block layer, which completes them afterwards, in an
//!   order and at a time the host decides; the guest sees the same results,
//!   sooner and always at the same instruction. A notification takes the
//!   requests available when it was made, the ones QEMU takes (see
//!   [`Block::handle_output`]).
//! - A host error is reported to the guest as `VIRTIO_BLK_S_IOERR`. That is
//!   QEMU's default for every failed read and for every failed write but
//!   one: a write that finds the host's disk full stops QEMU's machine
//!   (`werror=enospc`) until the user resumes it and the write is retried.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::queue::{Chain, QueueFault};
use super::{Core, Vdev};
use crate::disk::{Access, Disk, DiskOffset, SECTOR_SIZE};

/// The device's one queue.
pub const QUEUE: usize = 0;
/// QEMU's `queue-size` default.
pub const QUEUE_SIZE: u16 = 256;

const F_SEG_MAX: u64 = 1 << 2;
/// Offered over a read-only disk.
const F_RO: u64 = 1 << 5;
const F_BLK_SIZE: u64 = 1 << 6;
/// `VIRTIO_BLK_F_WCE`, also `VIRTIO_BLK_F_FLUSH`: offered because the cache
/// is write-back when the device is made.
const F_WCE: u64 = 1 << 9;
const F_TOPOLOGY: u64 = 1 << 10;
const F_CONFIG_WCE: u64 = 1 << 11;
const F_DISCARD: u64 = 1 << 13;
const F_WRITE_ZEROES: u64 = 1 << 14;

/// The device's own feature bits over a disk the guest may write.
pub const FEATURES: u64 = F_SEG_MAX
    | F_BLK_SIZE
    | F_WCE
    | F_TOPOLOGY
    | F_CONFIG_WCE
    | F_DISCARD
    | F_WRITE_ZEROES;

/// `struct virtio_blk_config` up to the end of `unused1`: the secure erase
/// fields after it exist only with `VIRTIO_BLK_F_SECURE_ERASE`.
pub const CONFIG_LEN: usize = 60;

/// `logical_block_size` (and `physical_block_size`).
const BLOCK_SIZE: u64 = 512;
/// `max_discard_sectors` and `max_write_zeroes_sectors`: the most the le32
/// fields hold, the device having no limit of its own.
const RANGE_MAX_SECTORS: u32 = u32::MAX;
/// `seg_max`: the most data buffers a request can have. A descriptor chain
/// is at most the queue size long (virtio 1.2 2.7.5: "this is the maximum
/// possible descriptor chain length"; 2.7.5.3.1: "A driver MUST NOT create
/// a descriptor chain longer than the Queue Size of the device"), and every
/// request spends one descriptor on its header and one on its status.
const SEG_MAX: u32 = QUEUE_SIZE as u32 - 2;
/// `VIRTIO_BLK_ID_BYTES`.
const ID_BYTES: u64 = 20;
/// QEMU's `serial` property, unset.
const SERIAL: &[u8] = b"";

/// `struct virtio_blk_outhdr`: type, priority, sector.
const OUT_HEADER_LEN: usize = 16;
/// `struct virtio_blk_discard_write_zeroes`: sector, sectors, flags.
const RANGE_LEN: usize = 16;

const T_IN: u32 = 0;
const T_OUT: u32 = 1;
const T_FLUSH: u32 = 4;
const T_GET_ID: u32 = 8;
const T_DISCARD: u32 = 11;
const T_WRITE_ZEROES: u32 = 13;

const WRITE_ZEROES_FLAG_UNMAP: u32 = 1;

/// A request's outcome, written to its status byte.
///
/// Closed (R5): virtio 1.2's three (5.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok = 0,
    IoError = 1,
    Unsupported = 2,
}

/// What a request did: its status, and how many bytes it wrote from the
/// first device-writable byte on before the status (`data`), past which
/// [`Block::complete`] zeroes the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Outcome {
    status: Status,
    data: u64,
}

impl Outcome {
    /// A request that wrote nothing but its status.
    const fn status(status: Status) -> Self {
        Self { status, data: 0 }
    }
}

/// Whether the host may hold a write before it reaches the disk (QEMU
/// `blk_enable_write_cache`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cache {
    /// A write is done once the host has it; a flush makes it durable.
    WriteBack,
    /// Every write is made durable before it completes.
    WriteThrough,
}

/// QEMU's `VirtIOBlock` over its disk.
pub struct Block {
    disk: Disk,
    cache: Cache,
}

impl Block {
    /// A device over `disk`, its cache write-back.
    pub fn new(disk: Disk) -> Self {
        Self {
            disk,
            cache: Cache::WriteBack,
        }
    }

    pub fn disk(&self) -> &Disk {
        &self.disk
    }

    pub fn disk_mut(&mut self) -> &mut Disk {
        &mut self.disk
    }

    /// QEMU `virtio_blk_update_config`.
    pub fn get_config(&self, config: &mut [u8]) {
        let mut c = [0u8; CONFIG_LEN];
        c[0..8].copy_from_slice(&self.disk.sectors().to_le_bytes());
        // size_max stays 0, and so does the geometry, which is not offered.
        c[12..16].copy_from_slice(&SEG_MAX.to_le_bytes());
        c[20..24].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes());
        // physical_block_exp, alignment_offset, min_io_size and opt_io_size
        // stay 0: the physical block is the logical one.
        c[32] = u8::from(self.cache == Cache::WriteBack);
        c[34..36].copy_from_slice(&1u16.to_le_bytes());
        c[36..40].copy_from_slice(&RANGE_MAX_SECTORS.to_le_bytes());
        c[40..44].copy_from_slice(&1u32.to_le_bytes());
        // discard_sector_alignment: the discard granularity, the block size,
        // in sectors.
        c[44..48].copy_from_slice(&((BLOCK_SIZE / SECTOR_SIZE) as u32).to_le_bytes());
        c[48..52].copy_from_slice(&RANGE_MAX_SECTORS.to_le_bytes());
        c[52..56].copy_from_slice(&1u32.to_le_bytes());
        c[56] = 1;
        config.copy_from_slice(&c[..config.len()]);
    }

    /// QEMU `virtio_blk_set_config`: `writeback` chooses the cache mode once
    /// the driver has accepted `VIRTIO_BLK_F_CONFIG_WCE` (5.2.6.1).
    pub fn set_config(&mut self, config: &[u8], core: &Core) {
        if !core.has_feature(F_CONFIG_WCE) {
            return;
        }
        self.cache = if config[32] != 0 {
            Cache::WriteBack
        } else {
            Cache::WriteThrough
        };
    }

    /// The cache mode once the driver sets `FEATURES_OK`, the first time
    /// it may read `writeback` (5.2.5.1). Without `VIRTIO_BLK_F_FLUSH`,
    /// which this device offers, writes are stable (5.2.6.2, case 1) and
    /// `writeback` is 0 (5.2.5.2); with it but without
    /// `VIRTIO_BLK_F_CONFIG_WCE` the cache is write-back, as the driver
    /// should assume (5.2.5.1); with both, the specification leaves the
    /// starting mode to the device, and it is write-back, the driver free
    /// to change it through `writeback`.
    pub fn features_ok(&mut self, core: &Core) {
        self.cache = if core.has_feature(F_WCE) {
            Cache::WriteBack
        } else {
            Cache::WriteThrough
        };
    }

    /// QEMU `virtio_blk_reset`: no request is ever left in flight, so only
    /// the cache mode goes back to what it was.
    pub fn reset(&mut self) {
        self.cache = Cache::WriteBack;
    }

    /// The device's feature bits: [`FEATURES`], and `VIRTIO_BLK_F_RO` over a
    /// read-only disk.
    pub fn features(&self) -> u64 {
        match self.disk.access() {
            Access::ReadWrite => FEATURES,
            Access::ReadOnly => FEATURES | F_RO,
        }
    }

    /// QEMU `virtio_blk_handle_output` and `virtio_blk_handle_vq`, for a
    /// notification or the driver's setting `DRIVER_OK`.
    ///
    /// QEMU's vCPU stays in the notifying write while the device works
    /// through the queue, and its block layer writes guest memory only after
    /// that, so the requests it takes are the ones available when the driver
    /// notified, and its check that the queue is empty before it returns
    /// finds no more. Taking no more here leaves a request this device's own
    /// reads made available for the next notification, as on QEMU, and
    /// keeps a guest from holding the device in one notification forever.
    pub fn handle_output(&mut self, vdev: &mut Vdev<'_, '_>, q: usize) {
        if q != QUEUE {
            return;
        }
        let mut available = vdev.available(QUEUE);
        vdev.set_notification(QUEUE, false);
        while available > 0 {
            let Some(chain) = vdev.pop(QUEUE) else {
                break;
            };
            available -= 1;
            if let Err(fault) = self.handle_request(vdev, &chain) {
                vdev.error(fault);
                vdev.detach(QUEUE);
                break;
            }
        }
        vdev.set_notification(QUEUE, true);
    }

    /// QEMU `virtio_blk_handle_request`. An `Err` is the `virtio_error` that
    /// leaves the chain taken and unanswered.
    fn handle_request(&mut self, vdev: &mut Vdev<'_, '_>, chain: &Chain) -> Result<(), QueueFault> {
        if chain.readable < 1 || chain.writable < 1 {
            return Err(QueueFault::BlockMissingHeaders);
        }
        let mut header = [0u8; OUT_HEADER_LEN];
        if vdev.read(chain, 0, &mut header) != OUT_HEADER_LEN {
            return Err(QueueFault::BlockHeaderShort);
        }
        // Where the status goes, found before the request writes anything,
        // as QEMU keeps a pointer to it (`req->in`).
        let status_at = Self::status_byte(vdev, chain).ok_or(QueueFault::BlockMissingHeaders)?;
        let kind = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let sector = u64::from_le_bytes([
            header[8], header[9], header[10], header[11], header[12], header[13], header[14],
            header[15],
        ]);
        let outcome = match kind {
            T_IN => self.read(vdev, chain, sector),
            T_OUT => Outcome::status(self.write(vdev, chain, sector)),
            T_FLUSH => Outcome::status(self.sync()),
            T_GET_ID => Self::get_id(vdev, chain),
            T_DISCARD | T_WRITE_ZEROES => {
                let range_room = chain.readable_len - OUT_HEADER_LEN as u64;
                if range_room > RANGE_LEN as u64 {
                    Outcome::status(Status::Unsupported)
                } else {
                    let mut range = [0u8; RANGE_LEN];
                    if vdev.read(chain, OUT_HEADER_LEN, &mut range) != RANGE_LEN {
                        return Err(QueueFault::BlockRangeShort);
                    }
                    Outcome::status(self.discard_write_zeroes(&range, kind == T_WRITE_ZEROES))
                }
            }
            _ => Outcome::status(Status::Unsupported),
        };
        Self::complete(vdev, chain, status_at, outcome);
        Ok(())
    }

    /// `VIRTIO_BLK_T_GET_ID`: the identifier, NUL-padded to 20 bytes (5.2.6),
    /// as much of it as the buffers before the status byte hold.
    fn get_id(vdev: &mut Vdev<'_, '_>, chain: &Chain) -> Outcome {
        let mut id = [0u8; ID_BYTES as usize];
        id[..SERIAL.len()].copy_from_slice(SERIAL);
        let len = (chain.writable_len - 1).min(ID_BYTES) as usize;
        // `len` leaves the status byte free, so the whole identifier fits
        // the chain that was just checked.
        let written = vdev.write(chain, 0, &id[..len]);
        Outcome {
            status: if written == len {
                Status::Ok
            } else {
                Status::IoError
            },
            data: written as u64,
        }
    }

    /// The last byte of the chain's last device-writable buffer with bytes,
    /// the status byte's place (`struct virtio_blk_inhdr`); `None` for a
    /// chain with no device-writable byte.
    fn status_byte(vdev: &Vdev<'_, '_>, chain: &Chain) -> Option<u64> {
        let mut buffers = chain.buffers();
        let mut last = None;
        while let Some(buffer) = buffers.next_buffer(vdev.ram) {
            if buffer.device_writes {
                last = Some(buffer.addr + u64::from(buffer.len) - 1);
            }
        }
        last
    }

    /// QEMU `virtio_blk_req_complete`: the device-writable bytes the request
    /// did not write before the status zeroed, the status, then the chain
    /// back with every device-writable byte counted, all of them now written
    /// (2.7.8.2: "The device MUST write at least len bytes ... beginning at
    /// the first device-writable buffer"). The status byte is then within
    /// the used length, which a driver may not look past (2.7.8.3), and a
    /// failed read leaves zeros, not what the buffers held. Zeroing costs
    /// no more than the RAM the buffers cover ([`Chain::zero`]).
    ///
    /// The request's own writes may have rewritten its descriptors, so the
    /// chain walked again can end sooner; the used length is then the
    /// bytes written from the start, the status byte past them. A used
    /// length is 32 bits, and a chain with more device-writable bytes is
    /// given back with the most it can say, all of them written.
    fn complete(vdev: &mut Vdev<'_, '_>, chain: &Chain, status_at: u64, outcome: Outcome) {
        let data_len = chain.writable_len - 1;
        let written = if outcome.data < data_len {
            let rest = data_len - outcome.data;
            let zeroed = chain.zero(vdev.ram, outcome.data, rest);
            if zeroed == rest {
                chain.writable_len
            } else {
                outcome.data + zeroed
            }
        } else {
            chain.writable_len
        };
        vdev.ram.store(status_at, &[outcome.status as u8]);
        vdev.push(QUEUE, chain, u32::try_from(written).unwrap_or(u32::MAX));
        vdev.notify(QUEUE);
    }

    /// QEMU `virtio_blk_sect_range_ok`, without its request limit: `size`
    /// bytes from `sector` are whole blocks inside the disk.
    fn range_ok(&self, sector: u64, size: u64) -> bool {
        let count = size / SECTOR_SIZE;
        let total = self.disk.sectors();
        size.is_multiple_of(BLOCK_SIZE) && sector <= total && count <= total - sector
    }

    /// A read: the disk into the device-writable buffers, all but the
    /// status byte. A buffer the host fails to fill counts as not written,
    /// and is zeroed with what follows it.
    fn read(&mut self, vdev: &mut Vdev<'_, '_>, chain: &Chain, sector: u64) -> Outcome {
        let size = chain.writable_len - 1;
        if !self.range_ok(sector, size) {
            return Outcome::status(Status::IoError);
        }
        let mut at = DiskOffset::of_sector(sector);
        let mut left = size;
        let mut buffers = chain.buffers();
        while left > 0 {
            let Some(buffer) = buffers.next_buffer(vdev.ram) else {
                break;
            };
            if !buffer.device_writes {
                continue;
            }
            let n = u64::from(buffer.len).min(left);
            let Some(dst) = vdev.ram.bytes_mut(buffer.addr, n) else {
                break;
            };
            if self.disk.read_at(at, dst).is_err() {
                return Outcome {
                    status: Status::IoError,
                    data: size - left,
                };
            }
            at = at.after(n);
            left -= n;
        }
        // Every buffer was checked to lie in RAM when the chain was taken.
        let status = if left > 0 {
            Status::IoError
        } else {
            Status::Ok
        };
        Outcome {
            status,
            data: size - left,
        }
    }

    /// A write: the device-readable buffers after the header onto the disk.
    /// A read-only disk refuses it whatever its length, as QEMU's block layer
    /// does (`-EPERM`).
    fn write(&mut self, vdev: &mut Vdev<'_, '_>, chain: &Chain, sector: u64) -> Status {
        let size = chain.readable_len - OUT_HEADER_LEN as u64;
        if !self.range_ok(sector, size) || self.disk.access() == Access::ReadOnly {
            return Status::IoError;
        }
        let mut at = DiskOffset::of_sector(sector);
        let mut skip = OUT_HEADER_LEN as u64;
        let mut left = size;
        let mut buffers = chain.buffers();
        while left > 0 {
            let Some(buffer) = buffers.next_buffer(vdev.ram) else {
                break;
            };
            if buffer.device_writes {
                break;
            }
            let len = u64::from(buffer.len);
            if skip >= len {
                skip -= len;
                continue;
            }
            let n = len - skip;
            let Some(src) = vdev.ram.bytes(buffer.addr + skip, n) else {
                break;
            };
            if self.disk.write_at(at, src).is_err() {
                return Status::IoError;
            }
            at = at.after(n);
            left -= n;
            skip = 0;
        }
        // Every buffer was checked to lie in RAM when the chain was taken.
        if left > 0 {
            return Status::IoError;
        }
        self.written()
    }

    /// A write is done: through a write-through cache, once it is durable
    /// (QEMU's `BDRV_REQ_FUA`).
    fn written(&mut self) -> Status {
        match self.cache {
            Cache::WriteBack => Status::Ok,
            Cache::WriteThrough => self.sync(),
        }
    }

    /// QEMU `virtio_blk_handle_flush`.
    fn sync(&mut self) -> Status {
        match self.disk.sync() {
            Ok(()) => Status::Ok,
            Err(_) => Status::IoError,
        }
    }

    /// QEMU `virtio_blk_handle_discard_write_zeroes`, the flags checked
    /// before the sectors (5.2.6.2: an unknown flag, or `unmap` on a
    /// discard, is `VIRTIO_BLK_S_UNSUPP`). Discarding is ignored, as QEMU's
    /// drives ignore it unless given `discard=unmap`.
    fn discard_write_zeroes(&mut self, range: &[u8; RANGE_LEN], write_zeroes: bool) -> Status {
        let sector = u64::from_le_bytes([
            range[0], range[1], range[2], range[3], range[4], range[5], range[6], range[7],
        ]);
        let sectors = u32::from_le_bytes([range[8], range[9], range[10], range[11]]);
        let flags = u32::from_le_bytes([range[12], range[13], range[14], range[15]]);
        if flags & !WRITE_ZEROES_FLAG_UNMAP != 0
            || (!write_zeroes && flags & WRITE_ZEROES_FLAG_UNMAP != 0)
        {
            return Status::Unsupported;
        }
        // A le32 count of sectors is under 2⁴¹ bytes.
        let bytes = u64::from(sectors) * SECTOR_SIZE;
        if !self.range_ok(sector, bytes) {
            return Status::IoError;
        }
        if write_zeroes {
            if self.disk.access() == Access::ReadOnly
                || self
                    .disk
                    .write_zeroes(DiskOffset::of_sector(sector), bytes)
                    .is_err()
            {
                return Status::IoError;
            }
            return self.written();
        }
        Status::Ok
    }

    pub fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bool(self.cache == Cache::WriteBack)
    }

    pub fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        self.cache = if r.read_bool()? {
            Cache::WriteBack
        } else {
            Cache::WriteThrough
        };
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/virtio_block.rs"]
mod tests;
