//! Unit tests of `src/devices/virtio/gpu.rs`: the 2D virtio-gpu of virtio
//! 1.2 section 5.7, as a driver commands it and as the host sees its
//! picture, with QEMU's behaviour where the specification leaves it to the
//! device (`hw/display/virtio-gpu.c`, `virtio-gpu-base.c`) and the board's
//! own limits and error codes. The EDID it answers with has its own tests
//! (`virtio_edid.rs`).

use std::vec;
use std::vec::Vec;

use rusty_box_devices::display::sink::{Dimensions, DisplaySink, Redraw, Refreshed, Rgb, TilePos};
use rusty_box_devices::display::vga::VgaTextModeInfo;
use rusty_box_devices::display::Resolution;

use super::Gpu;
use crate::board::RAM_BASE;
use crate::bus::AccessWidth;
use crate::devices::virtio::edid;
use crate::devices::virtio::test_driver::*;
use crate::devices::virtio::Device;
use crate::identity::{BoardIdentity, DisplayIdentity};

/// The display a board with the default identity plugs in, for a guest with
/// the test driver's RAM.
fn display(preferred: Resolution) -> Gpu {
    Gpu::new(preferred, boards_display(), RAM_SIZE)
}

/// What the default identity's EDID says of its display.
fn boards_display() -> DisplayIdentity {
    BoardIdentity::RUSTY_BOX.display()
}

/// What Linux accepts of what the device offers: the ring features and
/// `VIRTIO_GPU_F_EDID`.
const LINUX_FEATURES: u64 = F_VERSION_1 | F_INDIRECT_DESC | F_EVENT_IDX | (1 << 1);

const GET_DISPLAY_INFO: u32 = 0x0100;
const RESOURCE_CREATE_2D: u32 = 0x0101;
const RESOURCE_UNREF: u32 = 0x0102;
const SET_SCANOUT: u32 = 0x0103;
const RESOURCE_FLUSH: u32 = 0x0104;
const TRANSFER_TO_HOST_2D: u32 = 0x0105;
const RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const RESOURCE_DETACH_BACKING: u32 = 0x0107;
const GET_CAPSET_INFO: u32 = 0x0108;
const GET_EDID: u32 = 0x010A;
const RESOURCE_ASSIGN_UUID: u32 = 0x010B;
const RESOURCE_CREATE_BLOB: u32 = 0x010C;
const SET_SCANOUT_BLOB: u32 = 0x010D;
const CTX_CREATE: u32 = 0x0200;
const UPDATE_CURSOR: u32 = 0x0300;
const MOVE_CURSOR: u32 = 0x0301;

const OK_NODATA: u32 = 0x1100;
const OK_DISPLAY_INFO: u32 = 0x1101;
const OK_EDID: u32 = 0x1104;
const ERR_UNSPEC: u32 = 0x1200;
const ERR_OUT_OF_MEMORY: u32 = 0x1201;
const ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
const ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
const ERR_INVALID_PARAMETER: u32 = 0x1205;

const B8G8R8X8: u32 = 2;
const B8G8R8A8: u32 = 1;

fn gpu(width: u32, height: u32) -> Driver {
    let mut d = Driver::new(Device::Gpu(display(Resolution::new(width, height))));
    d.start(LINUX_FEATURES);
    d
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A command: the header of `kind` with no flags, then `args`.
fn cmd(kind: u32, args: &[u32]) -> Vec<u8> {
    let mut b = vec![0u8; 24];
    b[..4].copy_from_slice(&kind.to_le_bytes());
    for a in args {
        b.extend_from_slice(&a.to_le_bytes());
    }
    b
}

/// Send `request` on the control queue with room for `room` bytes of
/// response; the response the device wrote.
fn command(d: &mut Driver, request: &[u8], room: u32) -> Vec<u8> {
    let req = d.alloc_with(request);
    let resp = d.alloc(u64::from(room));
    d.offer(
        0,
        &[
            Buf::readable(req, request.len() as u32),
            Buf::writable(resp, room),
        ],
    );
    d.notify(0);
    let used = d.used(0);
    assert_eq!(used.len(), 1, "one command, one answer");
    d.mem(resp, used[0].1 as usize).to_vec()
}

/// The response type of a command that answers with a bare header.
fn status(d: &mut Driver, request: &[u8]) -> u32 {
    let resp = command(d, request, 24);
    assert_eq!(resp.len(), 24);
    le32(&resp, 0)
}

/// Attach one guest buffer holding `pixels` as resource `id`'s backing.
fn attach(d: &mut Driver, id: u32, pixels: &[u8]) -> u64 {
    let at = d.alloc_with(pixels);
    let mut req = cmd(RESOURCE_ATTACH_BACKING, &[id, 1]);
    req.extend_from_slice(&at.to_le_bytes());
    req.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    req.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(status(d, &req), OK_NODATA);
    at
}

/// A frame as the host received it.
#[derive(Default)]
struct Screen {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    resized: usize,
}

impl Screen {
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        ]
    }
}

impl DisplaySink for Screen {
    fn dimension_update(&mut self, dims: Dimensions) {
        self.width = dims.width;
        self.height = dims.height;
        self.rgba = vec![0; (dims.width * dims.height * 4) as usize];
        self.resized += 1;
    }

    fn text_update(
        &mut self,
        _previous: &[u8],
        _current: &[u8],
        _cursor: Option<rusty_box_devices::display::sink::CursorPos>,
        _info: &VgaTextModeInfo,
    ) {
        panic!("a virtio-gpu has no text mode");
    }

