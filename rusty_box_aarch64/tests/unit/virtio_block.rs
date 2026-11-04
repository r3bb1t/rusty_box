//! Unit tests of `src/devices/virtio/block.rs` and `src/disk.rs`: QEMU's
//! `virtio-blk-device` over a raw image (`hw/block/virtio-blk.c`,
//! `hw/block/hd-geometry.c`), driven through its registers and queue the
//! way Linux's `virtio_blk` drives it.

#[cfg(feature = "std")]
use std::format;
use std::vec;
use std::vec::Vec;

use super::Block;
use crate::bus::AccessWidth;
use crate::devices::virtio::test_driver::*;
use crate::devices::virtio::Device;
use crate::disk::Disk;

const F_SEG_MAX: u64 = 1 << 2;
const F_GEOMETRY: u64 = 1 << 4;
const F_BLK_SIZE: u64 = 1 << 6;
const F_WCE: u64 = 1 << 9;
const F_TOPOLOGY: u64 = 1 << 10;
const F_CONFIG_WCE: u64 = 1 << 11;
const F_DISCARD: u64 = 1 << 13;
const F_WRITE_ZEROES: u64 = 1 << 14;
/// Every feature of the device's own, all of which Linux's driver knows.
const BLK_FEATURES: u64 = F_SEG_MAX
    | F_GEOMETRY
    | F_BLK_SIZE
    | F_WCE
    | F_TOPOLOGY
    | F_CONFIG_WCE
    | F_DISCARD
    | F_WRITE_ZEROES;
/// What Linux accepts of what the device offers.
const LINUX_FEATURES: u64 = F_VERSION_1 | F_INDIRECT_DESC | F_EVENT_IDX | BLK_FEATURES;

const T_IN: u32 = 0;
const T_OUT: u32 = 1;
const T_SCSI_CMD: u32 = 2;
const T_FLUSH: u32 = 4;
const T_GET_ID: u32 = 8;
const T_DISCARD: u32 = 11;
const T_WRITE_ZEROES: u32 = 13;
const T_SECURE_ERASE: u32 = 14;
const T_ZONE_APPEND: u32 = 15;
const T_ZONE_REPORT: u32 = 16;
const T_ZONE_OPEN: u32 = 18;
const T_BARRIER: u32 = 0x8000_0000;

const S_OK: u8 = 0;
const S_IOERR: u8 = 1;
const S_UNSUPP: u8 = 2;
const S_ZONE_INVALID_CMD: u8 = 3;

const UNMAP: u32 = 1;

/// `sectors` sectors, each byte telling its sector and offset apart.
fn image(sectors: usize) -> Vec<u8> {
    (0..sectors * 512)
        .map(|i| ((i / 512) as u8).wrapping_mul(31) ^ (i as u8))
        .collect()
}

fn blk_over(disk: Disk, features: u64) -> Driver {
    let mut d = Driver::new(Device::Block(Block::new(disk)));
    d.start_queues(features, 1);
    d
}

fn blk(image: Vec<u8>) -> Driver {
    blk_over(Disk::memory(image), LINUX_FEATURES)
}

/// The disk's bytes, as the device holds them.
fn disk_bytes(d: &mut Driver) -> Vec<u8> {
    let bytes = d.host(|device, _| match device {
        Device::Block(b) => b.disk().memory_bytes().map(<[u8]>::to_vec),
        Device::Input(_) | Device::Gpu(_) => None,
    });
    bytes.expect("a memory disk")
}

/// A request's header: `kind`, priority 0, `sector`.
fn header(kind: u32, sector: u64) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[..4].copy_from_slice(&kind.to_le_bytes());
    h[8..].copy_from_slice(&sector.to_le_bytes());
    h
}

/// What became of a request.
#[derive(Debug)]
struct Done {
    /// The used-ring entries the notification added.
    used: Vec<(u32, u32)>,
    head: u16,
    /// The status byte the device wrote, 0xEE if it wrote none.
    status: u8,
}

