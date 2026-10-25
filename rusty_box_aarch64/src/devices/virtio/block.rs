//! virtio-blk: QEMU's `virtio-blk-device`.
//!
//! Provenance (R7): the virtio 1.2 specification, section 5.2, as QEMU
//! implements it in `hw/block/virtio-blk.c` for a raw image given with every
//! default (`-drive file=…,format=raw,if=none,id=hd0 -device
//! virtio-blk-device,drive=hd0`: one queue of 256 entries, a write-back
//! cache, discard requests ignored, no serial number, not zoned):
//!
//! - `virtio_blk_get_features`, `virtio_blk_device_realize` and
//!   `virtio_blk_cfg_size_params`: the features offered (`VIRTIO_BLK_F_RO`
//!   too over a drive given `readonly=on`, whose writes QEMU's block layer
//!   refuses), and a configuration that ends after `write_zeroes_may_unmap`;
//! - `virtio_blk_update_config`: the capacity, `seg_max` two less than the
//!   queue, the geometry `hd_geometry_guess` (`hw/block/hd-geometry.c`)
//!   chose when the device was made, 512-byte logical and physical blocks
//!   (`blkconf_blocksizes`, which finds no block sizes to probe in a regular
//!   file), the cache mode, one queue, and the discard and write-zeroes
//!   limits;
//! - `virtio_blk_set_config`, `virtio_blk_set_status` and `virtio_blk_reset`:
//!   the driver chooses the cache mode through `wce`, or, when it did not
//!   accept `VIRTIO_BLK_F_CONFIG_WCE`, through whether it accepted
//!   `VIRTIO_BLK_F_WCE`; a reset brings back the write-back cache;
//! - `virtio_blk_handle_vq` and `virtio_blk_handle_request`: the headers
//!   every request must have (a `virtio_error` when it has not), reads and
//!   writes checked by `virtio_blk_sect_range_ok`, a write-through cache
//!   syncing each write, flushes, `VIRTIO_BLK_T_GET_ID`, discard and
//!   write-zeroes (`virtio_blk_handle_discard_write_zeroes`), the legacy SCSI
//!   command (`virtio_blk_handle_scsi`), zone commands to a device that is
//!   not zoned, and `VIRTIO_BLK_S_UNSUPP` for any other type;
//! - `virtio_blk_req_complete`: the status in the last byte of the last
//!   device-writable buffer, and every device-writable byte counted as used.
//!
//! Where it differs from QEMU:
//!
//! - A request completes before the driver's notification returns. QEMU
//!   gives reads, writes and flushes to its block layer, which completes
//!   them afterwards, in an order and at a time the host decides; the guest
//!   sees the same results, sooner and always at the same instruction. A
//!   notification takes the requests available when it was made, the ones
//!   QEMU takes (see [`Block::handle_output`]).
//! - A host error is reported to the guest as `VIRTIO_BLK_S_IOERR`. That is
//!   QEMU's default for every failed read and for every failed write but
//!   one: a write that finds the host's disk full stops QEMU's machine
//!   (`werror=enospc`) until the user resumes it and the write is retried.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::queue::{Chain, QueueFault};
use super::{Core, Vdev, STATUS_DRIVER_OK};
use crate::disk::{Access, Disk, DiskOffset, SECTOR_SIZE};

/// The device's one queue.
pub const QUEUE: usize = 0;
/// QEMU's `queue-size` default.
pub const QUEUE_SIZE: u16 = 256;

const F_SEG_MAX: u64 = 1 << 2;
const F_GEOMETRY: u64 = 1 << 4;
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
pub const FEATURES: u64 =F_SEG_MAX | F_GEOMETRY | F_BLK_SIZE | F_WCE | F_TOPOLOGY | F_CONFIG_WCE | F_DISCARD | F_WRITE_ZEROES;

/// `struct virtio_blk_config` up to the end of `write_zeroes_may_unmap`.
pub const CONFIG_LEN: usize = 57;

/// `logical_block_size` (and `physical_block_size`).
const BLOCK_SIZE: u64 = 512;
/// `BDRV_REQUEST_MAX_SECTORS`: the most sectors one request moves, and the
/// default discard and write-zeroes limits.
const REQUEST_MAX_SECTORS: u64 = (i32::MAX as u64) >> 9;
/// `seg_max` with `seg-max-adjust`.
const SEG_MAX: u32 = QUEUE_SIZE as u32 - 2;
/// `VIRTIO_BLK_ID_BYTES`.
const ID_BYTES: u64 = 20;
/// QEMU's `serial` property, unset.
const SERIAL: &[u8] = b"";