    fn graphics_tile_update(&mut self, rgba: &[u8], at: TilePos) {
        for row in 0..at.height {
            let src = (row * at.width * 4) as usize;
            let dst = (((at.y + row) * self.width + at.x) * 4) as usize;
            let len = (at.width * 4) as usize;
            self.rgba[dst..dst + len].copy_from_slice(&rgba[src..src + len]);
        }
    }

    fn palette_change(&mut self, _index: u8, _colour: Rgb) -> Redraw {
        panic!("a virtio-gpu has no palette");
    }

    fn set_text_charmap(&mut self, _map: usize, _glyphs: &[u8]) {
        panic!("a virtio-gpu has no character generator");
    }

    fn clear_screen(&mut self) {
        self.rgba.fill(0);
    }

    fn flush(&mut self) {}
}

/// What the host sees now.
fn refresh(d: &mut Driver, screen: &mut Screen) -> Refreshed {
    d.host(|device, _| match device {
        Device::Gpu(g) => g.refresh(screen),
        Device::Input(_) | Device::Block(_) => panic!("not a GPU"),
    })
}

/// The test picture: pixel (x, y) of a `width`-wide B8G8R8X8 image.
fn bgrx(x: u32, y: u32) -> [u8; 4] {
    [x as u8, y as u8, (x * 3 + y) as u8, 0x5A]
}

fn picture(width: u32, height: u32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| bgrx(x, y)))
        .collect()
}

/// A `width` by `height` B8G8R8X8 resource 1 with the test picture as its
/// backing, shown whole on the scanout.
fn show_picture(d: &mut Driver, width: u32, height: u32) {
    assert_eq!(
        status(d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, width, height])),
        OK_NODATA
    );
    attach(d, 1, &picture(width, height));
    assert_eq!(
        status(d, &cmd(SET_SCANOUT, &[0, 0, width, height, 0, 1])),
        OK_NODATA
    );
    assert_eq!(
        status(
            d,
            &cmd(TRANSFER_TO_HOST_2D, &[0, 0, width, height, 0, 0, 1, 0])
        ),
        OK_NODATA
    );
    assert_eq!(
        status(d, &cmd(RESOURCE_FLUSH, &[0, 0, width, height, 1, 0])),
        OK_NODATA
    );
}

#[test]
fn the_device_offers_edid_and_one_scanout() {
    let mut d = gpu(1024, 768);
    assert_eq!(
        d.device_features(),
        F_INDIRECT_DESC | F_EVENT_IDX | F_VERSION_1 | (1 << 40) | (1 << 1)
    );
    let config: Vec<u32> = (0..4).map(|i| d.read(CONFIG + 4 * i)).collect();
    assert_eq!(
        config,
        [0, 0, 1, 0],
        "events_read, events_clear, num_scanouts, num_capsets"
    );
    // struct virtio_gpu_config is 16 bytes (5.7.4): QEMU's fifth field,
    // blob_alignment, is not there.
    assert_eq!(d.read_width(CONFIG + 16, AccessWidth::Byte), 0xFF);
    assert_eq!(d.read(CONFIG + 16), 0xFFFF_FFFF);
    d.write(QUEUE_SEL, 0);
    assert_eq!(d.read(QUEUE_NUM_MAX), 64);
    d.write(QUEUE_SEL, 1);
    assert_eq!(d.read(QUEUE_NUM_MAX), 16);
}

#[test]
fn display_info_reports_the_requested_size() {
    let mut d = gpu(1024, 768);
    let resp = command(&mut d, &cmd(GET_DISPLAY_INFO, &[]), 512);
    assert_eq!(resp.len(), 408);
    assert_eq!(le32(&resp, 0), OK_DISPLAY_INFO);
    let pmode0: Vec<u32> = (0..6).map(|i| le32(&resp, 24 + 4 * i)).collect();
    assert_eq!(
        pmode0,
        [0, 0, 1024, 768, 1, 0],
        "x, y, width, height, enabled, flags"
    );
    assert!(
        resp[48..].iter().all(|&b| b == 0),
        "the other fifteen scanouts are off"
    );
}

#[test]
fn get_edid_answers_with_the_displays_edid() {
    let mut d = gpu(1280, 800);
    let resp = command(&mut d, &cmd(GET_EDID, &[0, 0]), 2048);
    assert_eq!(resp.len(), 1056);
    assert_eq!(le32(&resp, 0), OK_EDID);
    assert_eq!(
        le32(&resp, 24),
        128,
        "size: the base block, with no extension (E-EDID 1.4 byte 7Eh)"
    );
    let mut large = gpu(5120, 2880);
    let large_resp = command(&mut large, &cmd(GET_EDID, &[0, 0]), 2048);
    assert_eq!(
        le32(&large_resp, 24),
        256,
        "size: a DisplayID block too, for a mode no detailed timing holds"
    );
    assert_eq!(
        resp[32..].to_vec(),
        edid::generate(1280, 800, &boards_display()).to_vec()
    );
    assert_eq!(
        status(&mut d, &cmd(GET_EDID, &[1, 0])),
        ERR_INVALID_SCANOUT_ID,
        "no scanout 1"
    );
}

#[test]
fn a_short_command_is_an_invalid_parameter() {
    let mut d = gpu(640, 480);
    assert_eq!(
        status(&mut d, &[0u8; 10]),
        ERR_INVALID_PARAMETER,
        "a short header"
    );
    let mut short_create = cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8]);
    short_create[4] = 1;
    short_create[8] = 0x42;
    let resp = command(&mut d, &short_create, 24);
    assert_eq!(le32(&resp, 0), ERR_INVALID_PARAMETER);
    assert_eq!(le32(&resp, 4), 1, "the fence is echoed");
    assert_eq!(resp[8], 0x42);
    // Nothing was created: the id is still free.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 64, 64])),
        OK_NODATA
    );
}