/// Offer a request laid out as Linux's `virtblk_add_req` lays it out — the
/// header, the data buffers, a status byte of its own — and notify.
fn submit(d: &mut Driver, kind: u32, sector: u64, data: &[Buf]) -> Done {
    let hdr = d.alloc_with(&header(kind, sector));
    let status = d.alloc_with(&[0xEE]);
    let mut chain = vec![Buf::readable(hdr, 16)];
    chain.extend_from_slice(data);
    chain.push(Buf::writable(status, 1));
    let head = d.offer(0, &chain);
    d.notify(0);
    Done {
        used: d.used(0),
        head,
        status: d.mem(status, 1)[0],
    }
}

/// A buffer of `len` bytes the device fills, 0xEE until it does.
fn fresh(d: &mut Driver, len: u32) -> u64 {
    d.alloc_with(&vec![0xEE; len as usize])
}

/// A discard or write-zeroes range.
fn range(sector: u64, sectors: u32, flags: u32) -> [u8; 16] {
    let mut r = [0u8; 16];
    r[..8].copy_from_slice(&sector.to_le_bytes());
    r[8..12].copy_from_slice(&sectors.to_le_bytes());
    r[12..].copy_from_slice(&flags.to_le_bytes());
    r
}

fn config_byte(d: &mut Driver, off: u64) -> u8 {
    d.read_width(CONFIG + off, AccessWidth::Byte) as u8
}

fn config_u16(d: &mut Driver, off: u64) -> u16 {
    d.read_width(CONFIG + off, AccessWidth::Half) as u16
}

fn config_u32(d: &mut Driver, off: u64) -> u32 {
    d.read_width(CONFIG + off, AccessWidth::Word) as u32
}

fn config_u64(d: &mut Driver, off: u64) -> u64 {
    u64::from(config_u32(d, off)) | (u64::from(config_u32(d, off + 4)) << 32)
}

/// A sector 0 holding a DOS partition table whose first partition ends at
/// `end_head` and `end_sector`.
fn with_partition(mut image: Vec<u8>, end_head: u8, end_sector: u8) -> Vec<u8> {
    let p = 0x1BE;
    image[p + 5] = end_head;
    image[p + 6] = end_sector;
    image[p + 12..p + 16].copy_from_slice(&2048u32.to_le_bytes());
    image[510] = 0x55;
    image[511] = 0xAA;
    image
}

#[test]
fn the_device_is_qemus_virtio_blk_with_one_queue() {
    let mut d = Driver::new(Device::Block(Block::new(Disk::memory(image(8)))));
    assert_eq!(d.read(DEVICE_ID), 2);
    // QEMU's common features: indirect descriptors, event index, VERSION_1
    // and queue reset.
    assert_eq!(
        d.device_features(),
        F_INDIRECT_DESC | F_EVENT_IDX | F_VERSION_1 | (1 << 40) | BLK_FEATURES
    );
    d.write(QUEUE_SEL, 0);
    assert_eq!(d.read(QUEUE_NUM_MAX), 256);
    d.write(QUEUE_SEL, 1);
    assert_eq!(d.read(QUEUE_NUM_MAX), 0, "the device has one queue");
}