/// `struct virtio_blk_outhdr`: type, priority, sector.
const OUT_HEADER_LEN: usize = 16;
/// `struct virtio_blk_discard_write_zeroes`: sector, sectors, flags.
const RANGE_LEN: usize = 16;
/// `struct virtio_scsi_inhdr`.
const SCSI_INHDR_LEN: u32 = 16;
/// The status byte, a `struct virtio_blk_zone_report` and one
/// `struct virtio_blk_zone_descriptor`.
const ZONE_REPORT_MIN: u64 = 1 + 64 + 64;

const T_OUT: u32 = 1;
const T_BARRIER: u32 = 0x8000_0000;
const T_IN: u32 = 0;
const T_SCSI_CMD: u32 = 2;
const T_FLUSH: u32 = 4;
const T_GET_ID: u32 = 8;
const T_DISCARD: u32 = 11;
const T_WRITE_ZEROES: u32 = 13;
const T_ZONE_APPEND: u32 = 15;
const T_ZONE_REPORT: u32 = 16;
const T_ZONE_OPEN: u32 = 18;
const T_ZONE_CLOSE: u32 = 20;
const T_ZONE_FINISH: u32 = 22;
const T_ZONE_RESET: u32 = 24;
const T_ZONE_RESET_ALL: u32 = 26;

/// The codes the request types share once `VIRTIO_BLK_T_OUT` is masked
/// off: QEMU tells the direction from that bit.
const ZONE_APPEND_CODE: u32 = T_ZONE_APPEND & !T_OUT;
const DISCARD_CODE: u32 = T_DISCARD & !T_OUT;
const WRITE_ZEROES_CODE: u32 = T_WRITE_ZEROES & !T_OUT;

const WRITE_ZEROES_FLAG_UNMAP: u32 = 1;

/// A request's outcome, written to its status byte.
///
/// Closed (R5): the statuses a device that is not zoned gives, and the one
/// a zone report too short to answer gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok = 0,
    IoError = 1,
    Unsupported = 2,
    ZoneInvalidCommand = 3,
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

/// The legacy geometry the configuration reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Geometry {
    cylinders: u32,
    heads: u32,
    sectors: u32,
}

impl Geometry {
    /// QEMU `hd_geometry_guess` with no geometry given and none to probe: the
    /// one a DOS partition table implies, if it implies one with no more than
    /// 16 heads, else the standard one for the disk's size.
    fn guess(disk: &mut Disk) -> Self {
        let sectors = disk.sectors();
        match Self::from_partition_table(disk, sectors) {
            Some(lchs) if lchs.heads <= 16 => lchs,
            Some(_) | None => Self::for_size(sectors),
        }
    }

    /// QEMU `guess_disk_lchs`: the first partition with sectors and an end
    /// head names the heads and sectors per track, if the cylinders they
    /// leave are between 1 and 16383.
    fn from_partition_table(disk: &mut Disk, total: u64) -> Option<Self> {
        let mut mbr = [0u8; SECTOR_SIZE as usize];
        disk.read_at(DiskOffset::of_sector(0), &mut mbr).ok()?;
        if mbr[510] != 0x55 || mbr[511] != 0xAA {
            return None;
        }
        mbr[0x1BE..0x1FE].chunks_exact(16).find_map(|p| {
            let (end_head, end_sector) = (p[5], p[6]);
            let nr_sects = u32::from_le_bytes([p[12], p[13], p[14], p[15]]);
            if nr_sects == 0 || end_head == 0 {
                return None;
            }
            let heads = u32::from(end_head) + 1;
            let sectors = u32::from(end_sector & 63);
            if sectors == 0 {
                return None;
            }
            // QEMU keeps the quotient in an int.
            let cylinders = (total / u64::from(heads * sectors)) as i32;
            (1..=16383).contains(&cylinders).then_some(Self {
                cylinders: cylinders as u32,
                heads,
                sectors,
            })
        })
    }

    /// QEMU `guess_chs_for_size`: 16 heads of 63 sectors, the cylinders
    /// between 2 and 16383.
    fn for_size(total: u64) -> Self {
        // QEMU keeps the quotient in an int.
        let cylinders = (total / (16 * 63)) as i32;
        Self {
            cylinders: cylinders.clamp(2, 16383) as u32,
            heads: 16,
            sectors: 63,
        }
    }
}