#[test]
fn commands_of_features_not_offered_are_unspecified_errors() {
    let mut d = gpu(640, 480);
    assert_eq!(status(&mut d, &cmd(CTX_CREATE, &[0; 24])), ERR_UNSPEC);
    assert_eq!(status(&mut d, &cmd(GET_CAPSET_INFO, &[0, 0])), ERR_UNSPEC);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_ASSIGN_UUID, &[0, 0])),
        ERR_UNSPEC
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_BLOB, &[0; 8])),
        ERR_UNSPEC
    );
    assert_eq!(status(&mut d, &cmd(SET_SCANOUT_BLOB, &[0; 20])), ERR_UNSPEC);
}

#[test]
fn fences_are_echoed_without_the_ring_index() {
    let mut d = gpu(640, 480);
    let mut req = cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 64, 64]);
    req[4..8].copy_from_slice(&3u32.to_le_bytes());
    req[8..16].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    req[16..20].copy_from_slice(&7u32.to_le_bytes());
    req[20] = 3;
    let resp = command(&mut d, &req, 24);
    assert_eq!(le32(&resp, 0), OK_NODATA);
    assert_eq!(le32(&resp, 4), 1, "only VIRTIO_GPU_FLAG_FENCE");
    assert_eq!(resp[8..20], req[8..20], "fence id and context");
    assert_eq!(resp[20], 0, "ring_idx is not echoed");
}

#[test]
fn resource_creation_checks_in_qemus_order() {
    let mut d = gpu(640, 480);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[0, B8G8R8X8, 64, 64])),
        ERR_INVALID_RESOURCE_ID
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, 99, 64, 64])),
        ERR_INVALID_PARAMETER
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 0, 64])),
        ERR_INVALID_PARAMETER,
        "a resource of no pixels"
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 64, 0])),
        ERR_INVALID_PARAMETER,
        "a resource of no rows"
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 8192, 8192])),
        ERR_OUT_OF_MEMORY,
        "more than the guest's RAM"
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 64, 64])),
        OK_NODATA
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8A8, 64, 64])),
        ERR_INVALID_RESOURCE_ID
    );
    assert_eq!(status(&mut d, &cmd(RESOURCE_UNREF, &[1, 0])), OK_NODATA);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_UNREF, &[1, 0])),
        ERR_INVALID_RESOURCE_ID
    );
}

/// An attachment request for resource `id` of `entries`, each `len` bytes
/// at `addr`.
fn attach_request(id: u32, entries: u32, addr: u64, len: u32) -> Vec<u8> {
    let mut req = cmd(RESOURCE_ATTACH_BACKING, &[id, entries]);
    for _ in 0..entries {
        req.extend_from_slice(&addr.to_le_bytes());
        req.extend_from_slice(&len.to_le_bytes());
        req.extend_from_slice(&0u32.to_le_bytes());
    }
    req
}

/// The device holds for the guest as much host memory as the guest has RAM,
/// its resources' pixels and 16 bytes for each backing entry, and nothing
/// else limits a resource or a backing list.
#[test]
fn the_guests_ram_is_the_devices_budget() {
    let ram = RAM_SIZE as u32;
    let mut d = gpu(640, 480);
    // A resource of every byte of it fits; one more pixel does not.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, ram / 4, 1])),
        OK_NODATA,
        "exactly the guest's RAM"
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[2, B8G8R8X8, 1, 1])),
        ERR_OUT_OF_MEMORY
    );
    assert_eq!(status(&mut d, &cmd(RESOURCE_UNREF, &[1, 0])), OK_NODATA);

    // A backing list longer than QEMU's 16384 entries, charged 16 bytes an
    // entry.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 16, 16])),
        OK_NODATA
    );
    let buffer = d.alloc(64);
    let entries = 16385;
    assert_eq!(
        status(&mut d, &attach_request(1, entries, buffer, 64)),
        OK_NODATA,
        "more entries than QEMU takes"
    );
    let left = ram - 16 * 16 * 4 - entries * 16;
    assert_eq!(
        status(
            &mut d,
            &cmd(RESOURCE_CREATE_2D, &[2, B8G8R8X8, left / 4, 1])
        ),
        OK_NODATA,
        "what the pixels and the entries left"
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[3, B8G8R8X8, 1, 1])),
        ERR_OUT_OF_MEMORY
    );
    // Detaching gives the entries' bytes back.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_DETACH_BACKING, &[1, 0])),
        OK_NODATA
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[3, B8G8R8X8, 1, 1])),
        OK_NODATA
    );
    // A list past what is left is refused before its entries are read.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_ATTACH_BACKING, &[1, u32::MAX])),
        ERR_OUT_OF_MEMORY
    );
    // Unreferencing a resource gives its pixels and its entries back.
    attach(&mut d, 1, &picture(16, 16));
    for id in [1, 2, 3] {
        assert_eq!(status(&mut d, &cmd(RESOURCE_UNREF, &[id, 0])), OK_NODATA);
    }
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, ram / 4, 1])),
        OK_NODATA,
        "the whole budget again"
    );
}

#[test]
fn a_frame_reaches_the_host() {
    let mut d = gpu(640, 480);
    let mut screen = Screen::default();
    assert_eq!(
        refresh(&mut d, &mut screen),
        Refreshed::Frame,
        "a blank display at the requested size"
    );
    assert_eq!((screen.width, screen.height), (640, 480));

    show_picture(&mut d, 32, 16);
    assert_eq!(refresh(&mut d, &mut screen), Refreshed::Frame);
    assert_eq!((screen.width, screen.height), (32, 16));
    for (x, y) in [(0, 0), (31, 0), (5, 7), (31, 15)] {
        let p = bgrx(x, y);
        assert_eq!(screen.pixel(x, y), [p[2], p[1], p[0], 0xFF], "({x}, {y})");
    }
    assert_eq!(refresh(&mut d, &mut screen), Refreshed::Unchanged);
    assert_eq!(
        d.host(|device, _| match device {
            Device::Gpu(g) => g.resolution(),
            Device::Input(_) | Device::Block(_) => None,
        }),
        Some(Resolution::new(32, 16))
    );
}