#[test]
fn the_configuration_describes_the_disk() {
    let mut d = blk(image(8192));
    assert_eq!(config_u64(&mut d, 0), 8192, "capacity in sectors");
    assert_eq!(config_u32(&mut d, 8), 0, "size_max");
    assert_eq!(config_u32(&mut d, 12), 254, "seg_max, the queue less two");
    // 16 heads of 63 sectors, as many cylinders as fit.
    assert_eq!(config_u16(&mut d, 16), 8);
    assert_eq!(config_byte(&mut d, 18), 16);
    assert_eq!(config_byte(&mut d, 19), 63);
    assert_eq!(config_u32(&mut d, 20), 512, "blk_size");
    assert_eq!(config_byte(&mut d, 24), 0, "physical_block_exp");
    assert_eq!(config_byte(&mut d, 25), 0, "alignment_offset");
    assert_eq!(config_u16(&mut d, 26), 0, "min_io_size");
    assert_eq!(config_u32(&mut d, 28), 0, "opt_io_size");
    assert_eq!(config_byte(&mut d, 32), 1, "a write-back cache");
    assert_eq!(config_u16(&mut d, 34), 1, "num_queues");
    assert_eq!(config_u32(&mut d, 36), 0x3F_FFFF, "max_discard_sectors");
    assert_eq!(config_u32(&mut d, 40), 1, "max_discard_seg");
    assert_eq!(config_u32(&mut d, 44), 1, "discard_sector_alignment");
    assert_eq!(
        config_u32(&mut d, 48),
        0x3F_FFFF,
        "max_write_zeroes_sectors"
    );
    assert_eq!(config_u32(&mut d, 52), 1, "max_write_zeroes_seg");
    assert_eq!(config_byte(&mut d, 56), 1, "write_zeroes_may_unmap");
    assert_eq!(
        config_byte(&mut d, 57),
        0xFF,
        "the configuration ends there"
    );
}

#[test]
fn a_tiny_disk_still_has_two_cylinders() {
    let mut d = blk(image(100));
    assert_eq!(config_u16(&mut d, 16), 2);
}

#[test]
fn a_partition_table_with_at_most_16_heads_is_the_geometry() {
    let mut d = blk(with_partition(image(8192), 15, 32));
    assert_eq!(config_u16(&mut d, 16), 8192 / (16 * 32));
    assert_eq!(config_byte(&mut d, 18), 16);
    assert_eq!(config_byte(&mut d, 19), 32);
}

#[test]
fn a_partition_table_with_more_heads_leaves_the_standard_geometry() {
    let mut d = blk(with_partition(image(8192), 254, 63));
    assert_eq!(config_u16(&mut d, 16), 8);
    assert_eq!(config_byte(&mut d, 18), 16);
    assert_eq!(config_byte(&mut d, 19), 63);
}

#[test]
fn a_read_fills_the_buffers_from_the_disk() {
    let disk = image(64);
    let mut d = blk(disk.clone());
    let (a, b) = (fresh(&mut d, 512), fresh(&mut d, 1024));
    let done = submit(
        &mut d,
        T_IN,
        3,
        &[Buf::writable(a, 512), Buf::writable(b, 1024)],
    );
    assert_eq!(done.status, S_OK);
    // Every writable byte counts as used, the status byte too.
    assert_eq!(done.used, [(u32::from(done.head), 512 + 1024 + 1)]);
    assert_eq!(d.mem(a, 512), &disk[3 * 512..4 * 512]);
    assert_eq!(d.mem(b, 1024), &disk[4 * 512..6 * 512]);
}

#[test]
fn a_write_reaches_the_disk() {
    let mut d = blk(image(64));
    let data: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
    let at = d.alloc_with(&data);
    let done = submit(
        &mut d,
        T_OUT,
        10,
        &[Buf::readable(at, 300), Buf::readable(at + 300, 724)],
    );
    assert_eq!(done.status, S_OK);
    assert_eq!(done.used, [(u32::from(done.head), 1)]);
    assert_eq!(&disk_bytes(&mut d)[10 * 512..12 * 512], data.as_slice());
}