/// QEMU's `VirtIOBlock` over its disk.
pub struct Block {
    disk: Disk,
    geometry: Geometry,
    cache: Cache,
}

impl Block {
    /// A device over `disk`, its geometry guessed from it as QEMU does when
    /// it realizes the device.
    pub fn new(disk: Disk) -> Self {
        let mut disk = disk;
        let geometry = Geometry::guess(&mut disk);
        Self {
            disk,
            geometry,
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
        // size_max stays 0.
        c[12..16].copy_from_slice(&SEG_MAX.to_le_bytes());
        c[16..18].copy_from_slice(&(self.geometry.cylinders as u16).to_le_bytes());
        c[18] = self.geometry.heads as u8;
        c[19] = self.geometry.sectors as u8;
        c[20..24].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes());
        // physical_block_exp, alignment_offset, min_io_size and opt_io_size
        // stay 0: the physical block is the logical one.
        c[32] = u8::from(self.cache == Cache::WriteBack);
        c[34..36].copy_from_slice(&1u16.to_le_bytes());
        c[36..40].copy_from_slice(&(REQUEST_MAX_SECTORS as u32).to_le_bytes());
        c[40..44].copy_from_slice(&1u32.to_le_bytes());
        // discard_sector_alignment: the discard granularity, the block size,
        // in sectors.
        c[44..48].copy_from_slice(&((BLOCK_SIZE / SECTOR_SIZE) as u32).to_le_bytes());
        c[48..52].copy_from_slice(&(REQUEST_MAX_SECTORS as u32).to_le_bytes());
        c[52..56].copy_from_slice(&1u32.to_le_bytes());
        c[56] = 1;
        config.copy_from_slice(&c[..config.len()]);
    }

    /// QEMU `virtio_blk_set_config`: `wce` chooses the cache mode.
    pub fn set_config(&mut self, config: &[u8]) {
        self.cache = if config[32] != 0 {
            Cache::WriteBack
        } else {
            Cache::WriteThrough
        };
    }