#[test]
fn a_partial_transfer_updates_only_its_rectangle() {
    let mut d = gpu(640, 480);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 32, 16])),
        OK_NODATA
    );
    attach(&mut d, 1, &picture(32, 16));
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[0, 0, 32, 16, 0, 1])),
        OK_NODATA
    );
    // x = 4, y = 2, 8 by 4, from the same place in the backing.
    let offset = 2 * 32 * 4 + 4 * 4;
    assert_eq!(
        status(
            &mut d,
            &cmd(TRANSFER_TO_HOST_2D, &[4, 2, 8, 4, offset, 0, 1, 0])
        ),
        OK_NODATA
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_FLUSH, &[0, 0, 32, 16, 1, 0])),
        OK_NODATA
    );
    let mut screen = Screen::default();
    refresh(&mut d, &mut screen);
    let p = bgrx(4, 2);
    assert_eq!(screen.pixel(4, 2), [p[2], p[1], p[0], 0xFF]);
    let p = bgrx(11, 5);
    assert_eq!(screen.pixel(11, 5), [p[2], p[1], p[0], 0xFF]);
    assert_eq!(screen.pixel(3, 2), [0, 0, 0, 0xFF], "left of the rectangle");
    assert_eq!(screen.pixel(4, 6), [0, 0, 0, 0xFF], "below it");
}

/// A transfer's `offset` is all 64 bits of it (virtio 1.2 5.7.6.8): one
/// 4 GiB past the place the low half names lies past the backing, so
/// nothing is read, where QEMU's 32-bit row offsets would read that place.
#[test]
fn a_transfer_offset_is_64_bits() {
    let mut d = gpu(640, 480);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 32, 16])),
        OK_NODATA
    );
    attach(&mut d, 1, &picture(32, 16));
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[0, 0, 32, 16, 0, 1])),
        OK_NODATA
    );
    // Row by row (part of the width), then whole rows at once (the whole
    // width), each with 2^32 added to the offset its rectangle starts at.
    let part = 2 * 32 * 4 + 4 * 4;
    let whole = 8 * 32 * 4;
    for args in [[4, 2, 8, 4, part, 1, 1, 0], [0, 8, 32, 4, whole, 1, 1, 0]] {
        assert_eq!(status(&mut d, &cmd(TRANSFER_TO_HOST_2D, &args)), OK_NODATA);
    }
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_FLUSH, &[0, 0, 32, 16, 1, 0])),
        OK_NODATA
    );
    let mut screen = Screen::default();
    refresh(&mut d, &mut screen);
    for (x, y) in [(4, 2), (11, 5), (0, 8), (31, 11)] {
        assert_eq!(screen.pixel(x, y), [0, 0, 0, 0xFF], "nothing transferred at ({x}, {y})");
    }
}

#[test]
fn backing_is_checked_as_qemu_maps_it() {
    let mut d = gpu(640, 480);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 16, 16])),
        OK_NODATA
    );
    let transfer = cmd(TRANSFER_TO_HOST_2D, &[0, 0, 16, 16, 0, 0, 1, 0]);
    // virtio_gpu_find_check_resource with require_backing.
    assert_eq!(status(&mut d, &transfer), ERR_UNSPEC, "no backing");
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_DETACH_BACKING, &[1, 0])),
        ERR_UNSPEC,
        "nothing to detach"
    );
    // An attachment of no entries is no backing at all.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_ATTACH_BACKING, &[1, 0])),
        OK_NODATA
    );
    assert_eq!(status(&mut d, &transfer), ERR_UNSPEC);
    for (addr, len) in [(UNASSIGNED, 64u32), (0x4010_0000, 0)] {
        assert_eq!(
            status(&mut d, &attach_request(1, 1, addr, len)),
            ERR_INVALID_PARAMETER,
            "entry {addr:#x}+{len}"
        );
    }
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_ATTACH_BACKING, &[1, 2])),
        ERR_INVALID_PARAMETER,
        "entries missing"
    );
    // The request carries the first 64 entries and not the 65th.
    let mut short = attach_request(1, 64, 0x4010_0000, 64);
    short[28..32].copy_from_slice(&65u32.to_le_bytes());
    assert_eq!(
        status(&mut d, &short),
        ERR_INVALID_PARAMETER,
        "the 65th entry missing"
    );
    assert_eq!(status(&mut d, &transfer), ERR_UNSPEC, "nothing attached");
    attach(&mut d, 1, &picture(16, 16));
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_ATTACH_BACKING, &[1, 0])),
        ERR_UNSPEC,
        "already attached"
    );
    assert_eq!(status(&mut d, &transfer), OK_NODATA);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_DETACH_BACKING, &[1, 0])),
        OK_NODATA
    );
    assert_eq!(status(&mut d, &transfer), ERR_UNSPEC);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_DETACH_BACKING, &[1, 0])),
        ERR_UNSPEC
    );
    assert_eq!(status(&mut d, &transfer), ERR_UNSPEC);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_DETACH_BACKING, &[2, 0])),
        ERR_INVALID_RESOURCE_ID
    );
    assert_eq!(
        status(
            &mut d,
            &cmd(TRANSFER_TO_HOST_2D, &[0, 0, 16, 16, 0, 0, 2, 0])
        ),
        ERR_INVALID_RESOURCE_ID
    );
}