#[test]
fn a_header_split_across_buffers_is_read_whole() {
    let mut d = blk(image(64));
    let mut payload = header(T_OUT, 7)[8..].to_vec();
    payload.extend_from_slice(&[0xA5; 512]);
    let first = d.alloc_with(&header(T_OUT, 7)[..8]);
    let rest = d.alloc_with(&payload);
    let status = d.alloc_with(&[0xEE]);
    d.offer(
        0,
        &[
            Buf::readable(first, 8),
            Buf::readable(rest, 8 + 512),
            Buf::writable(status, 1),
        ],
    );
    d.notify(0);
    assert_eq!(d.mem(status, 1), [S_OK]);
    assert_eq!(&disk_bytes(&mut d)[7 * 512..8 * 512], &[0xA5; 512]);
}

#[test]
fn a_barrier_flag_is_ignored() {
    let disk = image(8);
    let mut d = blk(disk.clone());
    let buf = fresh(&mut d, 512);
    let done = submit(&mut d, T_IN | T_BARRIER, 1, &[Buf::writable(buf, 512)]);
    assert_eq!(done.status, S_OK);
    assert_eq!(d.mem(buf, 512), &disk[512..1024]);
}

#[test]
fn a_request_past_the_disk_or_of_part_of_a_sector_fails() {
    let disk = image(64);
    let mut d = blk(disk.clone());
    let buf = fresh(&mut d, 1024);
    let past = submit(&mut d, T_IN, 63, &[Buf::writable(buf, 1024)]);
    assert_eq!(past.status, S_IOERR);
    assert_eq!(past.used, [(u32::from(past.head), 1025)]);
    assert_eq!(d.mem(buf, 1024), &[0xEE; 1024], "nothing was read");
    let partial = submit(&mut d, T_IN, 0, &[Buf::writable(buf, 513)]);
    assert_eq!(partial.status, S_IOERR);
    let at = d.alloc_with(&[0x11; 1024]);
    assert_eq!(
        submit(&mut d, T_OUT, 64, &[Buf::readable(at, 512)]).status,
        S_IOERR
    );
    assert_eq!(disk_bytes(&mut d), disk, "nothing was written");
    // No data at the very end is still inside the disk.
    assert_eq!(submit(&mut d, T_IN, 64, &[]).status, S_OK);
}

#[test]
fn a_flush_succeeds() {
    let mut d = blk(image(8));
    let done = submit(&mut d, T_FLUSH, 0, &[]);
    assert_eq!(done.status, S_OK);
    assert_eq!(done.used, [(u32::from(done.head), 1)]);
}

#[test]
fn get_id_gives_qemus_empty_serial() {
    let mut d = blk(image(8));
    let id = fresh(&mut d, 20);
    let done = submit(&mut d, T_GET_ID, 0, &[Buf::writable(id, 20)]);
    assert_eq!(done.status, S_OK);
    assert_eq!(done.used, [(u32::from(done.head), 21)]);
    let mut expected = [0xEE; 20];
    expected[0] = 0;
    assert_eq!(d.mem(id, 20), expected, "only the terminator is written");
}

#[test]
fn types_the_device_does_not_handle_are_unsupported() {
    let mut d = blk(image(8));
    for kind in [0x99, T_ZONE_OPEN, T_ZONE_APPEND, T_SECURE_ERASE] {
        assert_eq!(submit(&mut d, kind, 0, &[]).status, S_UNSUPP, "type {kind}");
    }
    let report = fresh(&mut d, 256);
    assert_eq!(
        submit(&mut d, T_ZONE_REPORT, 0, &[Buf::writable(report, 256)]).status,
        S_UNSUPP
    );
}