    /// QEMU `virtio_blk_set_status`.
    pub fn set_status(&mut self, status: u8, core: &Core) {
        if status & STATUS_DRIVER_OK == 0 {
            return;
        }
        if !core.has_feature(F_CONFIG_WCE) {
            self.cache = if core.has_feature(F_WCE) {
                Cache::WriteBack
            } else {
                Cache::WriteThrough
            };
        }
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

    /// QEMU `virtio_blk_handle_output` and `virtio_blk_handle_vq`.
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
            header[8], header[9], header[10], header[11], header[12], header[13], header[14], header[15],
        ]);
        let status = match kind & !(T_OUT | T_BARRIER) {
            T_IN if kind & T_OUT != 0 => self.write(vdev, chain, sector),
            T_IN => self.read(vdev, chain, sector),
            T_FLUSH => self.sync(),
            T_ZONE_REPORT => {
                if chain.writable_len < ZONE_REPORT_MIN {
                    vdev.error(QueueFault::BlockZoneReportShort);
                    Status::ZoneInvalidCommand
                } else {
                    Status::Unsupported
                }
            }
            T_ZONE_OPEN | T_ZONE_CLOSE | T_ZONE_FINISH | T_ZONE_RESET | T_ZONE_RESET_ALL => Status::Unsupported,
            T_SCSI_CMD => Self::scsi(vdev, chain),
            T_GET_ID => {
                let room = (chain.writable_len - 1).min(ID_BYTES);
                let len = (SERIAL.len() as u64 + 1).min(room) as usize;
                let mut id = [0u8; ID_BYTES as usize];
                id[..SERIAL.len()].copy_from_slice(SERIAL);
                // `len` leaves the status byte free, so the whole identifier
                // fits the chain that was just checked.
                if vdev.write(chain, 0, &id[..len]) == len {
                    Status::Ok
                } else {
                    Status::IoError
                }
            }
            // VIRTIO_BLK_T_SECURE_ERASE shares this code.
            ZONE_APPEND_CODE => Status::Unsupported,
            DISCARD_CODE | WRITE_ZEROES_CODE => {
                let range_room = chain.readable_len - OUT_HEADER_LEN as u64;
                if kind & T_OUT == 0 || range_room > RANGE_LEN as u64 {
                    Status::Unsupported
                } else {
                    let mut range = [0u8; RANGE_LEN];
                    if vdev.read(chain, OUT_HEADER_LEN, &mut range) != RANGE_LEN {
                        return Err(QueueFault::BlockRangeShort);
                    }
                    self.discard_write_zeroes(&range, kind & !T_BARRIER == T_WRITE_ZEROES)
                }
            }
            _ => Status::Unsupported,
        };
        Self::complete(vdev, chain, status_at, status);
        Ok(())
    }

    /// The last byte of the chain's last device-writable buffer, the
    /// status byte's place (`struct virtio_blk_inhdr`); `None` for a chain
    /// with no device-writable buffer.
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

    /// QEMU `virtio_blk_req_complete`: the status, then the chain back with
    /// every device-writable byte counted.
    fn complete(vdev: &mut Vdev<'_, '_>, chain: &Chain, status_at: u64, status: Status) {
        vdev.ram.store(status_at, &[status as u8]);
        vdev.push(QUEUE, chain, chain.writable_len as u32);
        vdev.notify(QUEUE);
    }

    /// QEMU `virtio_blk_sect_range_ok`: `size` bytes from `sector` are
    /// whole blocks, a request's worth at most, inside the disk.
    fn range_ok(&self, sector: u64, size: u64) -> bool {
        let count = size / SECTOR_SIZE;
        let total = self.disk.sectors();
        count <= REQUEST_MAX_SECTORS && size.is_multiple_of(BLOCK_SIZE) && sector <= total && count <= total - sector
    }

    /// A read: the disk into the device-writable buffers, all but the
    /// status byte.
    fn read(&mut self, vdev: &mut Vdev<'_, '_>, chain: &Chain, sector: u64) -> Status {
        let size = chain.writable_len - 1;
        if !self.range_ok(sector, size) {
            return Status::IoError;
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
                return Status::IoError;
            }
            at = at.after(n);
            left -= n;
        }
        // Every buffer was checked to lie in RAM when the chain was taken.
        if left > 0 {
            return Status::IoError;
        }
        Status::Ok
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

    /// QEMU `virtio_blk_handle_discard_write_zeroes`. Discarding is ignored,
    /// as QEMU's drives ignore it unless given `discard=unmap`.
    fn discard_write_zeroes(&mut self, range: &[u8; RANGE_LEN], write_zeroes: bool) -> Status {
        let sector = u64::from_le_bytes([range[0], range[1], range[2], range[3], range[4], range[5], range[6], range[7]]);
        let sectors = u32::from_le_bytes([range[8], range[9], range[10], range[11]]);
        let flags = u32::from_le_bytes([range[12], range[13], range[14], range[15]]);
        if u64::from(sectors) > REQUEST_MAX_SECTORS {
            return Status::IoError;
        }
        let bytes = u64::from(sectors) * SECTOR_SIZE;
        if !self.range_ok(sector, bytes) {
            return Status::IoError;
        }
        if flags & !WRITE_ZEROES_FLAG_UNMAP != 0 {
            return Status::Unsupported;
        }
        if write_zeroes {
            if self.disk.access() == Access::ReadOnly || self.disk.write_zeroes(DiskOffset::of_sector(sector), bytes).is_err() {
                return Status::IoError;
            }
            return self.written();
        }
        if flags & WRITE_ZEROES_FLAG_UNMAP != 0 {
            return Status::Unsupported;
        }
        Status::Ok
    }

    /// QEMU `virtio_blk_handle_scsi`: refused, with the SCSI header's
    /// `errors` set so the guest's ioctl fails, when the request has one in
    /// the device-writable buffer before the status.
    fn scsi(vdev: &mut Vdev<'_, '_>, chain: &Chain) -> Status {
        if chain.readable < 2 || chain.writable < 3 {
            return Status::IoError;
        }
        let mut buffers = chain.buffers();
        let mut writable = 0;
        while let Some(buffer) = buffers.next_buffer(vdev.ram) {
            if !buffer.device_writes {
                continue;
            }
            if writable == chain.writable - 2 {
                if buffer.len != SCSI_INHDR_LEN {
                    return Status::IoError;
                }
                vdev.ram.store(buffer.addr, &255u32.to_le_bytes());
                return Status::Unsupported;
            }
            writable += 1;
        }
        Status::IoError
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