#[test]
fn the_scanout_and_flush_are_checked() {
    let mut d = gpu(640, 480);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 32, 32])),
        OK_NODATA
    );
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[0, 0, 32, 32, 1, 1])),
        ERR_INVALID_SCANOUT_ID
    );
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[0, 0, 32, 32, 0, 2])),
        ERR_INVALID_RESOURCE_ID
    );
    // A scanout needs backing, checked before the rectangle.
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[0, 0, 0, 32, 0, 1])),
        ERR_UNSPEC
    );
    attach(&mut d, 1, &picture(32, 32));
    for rect in [[0, 0, 0, 32], [0, 0, 32, 0]] {
        assert_eq!(
            status(
                &mut d,
                &cmd(SET_SCANOUT, &[rect[0], rect[1], rect[2], rect[3], 0, 1])
            ),
            ERR_INVALID_PARAMETER,
            "no pixels: {rect:?}"
        );
    }
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[17, 0, 16, 16, 0, 1])),
        ERR_INVALID_PARAMETER,
        "past the resource"
    );
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[31, 31, 1, 1, 0, 1])),
        OK_NODATA,
        "one pixel, smaller than QEMU's 16 by 16"
    );
    assert_eq!(
        d.host(|device, _| match device {
            Device::Gpu(g) => g.resolution(),
            Device::Input(_) | Device::Block(_) => None,
        }),
        Some(Resolution::new(1, 1))
    );
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[16, 16, 16, 16, 0, 1])),
        OK_NODATA
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_FLUSH, &[0, 0, 33, 1, 1, 0])),
        ERR_INVALID_PARAMETER
    );
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_FLUSH, &[0, 0, 1, 1, 2, 0])),
        ERR_INVALID_RESOURCE_ID
    );
    // Resource 0 turns the scanout off.
    assert_eq!(
        status(&mut d, &cmd(SET_SCANOUT, &[0, 0, 0, 0, 0, 0])),
        OK_NODATA
    );
    assert_eq!(
        d.host(|device, _| match device {
            Device::Gpu(g) => g.resolution(),
            Device::Input(_) | Device::Block(_) => None,
        }),
        None
    );
}

#[test]
fn a_new_display_size_is_a_display_event() {
    let mut d = gpu(1024, 768);
    let changed = d.host(|device, _| match device {
        Device::Gpu(g) => g.ui_info(Resolution::new(800, 600)),
        Device::Input(_) | Device::Block(_) => false,
    });
    assert!(changed);
    d.transport.notify_config();
    assert_eq!(d.read(CONFIG), 1, "VIRTIO_GPU_EVENT_DISPLAY");
    assert_eq!(d.read(INTERRUPT_STATUS), 2, "a configuration change alone");
    assert_eq!(d.read(CONFIG_GENERATION), 1);
    let resp = command(&mut d, &cmd(GET_DISPLAY_INFO, &[]), 408);
    assert_eq!((le32(&resp, 32), le32(&resp, 36)), (800, 600));
    d.write(CONFIG + 4, 1);
    assert_eq!(d.read(CONFIG), 0, "events_clear cleared it");
    let unchanged = d.host(|device, _| match device {
        Device::Gpu(g) => g.ui_info(Resolution::new(800, 600)),
        Device::Input(_) | Device::Block(_) => true,
    });
    assert!(!unchanged);
}

#[test]
fn a_display_of_no_pixels_turns_the_output_off() {
    let mut d = gpu(1024, 768);
    let off = d.host(|device, _| match device {
        Device::Gpu(g) => g.ui_info(Resolution::new(0, 0)),
        Device::Input(_) | Device::Block(_) => false,
    });
    assert!(off, "a display event");
    let resp = command(&mut d, &cmd(GET_DISPLAY_INFO, &[]), 408);
    assert!(
        resp[24..].iter().all(|&b| b == 0),
        "scanout 0 disabled, as QEMU's enabled_output_bitmask says"
    );
    // The EDID of a display of no size: the base video mode, no screen
    // size.
    let resp = command(&mut d, &cmd(GET_EDID, &[0, 0]), 1056);
    assert_eq!(
        resp[32..].to_vec(),
        edid::generate(0, 0, &boards_display()).to_vec()
    );
    let on = d.host(|device, _| match device {
        Device::Gpu(g) => g.ui_info(Resolution::new(1024, 768)),
        Device::Input(_) | Device::Block(_) => false,
    });
    assert!(
        on,
        "the same size again is a change once the output was off"
    );
    let resp = command(&mut d, &cmd(GET_DISPLAY_INFO, &[]), 408);
    assert_eq!(le32(&resp, 40), 1, "enabled");
}

#[test]
fn a_reset_drops_resources_and_keeps_the_display() {
    let mut d = gpu(1024, 768);
    show_picture(&mut d, 32, 16);
    d.host(|device, _| match device {
        Device::Gpu(g) => g.ui_info(Resolution::new(800, 600)),
        Device::Input(_) | Device::Block(_) => false,
    });
    d.start(LINUX_FEATURES);
    assert_eq!(d.read(CONFIG), 1, "the pending display event stays");
    let resp = command(&mut d, &cmd(GET_DISPLAY_INFO, &[]), 408);
    assert_eq!((le32(&resp, 32), le32(&resp, 36)), (800, 600));
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_FLUSH, &[0, 0, 1, 1, 1, 0])),
        ERR_INVALID_RESOURCE_ID
    );
    let mut screen = Screen::default();
    refresh(&mut d, &mut screen);
    assert_eq!(
        (screen.width, screen.height),
        (800, 600),
        "a blank display at the requested size"
    );
}