#[test]
fn write_zeroes_clears_the_range_and_may_unmap() {
    let disk = image(64);
    let mut d = blk(disk.clone());
    for (sector, flags) in [(2, 0), (20, UNMAP)] {
        let r = d.alloc_with(&range(sector, 3, flags));
        assert_eq!(
            submit(&mut d, T_WRITE_ZEROES, 0, &[Buf::readable(r, 16)]).status,
            S_OK
        );
    }
    let bytes = disk_bytes(&mut d);
    assert_eq!(&bytes[2 * 512..5 * 512], &[0; 3 * 512]);
    assert_eq!(&bytes[20 * 512..23 * 512], &[0; 3 * 512]);
    assert_eq!(&bytes[5 * 512..20 * 512], &disk[5 * 512..20 * 512]);

    let r = d.alloc_with(&range(0, 1, 2));
    assert_eq!(
        submit(&mut d, T_WRITE_ZEROES, 0, &[Buf::readable(r, 16)]).status,
        S_UNSUPP,
        "an unknown flag"
    );
    let r = d.alloc_with(&range(63, 2, 0));
    assert_eq!(
        submit(&mut d, T_WRITE_ZEROES, 0, &[Buf::readable(r, 16)]).status,
        S_IOERR,
        "past the disk"
    );
    let r = d.alloc_with(&range(0, 0x40_0000, 0));
    assert_eq!(
        submit(&mut d, T_WRITE_ZEROES, 0, &[Buf::readable(r, 16)]).status,
        S_IOERR,
        "past the limit"
    );
}

#[test]
fn a_discard_is_ignored_and_refuses_unmap() {
    let disk = image(64);
    let mut d = blk(disk.clone());
    let r = d.alloc_with(&range(4, 8, 0));
    assert_eq!(
        submit(&mut d, T_DISCARD, 0, &[Buf::readable(r, 16)]).status,
        S_OK
    );
    assert_eq!(
        disk_bytes(&mut d),
        disk,
        "a drive without discard=unmap keeps its data"
    );
    let r = d.alloc_with(&range(4, 8, UNMAP));
    assert_eq!(
        submit(&mut d, T_DISCARD, 0, &[Buf::readable(r, 16)]).status,
        S_UNSUPP
    );
    let r = d.alloc_with(&range(4, 8, 0));
    assert_eq!(
        submit(&mut d, T_DISCARD & !T_OUT, 0, &[Buf::readable(r, 16)]).status,
        S_UNSUPP,
        "without T_OUT"
    );
    let two = d.alloc_with(&[range(4, 8, 0), range(20, 8, 0)].concat());
    assert_eq!(
        submit(&mut d, T_DISCARD, 0, &[Buf::readable(two, 32)]).status,
        S_UNSUPP,
        "two ranges"
    );
}

/// Check the device said it needs a reset and answered nothing, then that
/// a reset brings it back.
fn assert_broken(d: &mut Driver) {
    assert_eq!(d.read(STATUS) & NEEDS_RESET, NEEDS_RESET);
    assert!(d.used(0).is_empty());
    d.start_queues(LINUX_FEATURES, 1);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, 0);
    assert_eq!(submit(d, T_FLUSH, 0, &[]).status, S_OK);
}

#[test]
fn a_request_without_a_status_buffer_breaks_the_device() {
    let mut d = blk(image(8));
    let hdr = d.alloc_with(&header(T_FLUSH, 0));
    d.offer(0, &[Buf::readable(hdr, 16)]);
    d.notify(0);
    assert_broken(&mut d);
}

#[test]
fn a_header_shorter_than_sixteen_bytes_breaks_the_device() {
    let mut d = blk(image(8));
    let hdr = d.alloc_with(&header(T_FLUSH, 0));
    let status = d.alloc_with(&[0xEE]);
    d.offer(0, &[Buf::readable(hdr, 12), Buf::writable(status, 1)]);
    d.notify(0);
    assert_broken(&mut d);
}

#[test]
fn a_short_discard_range_breaks_the_device() {
    let mut d = blk(image(8));
    let r = d.alloc_with(&range(0, 1, 0));
    let hdr = d.alloc_with(&header(T_DISCARD, 0));
    let status = d.alloc_with(&[0xEE]);
    d.offer(
        0,
        &[
            Buf::readable(hdr, 16),
            Buf::readable(r, 8),
            Buf::writable(status, 1),
        ],
    );
    d.notify(0);
    assert_eq!(d.mem(status, 1), [0xEE], "no status");
    assert_broken(&mut d);
}

#[test]
fn a_zone_report_without_room_for_a_zone_breaks_the_device_and_answers_it() {
    let mut d = blk(image(8));
    let report = fresh(&mut d, 64);
    let done = submit(&mut d, T_ZONE_REPORT, 0, &[Buf::writable(report, 64)]);
    assert_eq!(done.status, S_ZONE_INVALID_CMD, "the status is written");
    assert_broken(&mut d);
}

#[test]
fn the_legacy_scsi_command_is_refused_through_its_header() {
    let mut d = blk(image(8));
    let cdb = d.alloc_with(&[0x12; 6]);
    let (sense, scsi) = (fresh(&mut d, 96), fresh(&mut d, 16));
    let done = submit(
        &mut d,
        T_SCSI_CMD,
        0,
        &[
            Buf::readable(cdb, 6),
            Buf::writable(sense, 96),
            Buf::writable(scsi, 16),
        ],
    );
    assert_eq!(done.status, S_UNSUPP);
    assert_eq!(d.mem(scsi, 4), 255u32.to_le_bytes(), "errors");
    assert_eq!(done.used, [(u32::from(done.head), 96 + 16 + 1)]);

    let scsi = fresh(&mut d, 16);
    assert_eq!(
        submit(
            &mut d,
            T_SCSI_CMD,
            0,
            &[Buf::readable(cdb, 6), Buf::writable(scsi, 16)]
        )
        .status,
        S_IOERR,
        "no sense buffer"
    );
    let (sense, scsi) = (fresh(&mut d, 96), fresh(&mut d, 12));
    let done = submit(
        &mut d,
        T_SCSI_CMD,
        0,
        &[
            Buf::readable(cdb, 6),
            Buf::writable(sense, 96),
            Buf::writable(scsi, 12),
        ],
    );
    assert_eq!(done.status, S_IOERR, "a SCSI header of the wrong size");
}

#[test]
fn the_driver_chooses_write_through_through_wce() {
    let mut d = blk(image(8));
    d.write_width(CONFIG + 32, AccessWidth::Byte, 0);
    assert_eq!(config_byte(&mut d, 32), 0);
    // Writes still land.
    let at = d.alloc_with(&[0x77; 512]);
    assert_eq!(
        submit(&mut d, T_OUT, 1, &[Buf::readable(at, 512)]).status,
        S_OK
    );
    assert_eq!(&disk_bytes(&mut d)[512..1024], &[0x77; 512]);
    // A reset brings the write-back cache back.
    d.write(STATUS, 0);
    assert_eq!(config_byte(&mut d, 32), 1);
}

#[test]
fn without_config_wce_the_accepted_features_choose_the_cache() {
    let mut d = blk_over(
        Disk::memory(image(8)),
        LINUX_FEATURES & !(F_CONFIG_WCE | F_WCE),
    );
    assert_eq!(config_byte(&mut d, 32), 0, "write-through");
    let mut d = blk_over(Disk::memory(image(8)), LINUX_FEATURES & !F_CONFIG_WCE);
    assert_eq!(config_byte(&mut d, 32), 1, "write-back");
}

#[test]
fn the_device_works_through_every_request_before_it_publishes_where_it_got_to() {
    let mut d = blk(image(8));
    for _ in 0..3 {
        let hdr = d.alloc_with(&header(T_FLUSH, 0));
        let status = d.alloc_with(&[0xEE]);
        d.offer(0, &[Buf::readable(hdr, 16), Buf::writable(status, 1)]);
    }
    d.notify(0);
    assert_eq!(d.used(0).len(), 3);
    assert_eq!(
        d.avail_event(0),
        d.avail_idx(0),
        "the driver is asked to notify the next request"
    );

    let mut d = blk_over(Disk::memory(image(8)), LINUX_FEATURES & !F_EVENT_IDX);
    assert_eq!(submit(&mut d, T_FLUSH, 0, &[]).status, S_OK);
    assert_eq!(d.used_flags(0), 0, "notifications are on again");
}