/// Send a cursor command on the cursor queue as Linux sends it, with no
/// room for a response.
fn cursor(d: &mut Driver, kind: u32, x: u32, y: u32, resource: u32) {
    let req = d.alloc_with(&cmd(kind, &[0, x, y, 0, resource, 0, 0, 0]));
    let head = d.offer(1, &[Buf::readable(req, 56)]);
    d.notify(1);
    assert_eq!(d.used(1), [(u32::from(head), 0)]);
}

/// Both cursor commands answer `VIRTIO_GPU_RESP_OK_NODATA` (virtio 1.2
/// 5.7.6.10) to a chain with room for it, its fence echoed.
#[test]
fn a_cursor_command_answers_ok_nodata() {
    let mut d = gpu(640, 480);
    for kind in [UPDATE_CURSOR, MOVE_CURSOR] {
        let mut request = cmd(kind, &[0, 1, 2, 0, 0, 0, 0, 0]);
        request[4..8].copy_from_slice(&1u32.to_le_bytes()); // VIRTIO_GPU_FLAG_FENCE
        request[8..16].copy_from_slice(&77u64.to_le_bytes());
        let req = d.alloc_with(&request);
        let resp = d.alloc(24);
        let head = d.offer(1, &[Buf::readable(req, 56), Buf::writable(resp, 24)]);
        d.notify(1);
        assert_eq!(d.used(1), [(u32::from(head), 24)], "command {kind:#x}");
        let resp = d.mem(resp, 24).to_vec();
        assert_eq!(le32(&resp, 0), OK_NODATA, "command {kind:#x}");
        assert_eq!(le32(&resp, 4), 1, "the fence flag, echoed");
        assert_eq!(resp[8..16], 77u64.to_le_bytes(), "the fence id, echoed");
    }
    // A command too short for a cursor's, a command no cursor's, a scanout
    // the device does not have, a resource it does not have, and one that
    // is not 64 by 64, each answered as the control queue answers it.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[3, B8G8R8A8, 32, 32])),
        OK_NODATA
    );
    for (request, len, answer) in [
        (cmd(UPDATE_CURSOR, &[]), 24, ERR_INVALID_PARAMETER),
        (cmd(GET_DISPLAY_INFO, &[0, 1, 2, 0, 0, 0, 0, 0]), 56, ERR_UNSPEC),
        (cmd(MOVE_CURSOR, &[3, 1, 2, 0, 0, 0, 0, 0]), 56, ERR_INVALID_SCANOUT_ID),
        (cmd(UPDATE_CURSOR, &[0, 1, 2, 0, 99, 0, 0, 0]), 56, ERR_INVALID_RESOURCE_ID),
        (cmd(UPDATE_CURSOR, &[0, 1, 2, 0, 3, 0, 0, 0]), 56, ERR_INVALID_PARAMETER),
    ] {
        let req = d.alloc_with(&request);
        let resp = d.alloc(24);
        let head = d.offer(1, &[Buf::readable(req, len), Buf::writable(resp, 24)]);
        d.notify(1);
        assert_eq!(d.used(1), [(u32::from(head), 24)]);
        assert_eq!(le32(d.mem(resp, 24), 0), answer);
    }
    // A header the driver did not give whole echoes no fence.
    let mut request = cmd(UPDATE_CURSOR, &[]);
    request[4..8].copy_from_slice(&1u32.to_le_bytes());
    let req = d.alloc_with(&request);
    let resp = d.alloc(24);
    d.offer(1, &[Buf::readable(req, 8), Buf::writable(resp, 24)]);
    d.notify(1);
    assert_eq!(d.used(1).len(), 1);
    assert_eq!(le32(d.mem(resp, 24), 0), ERR_INVALID_PARAMETER);
    assert_eq!(le32(d.mem(resp, 24), 4), 0, "no fence flag");
}

#[test]
fn the_cursor_is_drawn_where_the_guest_puts_it() {
    let mut d = gpu(640, 480);
    show_picture(&mut d, 32, 16);
    // A 64 by 64 cursor, opaque white at its hot spot, clear elsewhere.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[2, B8G8R8A8, 64, 64])),
        OK_NODATA
    );
    let mut image = vec![0u8; 64 * 64 * 4];
    image[..4].copy_from_slice(&[0xFF; 4]);
    attach(&mut d, 2, &image);
    assert_eq!(
        status(
            &mut d,
            &cmd(TRANSFER_TO_HOST_2D, &[0, 0, 64, 64, 0, 0, 2, 0])
        ),
        OK_NODATA
    );
    let mut screen = Screen::default();
    refresh(&mut d, &mut screen);

    cursor(&mut d, UPDATE_CURSOR, 5, 6, 2);
    refresh(&mut d, &mut screen);
    assert_eq!(screen.pixel(5, 6), [0xFF, 0xFF, 0xFF, 0xFF]);
    let p = bgrx(6, 6);
    assert_eq!(
        screen.pixel(6, 6),
        [p[2], p[1], p[0], 0xFF],
        "a clear cursor pixel shows the picture"
    );

    cursor(&mut d, MOVE_CURSOR, 7, 6, 2);
    refresh(&mut d, &mut screen);
    assert_eq!(screen.pixel(7, 6), [0xFF, 0xFF, 0xFF, 0xFF]);
    let p = bgrx(5, 6);
    assert_eq!(
        screen.pixel(5, 6),
        [p[2], p[1], p[0], 0xFF],
        "where it was is redrawn"
    );

    // A move's resource is not used (virtio 1.2 5.7.6.10): it moves the
    // cursor still shown.
    cursor(&mut d, MOVE_CURSOR, 8, 6, 0);
    refresh(&mut d, &mut screen);
    assert_eq!(screen.pixel(8, 6), [0xFF, 0xFF, 0xFF, 0xFF]);

    // An update naming no resource hides it.
    cursor(&mut d, UPDATE_CURSOR, 8, 6, 0);
    refresh(&mut d, &mut screen);
    let p = bgrx(8, 6);
    assert_eq!(screen.pixel(8, 6), [p[2], p[1], p[0], 0xFF]);
}

/// The image sits with its hot spot on the pointer, so a pointer nearer the
/// screen's top-left corner than the hot spot is to the image's shows the
/// image clipped, its hot spot still on the pointer.
#[test]
fn a_cursor_near_the_corner_keeps_its_hot_spot_on_the_pointer() {
    let mut d = gpu(640, 480);
    show_picture(&mut d, 32, 16);
    // Opaque white only at (4, 4), the hot spot.
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[2, B8G8R8A8, 64, 64])),
        OK_NODATA
    );
    let mut image = vec![0u8; 64 * 64 * 4];
    let hot = (4 * 64 + 4) * 4;
    image[hot..hot + 4].copy_from_slice(&[0xFF; 4]);
    attach(&mut d, 2, &image);
    assert_eq!(
        status(
            &mut d,
            &cmd(TRANSFER_TO_HOST_2D, &[0, 0, 64, 64, 0, 0, 2, 0])
        ),
        OK_NODATA
    );
    let mut screen = Screen::default();
    refresh(&mut d, &mut screen);

    let req = d.alloc_with(&cmd(UPDATE_CURSOR, &[0, 1, 2, 0, 2, 4, 4, 0]));
    d.offer(1, &[Buf::readable(req, 56)]);
    d.notify(1);
    assert_eq!(d.used(1).len(), 1);
    refresh(&mut d, &mut screen);
    assert_eq!(screen.pixel(1, 2), [0xFF, 0xFF, 0xFF, 0xFF]);
    let p = bgrx(4, 4);
    assert_eq!(
        screen.pixel(4, 4),
        [p[2], p[1], p[0], 0xFF],
        "the image is not pushed inside the screen"
    );
}

/// A display asking for no size (QEMU's `xres=0,yres=0`, whose EDID falls
/// back to 1280 by 800) is as restorable as any other.
#[test]
fn a_display_of_no_requested_size_survives_a_snapshot() {
    let d = gpu(0, 0);
    let saved = d.save();
    let resumed = d.restored(Device::Gpu(display(Resolution::new(1, 1))), &saved);
    assert_eq!(resumed.save(), saved);
}

#[test]
fn a_snapshot_shows_the_same_picture() {
    let mut d = gpu(640, 480);
    show_picture(&mut d, 32, 16);
    let saved = d.save();
    let mut resumed = d.restored(Device::Gpu(display(Resolution::new(1, 1))), &saved);
    assert_eq!(resumed.save(), saved);
    let (mut a, mut b) = (Screen::default(), Screen::default());
    refresh(&mut d, &mut a);
    refresh(&mut resumed, &mut b);
    assert_eq!((b.width, b.height), (32, 16));
    assert!(a.rgba == b.rgba);
    // The resumed device goes on with its resources.
    assert_eq!(
        status(&mut resumed, &cmd(RESOURCE_FLUSH, &[0, 0, 32, 16, 1, 0])),
        OK_NODATA
    );
    // And with what they took of the budget: the picture's pixels and its
    // one backing entry.
    let left = RAM_SIZE as u32 - 32 * 16 * 4 - 16;
    assert_eq!(
        status(
            &mut resumed,
            &cmd(RESOURCE_CREATE_2D, &[2, B8G8R8X8, left / 4, 1])
        ),
        OK_NODATA
    );
    assert_eq!(
        status(&mut resumed, &cmd(RESOURCE_CREATE_2D, &[3, B8G8R8X8, 1, 1])),
        ERR_OUT_OF_MEMORY
    );
}

/// Send the request in `buffers` on the control queue, room for a bare
/// header at `resp` after them; the response type.
fn status_of_chain(d: &mut Driver, buffers: &[Buf], resp: u64) -> u32 {
    let mut chain = buffers.to_vec();
    chain.push(Buf::writable(resp, 24));
    d.offer(0, &chain);
    d.notify(0);
    assert_eq!(d.used(0).len(), 1, "one command, one answer");
    le32(d.mem(resp, 4), 0)
}

/// As many resources as the budget holds survive a snapshot, past any
/// count but the budget's: 65,537 resources of a pixel each, the last of
/// which the resumed device still answers for.
#[test]
fn a_snapshot_keeps_every_resource_the_budget_holds() {
    const COUNT: u32 = (1 << 16) + 1;
    let mut d = gpu(640, 480);
    // One request buffer and one response buffer for every command, so the
    // driver's memory holds them all.
    let req = d.alloc(40);
    let resp = d.alloc(24);
    for id in 1..=COUNT {
        d.put(req, &cmd(RESOURCE_CREATE_2D, &[id, B8G8R8X8, 1, 1]));
        assert_eq!(
            status_of_chain(&mut d, &[Buf::readable(req, 40)], resp),
            OK_NODATA,
            "resource {id}"
        );
    }
    let saved = d.save();
    let mut resumed = d.restored(Device::Gpu(display(Resolution::new(1, 1))), &saved);
    assert_eq!(resumed.save(), saved);
    assert_eq!(
        status(&mut resumed, &cmd(RESOURCE_FLUSH, &[0, 0, 1, 1, COUNT, 0])),
        OK_NODATA,
        "the last resource is still there"
    );
    assert_eq!(
        status(
            &mut resumed,
            &cmd(RESOURCE_CREATE_2D, &[COUNT, B8G8R8X8, 1, 1])
        ),
        ERR_INVALID_RESOURCE_ID,
        "and its id still taken"
    );
    // And every one still takes its pixel of the budget.
    let left = RAM_SIZE as u32 - COUNT * 4;
    assert_eq!(
        status(
            &mut resumed,
            &cmd(RESOURCE_CREATE_2D, &[COUNT + 1, B8G8R8X8, left / 4, 1])
        ),
        OK_NODATA
    );
    assert_eq!(
        status(
            &mut resumed,
            &cmd(RESOURCE_CREATE_2D, &[COUNT + 2, B8G8R8X8, 1, 1])
        ),
        ERR_OUT_OF_MEMORY
    );
}