#[test]
fn a_read_only_disk_says_so_and_refuses_every_write() {
    let disk = image(64);
    let mut d = blk_over(Disk::memory(disk.clone()).read_only(), LINUX_FEATURES);
    assert_eq!(d.device_features() & (1 << 5), 1 << 5, "VIRTIO_BLK_F_RO");
    let at = d.alloc_with(&[0x11; 512]);
    assert_eq!(
        submit(&mut d, T_OUT, 0, &[Buf::readable(at, 512)]).status,
        S_IOERR
    );
    assert_eq!(
        submit(&mut d, T_OUT, 0, &[]).status,
        S_IOERR,
        "even of no bytes"
    );
    let r = d.alloc_with(&range(0, 1, 0));
    assert_eq!(
        submit(&mut d, T_WRITE_ZEROES, 0, &[Buf::readable(r, 16)]).status,
        S_IOERR
    );
    assert_eq!(
        submit(&mut d, T_DISCARD, 0, &[Buf::readable(r, 16)]).status,
        S_OK,
        "ignored"
    );
    assert_eq!(
        submit(&mut d, T_FLUSH, 0, &[]).status,
        S_OK,
        "nothing to flush"
    );
    let buf = fresh(&mut d, 512);
    assert_eq!(
        submit(&mut d, T_IN, 2, &[Buf::writable(buf, 512)]).status,
        S_OK
    );
    assert_eq!(d.mem(buf, 512), &disk[1024..1536]);
    assert_eq!(disk_bytes(&mut d), disk);
}

#[test]
fn a_request_the_devices_own_read_made_available_waits_for_the_next_notification() {
    // Sector 0 holds an available ring with one more request in it: index 2,
    // its second entry descriptor 3.
    let mut disk = image(8);
    disk[..512].fill(0);
    disk[2..4].copy_from_slice(&2u16.to_le_bytes());
    disk[6..8].copy_from_slice(&3u16.to_le_bytes());
    let mut d = blk(disk);
    // A flush at descriptors 3 and 4, not yet made available.
    let hdr = d.alloc_with(&header(T_FLUSH, 0));
    let status = d.alloc_with(&[0xEE]);
    d.put_desc(0, 3, hdr, 16, DESC_NEXT, 4);
    d.put_desc(0, 4, status, 1, DESC_WRITE, 0);
    // A read of sector 0 over the available ring, at descriptors 0 to 2.
    let ring = d.avail_ring(0);
    let read = submit(&mut d, T_IN, 0, &[Buf::writable(ring, 512)]);
    assert_eq!(read.status, S_OK);
    assert_eq!(
        read.used.len(),
        1,
        "only the request the driver notified about"
    );
    assert_eq!(d.mem(status, 1), [0xEE]);
    d.notify(0);
    assert_eq!(d.used(0), [(3, 1)]);
    assert_eq!(d.mem(status, 1), [S_OK]);
}

#[test]
fn a_snapshot_keeps_the_cache_mode() {
    let mut d = blk(image(8));
    d.write_width(CONFIG + 32, AccessWidth::Byte, 0);
    let saved = d.save();
    let mut resumed = d.restored(Device::Block(Block::new(Disk::memory(image(8)))), &saved);
    assert_eq!(resumed.save(), saved);
    assert_eq!(config_byte(&mut resumed, 32), 0);
}

#[test]
fn an_image_of_part_of_a_sector_reads_as_zeros_to_its_end() {
    let mut d = blk(vec![0xAB; 700]);
    assert_eq!(config_u64(&mut d, 0), 2);
    let buf = fresh(&mut d, 512);
    assert_eq!(
        submit(&mut d, T_IN, 1, &[Buf::writable(buf, 512)]).status,
        S_OK
    );
    assert_eq!(&d.mem(buf, 512)[..188], &[0xAB; 188]);
    assert_eq!(&d.mem(buf, 512)[188..], &[0; 324]);
}