/// The start of a device's state as `Gpu::save` lays it out: a display of
/// 640 by 480, enabled, `count` resources, and then `resources`, each its
/// id, 1 by 1, its pixel and `entries` backing entries of 4 bytes.
fn saved_resources(count: u32, resources: &[(u32, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [640u32, 480] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(1);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for &(id, entries) in resources {
        for v in [id, B8G8R8X8, 1, 1, 0, entries] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..entries {
            out.extend_from_slice(&RAM_BASE.to_le_bytes());
            out.extend_from_slice(&4u32.to_le_bytes());
        }
    }
    out
}

/// What restoring `saved` onto a fresh device says, which reads it to its
/// end or refuses it.
fn restore_error(saved: &[u8]) -> Option<rusty_box_core::snap::SnapError> {
    let mut r: &[u8] = saved;
    display(Resolution::new(1, 1)).restore(&mut r).err()
}

/// A restore refuses a count of resources or of backing entries past what
/// the budget holds, before reading any of them; up to that, it reads on
/// until the image runs out. Two resources of one id are refused too.
#[test]
fn a_snapshot_past_the_budget_is_refused() {
    use rusty_box_core::snap::SnapError;
    let most = (RAM_SIZE / 4) as u32;
    assert_eq!(
        restore_error(&saved_resources(most, &[])),
        Some(SnapError::Truncated),
        "as many resources as the budget holds"
    );
    for count in [most + 1, u32::MAX] {
        assert_eq!(
            restore_error(&saved_resources(count, &[])),
            Some(SnapError::Invalid(
                "virtio-gpu resources past what the budget holds"
            ))
        );
    }
    let room = ((RAM_SIZE - 4) / 16) as u32;
    let mut short = saved_resources(1, &[(1, 0)]);
    let at = short.len() - 4;
    short[at..].copy_from_slice(&room.to_le_bytes());
    assert_eq!(
        restore_error(&short),
        Some(SnapError::Truncated),
        "as many entries as the budget has left room for"
    );
    for entries in [room + 1, u32::MAX] {
        short[at..].copy_from_slice(&entries.to_le_bytes());
        assert_eq!(
            restore_error(&short),
            Some(SnapError::Invalid(
                "virtio-gpu backing list past what the budget has left"
            ))
        );
    }
    assert_eq!(
        restore_error(&saved_resources(2, &[(7, 1), (7, 1)])),
        Some(SnapError::Invalid(
            "virtio-gpu resource the device could not have made"
        ))
    );
}

/// As long a backing list as the budget has room for survives a snapshot,
/// past any count but the budget's: 2^20 + 1 entries, on a device whose
/// guest has 32 MiB of RAM. The list repeats one buffer in the test
/// driver's RAM, as a guest may.
#[test]
fn a_snapshot_keeps_every_backing_entry_the_budget_holds() {
    const RAM: usize = 32 << 20;
    const BLOCK_ENTRIES: u32 = 1 << 16;
    const ENTRIES: u32 = 16 * BLOCK_ENTRIES + 1;
    let device = |preferred| Device::Gpu(Gpu::new(preferred, boards_display(), RAM));
    let mut d = Driver::new(device(Resolution::new(640, 480)));
    d.start(LINUX_FEATURES);
    assert_eq!(
        status(&mut d, &cmd(RESOURCE_CREATE_2D, &[1, B8G8R8X8, 1, 1])),
        OK_NODATA
    );
    let pixel = d.alloc_with(&[1, 2, 3, 4]);
    let mut entry = pixel.to_le_bytes().to_vec();
    entry.extend_from_slice(&4u32.to_le_bytes());
    entry.extend_from_slice(&0u32.to_le_bytes());
    let block = d.alloc_with(&entry.repeat(BLOCK_ENTRIES as usize));
    let header = d.alloc_with(&cmd(RESOURCE_ATTACH_BACKING, &[1, ENTRIES]));
    let resp = d.alloc(24);
    let mut request = vec![Buf::readable(header, 32)];
    request.extend([Buf::readable(block, BLOCK_ENTRIES * 16); 16]);
    request.push(Buf::readable(block, 16));
    assert_eq!(status_of_chain(&mut d, &request, resp), OK_NODATA);

    let saved = d.save();
    let mut resumed = d.restored(device(Resolution::new(1, 1)), &saved);
    assert_eq!(resumed.save(), saved);
    assert_eq!(
        status(
            &mut resumed,
            &cmd(TRANSFER_TO_HOST_2D, &[0, 0, 1, 1, 0, 0, 1, 0])
        ),
        OK_NODATA,
        "the resource still has its backing"
    );
    // And the list still takes 16 bytes an entry of the budget.
    let left = RAM as u32 - 4 - ENTRIES * 16;
    assert_eq!(
        status(
            &mut resumed,
            &cmd(RESOURCE_CREATE_2D, &[2, B8G8R8X8, left / 4, 1])
        ),
        OK_NODATA
    );
    assert_eq!(
        status(&mut resumed, &cmd(RESOURCE_CREATE_2D, &[3, B8G8R8X8, 1, 1])),
        ERR_OUT_OF_MEMORY
    );
}