/// A file in the host's temporary directory, removed when dropped.
#[cfg(feature = "std")]
struct TempFile(std::path::PathBuf);

#[cfg(feature = "std")]
impl TempFile {
    fn new(name: &str, contents: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rusty_box_virtio_blk_{}_{name}",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("the temporary file is written");
        Self(path)
    }

    fn open(&self) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.0)
            .expect("the temporary file opens")
    }

    fn contents(&self) -> Vec<u8> {
        std::fs::read(&self.0).expect("the temporary file reads")
    }
}

#[cfg(feature = "std")]
impl Drop for TempFile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            std::eprintln!("leaving {}: {e}", self.0.display());
        }
    }
}

#[cfg(feature = "std")]
#[test]
fn a_file_disk_is_read_and_written_in_place() {
    let mut contents = image(4);
    contents.truncate(1800);
    let file = TempFile::new("in_place", &contents);
    let disk = Disk::file(file.open()).expect("the file's length reads");
    assert_eq!(disk.sectors(), 4, "rounded up to whole sectors");
    let mut d = blk_over(disk, LINUX_FEATURES);

    let buf = fresh(&mut d, 512);
    assert_eq!(
        submit(&mut d, T_IN, 3, &[Buf::writable(buf, 512)]).status,
        S_OK
    );
    assert_eq!(&d.mem(buf, 512)[..264], &contents[1536..1800]);
    assert_eq!(
        &d.mem(buf, 512)[264..],
        &[0; 248],
        "past the end of the file"
    );

    let at = d.alloc_with(&[0x3C; 1024]);
    assert_eq!(
        submit(&mut d, T_OUT, 2, &[Buf::readable(at, 1024)]).status,
        S_OK
    );
    assert_eq!(submit(&mut d, T_FLUSH, 0, &[]).status, S_OK);
    let after = file.contents();
    assert_eq!(after.len(), 2048, "the write extended the file");
    assert_eq!(&after[..1024], &contents[..1024]);
    assert_eq!(&after[1024..], &[0x3C; 1024]);

    let r = d.alloc_with(&range(0, 1, 0));
    assert_eq!(
        submit(&mut d, T_WRITE_ZEROES, 0, &[Buf::readable(r, 16)]).status,
        S_OK
    );
    assert_eq!(&file.contents()[..512], &[0; 512]);
    let flushed = d.host(|device, _| match device {
        Device::Block(b) => b.disk_mut().flush().is_ok() && b.disk_mut().take_io_error().is_none(),
        Device::Input(_) | Device::Gpu(_) => false,
    });
    assert!(flushed);
}

#[cfg(feature = "std")]
#[test]
fn a_host_error_is_an_io_error_the_host_can_take() {
    let file = TempFile::new("read_only", &image(4));
    let read_only = std::fs::File::open(&file.0).expect("the temporary file opens");
    let mut d = blk_over(
        Disk::file(read_only).expect("the file's length reads"),
        LINUX_FEATURES,
    );
    let at = d.alloc_with(&[0x3C; 512]);
    assert_eq!(
        submit(&mut d, T_OUT, 0, &[Buf::readable(at, 512)]).status,
        S_IOERR
    );
    assert_eq!(file.contents(), image(4));
    let error = d.host(|device, _| match device {
        Device::Block(b) => b.disk_mut().take_io_error().map(|e| e.kind()),
        Device::Input(_) | Device::Gpu(_) => None,
    });
    assert!(error.is_some(), "the host is told");
    let again = d.host(|device, _| match device {
        Device::Block(b) => b.disk_mut().take_io_error().is_none(),
        Device::Input(_) | Device::Gpu(_) => false,
    });
    assert!(again, "once");
}
