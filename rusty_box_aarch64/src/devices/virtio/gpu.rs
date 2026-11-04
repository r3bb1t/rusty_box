//! virtio-gpu, 2D: QEMU's `virtio-gpu-device` with its default properties.
//!
//! Provenance (R7): the virtio 1.2 specification, section 5.7, as QEMU
//! implements it in `hw/display/virtio-gpu.c` and
//! `hw/display/virtio-gpu-base.c`:
//!
//! - one scanout, `VIRTIO_GPU_F_EDID` and nothing else (`edid` on; no virgl,
//!   blob, UUID or context features), a control queue of 64 entries and a
//!   cursor queue of 16 (`virtio_gpu_base_device_realize`), `max_hostmem`
//!   256 MiB;
//! - `virtio_gpu_handle_ctrl`: every available command is taken, then each is
//!   run (`virtio_gpu_simple_process_cmd`) and answered in turn, a fence
//!   echoed (`virtio_gpu_ctrl_response`); a command shorter than its
//!   structure is answered `ERR_INVALID_PARAMETER` (`VIRTIO_GPU_FILL_CMD`),
//!   an unknown one `ERR_UNSPEC`, a blob command `ERR_INVALID_PARAMETER`;
//! - `virtio_gpu_get_display_info` and `virtio_gpu_get_edid` from the
//!   requested display size (`virtio_gpu_base_fill_display_info`,
//!   `virtio_gpu_base_generate_edid`), which the host changes with a display
//!   event (`virtio_gpu_ui_info`, `virtio_gpu_notify_event`);
//! - `virtio_gpu_resource_create_2d`, `_unref`, `virtio_gpu_set_scanout`,
//!   `virtio_gpu_resource_flush`, `virtio_gpu_transfer_to_host_2d`,
//!   `virtio_gpu_resource_attach_backing` (`virtio_gpu_create_mapping_iov`)
//!   and `_detach_backing`, with their checks and error codes in QEMU's
//!   order;
//! - `virtio_gpu_handle_cursor` and `update_cursor`;
//! - `virtio_gpu_reset` and `virtio_gpu_base_reset`: resources and the
//!   scanout go, the requested size, pending events and the cursor stay;
//! - `virtio_gpu_get_config` and `virtio_gpu_set_config`.
//!
//! Resources live in host memory, as QEMU's pixman images do (it maps them
//! from a memfd, so a resource of no bytes cannot be made). How the picture
//! reaches a host window is the front end's business in QEMU; here
//! [`Gpu::refresh`] pushes the scanout's changed rectangles into a
//! [`DisplaySink`], with the guest's cursor drawn over them.
//!
//! Host resources are pixel buffers of the guest's choosing, so this device
//! needs an allocator; the board offers it only with `alloc`.

use alloc::vec;
use alloc::vec::Vec;

use rusty_box_core::snap::{SnapError, SnapRead, SnapResult, SnapWrite};
use rusty_box_devices::display::sink::{Dimensions, DisplaySink, Refreshed, TilePos};
use rusty_box_devices::display::Resolution;

use super::edid;
use super::queue::{Chain, GuestRam};
use super::Vdev;
use crate::identity::DisplayIdentity;

pub const CONTROL_QUEUE: usize = 0;
pub const CURSOR_QUEUE: usize = 1;
pub const CONTROL_QUEUE_SIZE: u16 = 64;
pub const CURSOR_QUEUE_SIZE: u16 = 16;

/// `VIRTIO_GPU_F_EDID`.
pub const FEATURES: u64 = 1 << 1;
/// `sizeof(struct virtio_gpu_config)` in QEMU 11.1.2's copy of the Linux
/// header: `events_read`, `events_clear`, `num_scanouts`, `num_capsets`,
/// `blob_alignment`.
pub const CONFIG_LEN: usize = 20;

/// QEMU `virtio-gpu`'s default `max_hostmem`: the guest's resources must
/// stay below it.
const MAX_HOSTMEM: u64 = 256 << 20;
/// The most backing entries one resource may have (QEMU
/// `virtio_gpu_create_mapping_iov`).
const MAX_BACKING_ENTRIES: u32 = 16384;
/// The cursor image: 64 by 64 (QEMU `cursor_alloc(64, 64)`).
const CURSOR_SIZE: u32 = 64;
const CURSOR_PIXELS: usize = (CURSOR_SIZE * CURSOR_SIZE) as usize;

const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_RESOURCE_UNREF: u32 = 0x0102;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
const CMD_GET_EDID: u32 = 0x010A;
const CMD_RESOURCE_CREATE_BLOB: u32 = 0x010C;
const CMD_SET_SCANOUT_BLOB: u32 = 0x010D;
const CMD_MOVE_CURSOR: u32 = 0x0301;

const RESP_OK_NODATA: u32 = 0x1100;
const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const RESP_OK_EDID: u32 = 0x1104;
const RESP_ERR_UNSPEC: u32 = 0x1200;
const RESP_ERR_OUT_OF_MEMORY: u32 = 0x1201;
const RESP_ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
const RESP_ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
const RESP_ERR_INVALID_PARAMETER: u32 = 0x1205;

const FLAG_FENCE: u32 = 1;
/// `VIRTIO_GPU_EVENT_DISPLAY`, in `events_read`.
const EVENT_DISPLAY: u32 = 1;
/// `VIRTIO_GPU_MAX_SCANOUTS`: the display-info response has this many.
const MAX_SCANOUTS: usize = 16;

/// `struct virtio_gpu_ctrl_hdr`.
const HDR_LEN: usize = 24;
/// `struct virtio_gpu_resp_display_info`.
const DISPLAY_INFO_LEN: usize = HDR_LEN + 24 * MAX_SCANOUTS;
/// `struct virtio_gpu_resp_edid`.
const EDID_RESPONSE_LEN: usize = HDR_LEN + 8 + edid::EDID_LEN;
/// `struct virtio_gpu_update_cursor`.
const CURSOR_COMMAND_LEN: usize = 56;

/// A pixel format the guest may create a resource in, by the order of its
/// bytes in memory (QEMU `virtio_gpu_get_pixman_format` on a little-endian
/// host).
///
/// Closed (R5): the eight formats 2D virtio-gpu accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    B8G8R8A8,
    B8G8R8X8,
    A8R8G8B8,
    X8R8G8B8,
    R8G8B8A8,
    X8B8G8R8,
    A8B8G8R8,
    R8G8B8X8,
}

impl Format {
    fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::B8G8R8A8,
            2 => Self::B8G8R8X8,
            3 => Self::A8R8G8B8,
            4 => Self::X8R8G8B8,
            67 => Self::R8G8B8A8,
            68 => Self::X8B8G8R8,
            121 => Self::A8B8G8R8,
            134 => Self::R8G8B8X8,
            _ => return None,
        })
    }

    fn to_wire(self) -> u32 {
        match self {
            Self::B8G8R8A8 => 1,
            Self::B8G8R8X8 => 2,
            Self::A8R8G8B8 => 3,
            Self::X8R8G8B8 => 4,
            Self::R8G8B8A8 => 67,
            Self::X8B8G8R8 => 68,
            Self::A8B8G8R8 => 121,
            Self::R8G8B8X8 => 134,
        }
    }

    /// Red, green, blue and alpha of the pixel whose bytes are `p`; a
    /// format without alpha is opaque.
    fn rgba(self, p: [u8; 4]) -> [u8; 4] {
        match self {
            Self::B8G8R8A8 => [p[2], p[1], p[0], p[3]],
            Self::B8G8R8X8 => [p[2], p[1], p[0], 0xFF],
            Self::A8R8G8B8 => [p[1], p[2], p[3], p[0]],
            Self::X8R8G8B8 => [p[1], p[2], p[3], 0xFF],
            Self::R8G8B8A8 => [p[0], p[1], p[2], p[3]],
            Self::X8B8G8R8 => [p[3], p[2], p[1], 0xFF],
            Self::A8B8G8R8 => [p[3], p[2], p[1], p[0]],
            Self::R8G8B8X8 => [p[0], p[1], p[2], 0xFF],
        }
    }
}

/// `struct virtio_gpu_rect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    fn from_le(b: &[u8]) -> Self {
        Self {
            x: le32(b, 0),
            y: le32(b, 4),
            width: le32(b, 8),
            height: le32(b, 12),
        }
    }

    /// Whether this rectangle lies inside a `width` by `height` area.
    fn within(self, width: u32, height: u32) -> bool {
        u64::from(self.x) + u64::from(self.width) <= u64::from(width)
            && u64::from(self.y) + u64::from(self.height) <= u64::from(height)
    }

    fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The smallest rectangle covering both.
    fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (u64::from(self.x) + u64::from(self.width))
            .max(u64::from(other.x) + u64::from(other.width));
        let y1 = (u64::from(self.y) + u64::from(self.height))
            .max(u64::from(other.y) + u64::from(other.height));
        Rect {
            x: x0,
            y: y0,
            width: (x1 - u64::from(x0)).min(u64::from(u32::MAX)) as u32,
            height: (y1 - u64::from(y0)).min(u64::from(u32::MAX)) as u32,
        }
    }

    /// The part of this rectangle inside `other` (QEMU
    /// `qemu_rect_intersect`), empty when they do not meet.
    fn intersect(self, other: Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (u64::from(self.x) + u64::from(self.width))
            .min(u64::from(other.x) + u64::from(other.width));
        let y1 = (u64::from(self.y) + u64::from(self.height))
            .min(u64::from(other.y) + u64::from(other.height));
        if x1 <= u64::from(x0) || y1 <= u64::from(y0) {
            return Rect::default();
        }
        Rect {
            x: x0,
            y: y0,
            width: (x1 - u64::from(x0)) as u32,
            height: (y1 - u64::from(y0)) as u32,
        }
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        b[at],
        b[at + 1],
        b[at + 2],
        b[at + 3],
        b[at + 4],
        b[at + 5],
        b[at + 6],
        b[at + 7],
    ])
}

/// One guest buffer backing a resource (`struct virtio_gpu_mem_entry`).
#[derive(Debug, Clone, Copy)]
struct BackingEntry {
    addr: u64,
    len: u32,
}

/// A host resource: the guest's pixels as of its last transfer.
struct Resource {
    id: u32,
    format: Format,
    width: u32,
    height: u32,
    /// Four bytes a pixel, `width * 4` a row.
    pixels: Vec<u8>,
    /// The guest memory attached to it; empty while none is (QEMU's
    /// `res->iov` is `NULL` for an attachment of no entries too).
    backing: Vec<BackingEntry>,
}

impl Resource {
    fn stride(&self) -> u32 {
        self.width * 4
    }

    /// QEMU `iov_to_buf` over the backing: fill `out` from `offset` bytes
    /// in, as far as the backing reaches.
    fn read_backing(&self, ram: &GuestRam<'_>, offset: u64, out: &mut [u8]) {
        let mut skip = offset;
        let mut done = 0;
        for entry in &self.backing {
            if done == out.len() {
                break;
            }
            let len = u64::from(entry.len);
            if skip >= len {
                skip -= len;
                continue;
            }
            let n = ((len - skip) as usize).min(out.len() - done);
            let Some(at) = entry.addr.checked_add(skip) else {
                break;
            };
            if ram.read(at, &mut out[done..done + n]).is_none() {
                // Attaching checked every entry lies in guest RAM, which does
                // not move: nothing is left unread.
                break;
            }
            done += n;
            skip = 0;
        }
    }
}

/// The scanout: which resource is shown, and which part of it.
#[derive(Debug, Clone, Copy, Default)]
struct Scanout {
    resource_id: u32,
    rect: Rect,
}

/// The hardware cursor (QEMU `update_cursor`): the 64 by 64 image, its hot
/// spot, where it is, and whether the console shows it
/// (`qemu_console_set_mouse`'s `on`).
struct Cursor {
    /// The raw pixels of the resource last defined as the cursor, read as
    /// QEMU's `QEMUCursor` reads them (32-bit ARGB, premultiplied); empty
    /// before any.
    image: Vec<[u8; 4]>,
    hot_x: u32,
    hot_y: u32,
    x: u32,
    y: u32,
    shown: bool,
}

/// Where the cursor image's top-left pixel falls on the screen, which may
/// be above or left of it.
#[derive(Debug, Clone, Copy)]
struct CursorOrigin {
    left: i64,
    top: i64,
}

/// How a command ended.
enum Reply {
    /// Answered `RESP_OK_NODATA`.
    Done,
    /// Answered with this error.
    Error(u32),
    /// The command wrote its own response.
    Answered,
}

/// Whether the display asks the guest to drive scanout 0
/// (`enabled_output_bitmask` bit 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    Enabled,
    Disabled,
}

/// A virtio-gpu device.
pub struct Gpu {
    /// `req_state[0]`: the size the display asks the guest for.
    preferred: Resolution,
    /// The maker, product and name the display's EDID gives.
    display: DisplayIdentity,
    output: Output,
    /// `virtio_config.events_read`.
    events_read: u32,
    resources: Vec<Resource>,
    hostmem: u64,
    scanout: Scanout,
    cursor: Cursor,
    /// Commands taken from the control queue and not yet run (QEMU
    /// `cmdq`); empty between notifications.
    cmdq: Vec<Chain>,
    /// What the host has not yet been shown.
    dirty: Rect,
    /// The scanout's size changed since the host last looked.
    resized: bool,
    /// The rectangle a frame is converted in, reused.
    frame: Vec<u8>,
}

impl Gpu {
    /// A device asking for `preferred`, its output enabled whatever the size
    /// (`virtio_gpu_base_device_realize`), whose EDID gives `display`.
    pub(crate) fn new(preferred: Resolution, display: DisplayIdentity) -> Self {
        Self {
            preferred,
            display,
            output: Output::Enabled,
            events_read: 0,
            resources: Vec::new(),
            hostmem: 0,
            scanout: Scanout::default(),
            cursor: Cursor {
                image: Vec::new(),
                hot_x: 0,
                hot_y: 0,
                x: 0,
                y: 0,
                shown: false,
            },
            cmdq: Vec::new(),
            dirty: Rect::default(),
            resized: true,
            frame: Vec::new(),
        }
    }

    /// QEMU `virtio_gpu_reset` and `virtio_gpu_base_reset`.
    pub fn reset(&mut self) {
        self.resources.clear();
        self.hostmem = 0;
        self.cmdq.clear();
        self.disable_scanout();
    }

    /// The size the display asks the guest for.
    pub fn preferred(&self) -> Resolution {
        self.preferred
    }

    /// QEMU `virtio_gpu_ui_info` for a display of `size`: the requested size
    /// changes, the output is enabled unless the size is empty, and a
    /// display event is raised. Returns whether there was anything to
    /// change, as QEMU's console passes on only a change
    /// (`qemu_console_set_ui_info`); the caller then notifies the configuration
    /// change, as `virtio_gpu_notify_event` does.
    pub fn ui_info(&mut self, size: Resolution) -> bool {
        let output = if size.width != 0 && size.height != 0 {
            Output::Enabled
        } else {
            Output::Disabled
        };
        if size == self.preferred && output == self.output {
            return false;
        }
        self.preferred = size;
        self.output = output;
        self.events_read |= EVENT_DISPLAY;
        true
    }

    /// QEMU `virtio_gpu_get_config`: `events_clear` is never kept, one
    /// scanout, no capability sets, and no blob alignment (a 2D device has
    /// no blobs).
    pub fn get_config(&self, config: &mut [u8]) {
        config[..4].copy_from_slice(&self.events_read.to_le_bytes());
        config[4..8].fill(0);
        config[8..12].copy_from_slice(&1u32.to_le_bytes());
        config[12..CONFIG_LEN].fill(0);
    }

    /// QEMU `virtio_gpu_set_config`: `events_clear` clears those events.
    pub fn set_config(&mut self, config: &[u8]) {
        let clear = le32(config, 4);
        if clear != 0 {
            self.events_read &= !clear;
        }
    }

    /// The size of the picture the guest shows, if it shows one.
    pub fn resolution(&self) -> Option<Resolution> {
        (self.scanout.resource_id != 0).then_some(Resolution::new(
            self.scanout.rect.width,
            self.scanout.rect.height,
        ))
    }

    fn resource(&self, id: u32) -> Option<&Resource> {
        self.resources.iter().find(|r| r.id == id)
    }

    fn resource_index(&self, id: u32) -> Option<usize> {
        self.resources.iter().position(|r| r.id == id)
    }

    /// QEMU `virtio_gpu_find_check_resource` with `require_backing`: the
    /// resource, if it exists (else `RESP_ERR_INVALID_RESOURCE_ID`) and has
    /// backing attached (else `RESP_ERR_UNSPEC`).
    fn backed_resource_index(&self, id: u32) -> Result<usize, Reply> {
        let index = self
            .resource_index(id)
            .ok_or(Reply::Error(RESP_ERR_INVALID_RESOURCE_ID))?;
        if self.resources[index].backing.is_empty() {
            return Err(Reply::Error(RESP_ERR_UNSPEC));
        }
        Ok(index)
    }

    /// The driver notified queue `q`.
    pub fn handle_output(&mut self, vdev: &mut Vdev<'_, '_>, q: usize) {
        match q {
            CONTROL_QUEUE => self.handle_ctrl(vdev),
            CURSOR_QUEUE => self.handle_cursor(vdev),
            _ => {}
        }
    }

    /// QEMU `virtio_gpu_handle_ctrl` and `virtio_gpu_process_cmdq`.
    fn handle_ctrl(&mut self, vdev: &mut Vdev<'_, '_>) {
        if !vdev.queue_ready(CONTROL_QUEUE) {
            return;
        }
        let mut cmdq = core::mem::take(&mut self.cmdq);
        while let Some(chain) = vdev.pop(CONTROL_QUEUE) {
            cmdq.push(chain);
        }
        for chain in cmdq.drain(..) {
            self.process_cmd(vdev, &chain);
        }
        self.cmdq = cmdq;
    }

    /// QEMU `virtio_gpu_handle_cursor`.
    fn handle_cursor(&mut self, vdev: &mut Vdev<'_, '_>) {
        if !vdev.queue_ready(CURSOR_QUEUE) {
            return;
        }
        while let Some(chain) = vdev.pop(CURSOR_QUEUE) {
            let mut command = [0u8; CURSOR_COMMAND_LEN];
            if vdev.read(&chain, 0, &mut command) == CURSOR_COMMAND_LEN {
                self.update_cursor(&command);
            }
            vdev.push(CURSOR_QUEUE, &chain, 0);
            vdev.notify(CURSOR_QUEUE);
        }
    }

    /// QEMU `update_cursor`: any command but a move defines the cursor.
    fn update_cursor(&mut self, b: &[u8; CURSOR_COMMAND_LEN]) {
        // virtio_gpu_cursor_pos at 24: scanout_id, x, y; then resource_id,
        // hot_x, hot_y.
        let (scanout_id, x, y) = (le32(b, 24), le32(b, 28), le32(b, 32));
        let (resource_id, hot_x, hot_y) = (le32(b, 40), le32(b, 44), le32(b, 48));
        if scanout_id != 0 {
            return;
        }
        let before = self.cursor_rect();
        if le32(b, 0) != CMD_MOVE_CURSOR {
            if self.cursor.image.is_empty() {
                self.cursor.image = vec![[0; 4]; CURSOR_PIXELS];
            }
            self.cursor.hot_x = hot_x;
            self.cursor.hot_y = hot_y;
            if resource_id > 0 {
                // virtio_gpu_update_cursor_data: only a 64 by 64 resource.
                let res = self.resources.iter().find(|r| r.id == resource_id);
                if let Some(res) = res.filter(|r| r.width == CURSOR_SIZE && r.height == CURSOR_SIZE)
                {
                    let pixels = res.pixels.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]);
                    for (dst, src) in self.cursor.image.iter_mut().zip(pixels) {
                        *dst = src;
                    }
                }
            }
        }
        self.cursor.x = x;
        self.cursor.y = y;
        self.cursor.shown = resource_id != 0;
        let after = self.cursor_rect();
        self.dirty = self.dirty.union(before).union(after);
    }

    /// Where the cursor covers the screen, empty while it is not shown.
    fn cursor_rect(&self) -> Rect {
        if !self.cursor.shown || self.cursor.image.is_empty() {
            return Rect::default();
        }
        let origin = self.cursor_origin();
        let size = i64::from(CURSOR_SIZE);
        let left = origin.left.max(0);
        let top = origin.top.max(0);
        let right = origin.left + size;
        let bottom = origin.top + size;
        if right <= left || bottom <= top {
            return Rect::default();
        }
        Rect {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        }
    }

    /// Where the cursor image's top-left pixel falls: the pointer less the
    /// hot spot, above or left of the screen when the pointer is nearer its
    /// edge than the hot spot is to the image's.
    fn cursor_origin(&self) -> CursorOrigin {
        CursorOrigin {
            left: i64::from(self.cursor.x) - i64::from(self.cursor.hot_x),
            top: i64::from(self.cursor.y) - i64::from(self.cursor.hot_y),
        }
    }

    /// `VIRTIO_GPU_FILL_CMD`: the command's structure, or `None` when the
    /// driver's buffers are shorter.
    fn fill<const N: usize>(vdev: &Vdev<'_, '_>, chain: &Chain) -> Option<[u8; N]> {
        let mut b = [0u8; N];
        (vdev.read(chain, 0, &mut b) == N).then_some(b)
    }

    /// QEMU `virtio_gpu_simple_process_cmd`.
    fn process_cmd(&mut self, vdev: &mut Vdev<'_, '_>, chain: &Chain) {
        let Some(hdr) = Self::fill::<HDR_LEN>(vdev, chain) else {
            Self::respond_nodata(vdev, chain, &[0; HDR_LEN], RESP_ERR_INVALID_PARAMETER);
            return;
        };
        let short = Reply::Error(RESP_ERR_INVALID_PARAMETER);
        let reply = match le32(&hdr, 0) {
            CMD_GET_DISPLAY_INFO => {
                self.respond_display_info(vdev, chain, &hdr);
                Reply::Answered
            }
            CMD_GET_EDID => match Self::fill::<32>(vdev, chain) {
                Some(b) if le32(&b, 24) == 0 => {
                    self.respond_edid(vdev, chain, &hdr);
                    Reply::Answered
                }
                Some(_) | None => short,
            },
            CMD_RESOURCE_CREATE_2D => match Self::fill::<40>(vdev, chain) {
                Some(b) => self.create_2d(le32(&b, 24), le32(&b, 28), le32(&b, 32), le32(&b, 36)),
                None => short,
            },
            CMD_RESOURCE_UNREF => match Self::fill::<32>(vdev, chain) {
                Some(b) => self.unref(le32(&b, 24)),
                None => short,
            },
            CMD_RESOURCE_FLUSH => match Self::fill::<48>(vdev, chain) {
                Some(b) => self.flush(Rect::from_le(&b[24..40]), le32(&b, 40)),
                None => short,
            },
            CMD_TRANSFER_TO_HOST_2D => match Self::fill::<56>(vdev, chain) {
                Some(b) => self.transfer(
                    vdev.ram(),
                    Rect::from_le(&b[24..40]),
                    le64(&b, 40),
                    le32(&b, 48),
                ),
                None => short,
            },
            CMD_SET_SCANOUT => match Self::fill::<48>(vdev, chain) {
                Some(b) => self.set_scanout(Rect::from_le(&b[24..40]), le32(&b, 40), le32(&b, 44)),
                None => short,
            },
            CMD_RESOURCE_ATTACH_BACKING => match Self::fill::<32>(vdev, chain) {
                Some(b) => self.attach_backing(vdev, chain, le32(&b, 24), le32(&b, 28)),
                None => short,
            },
            CMD_RESOURCE_DETACH_BACKING => match Self::fill::<32>(vdev, chain) {
                Some(b) => self.detach_backing(le32(&b, 24)),
                None => short,
            },
            // Blob resources are off.
            CMD_RESOURCE_CREATE_BLOB | CMD_SET_SCANOUT_BLOB => short,
            _ => Reply::Error(RESP_ERR_UNSPEC),
        };
        match reply {
            Reply::Done => Self::respond_nodata(vdev, chain, &hdr, RESP_OK_NODATA),
            Reply::Error(code) => Self::respond_nodata(vdev, chain, &hdr, code),
            Reply::Answered => {}
        }
    }

    /// QEMU `virtio_gpu_ctrl_response`: echo the command's fence into
    /// `resp`, write it, hand the chain back and notify.
    fn respond(vdev: &mut Vdev<'_, '_>, chain: &Chain, cmd: &[u8; HDR_LEN], resp: &mut [u8]) {
        if le32(cmd, 4) & FLAG_FENCE != 0 {
            let flags = le32(resp, 4) | FLAG_FENCE;
            resp[4..8].copy_from_slice(&flags.to_le_bytes());
            resp[8..20].copy_from_slice(&cmd[8..20]);
        }
        let written = vdev.write(chain, 0, resp);
        vdev.push(CONTROL_QUEUE, chain, written as u32);
        vdev.notify(CONTROL_QUEUE);
    }

    /// QEMU `virtio_gpu_ctrl_response_nodata`.
    fn respond_nodata(vdev: &mut Vdev<'_, '_>, chain: &Chain, cmd: &[u8; HDR_LEN], kind: u32) {
        let mut resp = [0u8; HDR_LEN];
        resp[..4].copy_from_slice(&kind.to_le_bytes());
        Self::respond(vdev, chain, cmd, &mut resp);
    }

    /// QEMU `virtio_gpu_get_display_info`: scanout 0 at the requested size
    /// while its output is enabled, all zeros while it is not.
    fn respond_display_info(&self, vdev: &mut Vdev<'_, '_>, chain: &Chain, cmd: &[u8; HDR_LEN]) {
        let mut resp = [0u8; DISPLAY_INFO_LEN];
        resp[..4].copy_from_slice(&RESP_OK_DISPLAY_INFO.to_le_bytes());
        match self.output {
            Output::Enabled => {
                // pmodes[0]: r = (0, 0, width, height), enabled = 1, flags = 0.
                resp[HDR_LEN + 8..HDR_LEN + 12]
                    .copy_from_slice(&self.preferred.width.to_le_bytes());
                resp[HDR_LEN + 12..HDR_LEN + 16]
                    .copy_from_slice(&self.preferred.height.to_le_bytes());
                resp[HDR_LEN + 16..HDR_LEN + 20].copy_from_slice(&1u32.to_le_bytes());
            }
            Output::Disabled => {}
        }
        Self::respond(vdev, chain, cmd, &mut resp);
    }

    /// QEMU `virtio_gpu_get_edid` for scanout 0.
    fn respond_edid(&self, vdev: &mut Vdev<'_, '_>, chain: &Chain, cmd: &[u8; HDR_LEN]) {
        let mut resp = [0u8; EDID_RESPONSE_LEN];
        resp[..4].copy_from_slice(&RESP_OK_EDID.to_le_bytes());
        resp[HDR_LEN..HDR_LEN + 4].copy_from_slice(&(edid::EDID_LEN as u32).to_le_bytes());
        resp[HDR_LEN + 8..].copy_from_slice(&edid::generate(
            self.preferred.width,
            self.preferred.height,
            &self.display,
        ));
        Self::respond(vdev, chain, cmd, &mut resp);
    }

    /// QEMU `virtio_gpu_resource_create_2d`.
    fn create_2d(&mut self, id: u32, format: u32, width: u32, height: u32) -> Reply {
        if id == 0 || self.resource(id).is_some() {
            return Reply::Error(RESP_ERR_INVALID_RESOURCE_ID);
        }
        let Some(format) = Format::from_wire(format) else {
            return Reply::Error(RESP_ERR_INVALID_PARAMETER);
        };
        // calc_image_hostmem: 32 bits a pixel, rows of whole words.
        let bytes = u64::from(height) * u64::from(width) * 4;
        if bytes > u64::from(u32::MAX) || bytes + self.hostmem >= MAX_HOSTMEM || bytes == 0 {
            return Reply::Error(RESP_ERR_OUT_OF_MEMORY);
        }
        self.hostmem += bytes;
        self.resources.push(Resource {
            id,
            format,
            width,
            height,
            pixels: vec![0; bytes as usize],
            backing: Vec::new(),
        });
        Reply::Done
    }

    /// QEMU `virtio_gpu_disable_scanout`.
    fn disable_scanout(&mut self) {
        if self.scanout.resource_id != 0 {
            self.resized = true;
        }
        self.scanout = Scanout::default();
    }

    /// QEMU `virtio_gpu_resource_unref` and `virtio_gpu_resource_destroy`.
    fn unref(&mut self, id: u32) -> Reply {
        let Some(index) = self.resource_index(id) else {
            return Reply::Error(RESP_ERR_INVALID_RESOURCE_ID);
        };
        if self.scanout.resource_id == id {
            self.disable_scanout();
        }
        let res = self.resources.swap_remove(index);
        self.hostmem -= res.pixels.len() as u64;
        Reply::Done
    }

    /// QEMU `virtio_gpu_set_scanout` and `virtio_gpu_do_set_scanout`.
    fn set_scanout(&mut self, rect: Rect, scanout_id: u32, resource_id: u32) -> Reply {
        if scanout_id != 0 {
            return Reply::Error(RESP_ERR_INVALID_SCANOUT_ID);
        }
        if resource_id == 0 {
            self.disable_scanout();
            return Reply::Done;
        }
        let res = match self.backed_resource_index(resource_id) {
            Ok(index) => &self.resources[index],
            Err(reply) => return reply,
        };
        // virtio_gpu_check_scanout_bounds.
        if rect.width < 16 || rect.height < 16 || !rect.within(res.width, res.height) {
            return Reply::Error(RESP_ERR_INVALID_PARAMETER);
        }
        let s = self.scanout;
        if s.resource_id == 0 || s.rect.width != rect.width || s.rect.height != rect.height {
            self.resized = true;
        }
        self.scanout = Scanout { resource_id, rect };
        self.dirty = Rect {
            x: 0,
            y: 0,
            width: rect.width,
            height: rect.height,
        };
        Reply::Done
    }

    /// QEMU `virtio_gpu_resource_flush`: the part of the scanout the rect
    /// covers is ready to show.
    fn flush(&mut self, rect: Rect, resource_id: u32) -> Reply {
        let Some(res) = self.resource(resource_id) else {
            return Reply::Error(RESP_ERR_INVALID_RESOURCE_ID);
        };
        if !rect.within(res.width, res.height) {
            return Reply::Error(RESP_ERR_INVALID_PARAMETER);
        }
        if self.scanout.resource_id == resource_id {
            let s = self.scanout.rect;
            let on_screen = rect.intersect(s);
            if !on_screen.is_empty() {
                self.dirty = self.dirty.union(Rect {
                    x: on_screen.x - s.x,
                    y: on_screen.y - s.y,
                    width: on_screen.width,
                    height: on_screen.height,
                });
            }
        }
        Reply::Done
    }

    /// QEMU `virtio_gpu_transfer_to_host_2d`, its offsets 32-bit as QEMU
    /// computes them.
    fn transfer(&mut self, ram: &GuestRam<'_>, rect: Rect, offset: u64, resource_id: u32) -> Reply {
        let index = match self.backed_resource_index(resource_id) {
            Ok(index) => index,
            Err(reply) => return reply,
        };
        let res = &self.resources[index];
        if !rect.within(res.width, res.height) {
            return Reply::Error(RESP_ERR_INVALID_PARAMETER);
        }
        let stride = res.stride();
        let mut pixels = core::mem::take(&mut self.resources[index].pixels);
        let res = &self.resources[index];
        if rect.x != 0 || rect.width != res.width {
            let row = rect.width as usize * 4;
            for h in 0..rect.height {
                let src = (offset as u32).wrapping_add(stride.wrapping_mul(h));
                let dst = ((rect.y + h) * stride + rect.x * 4) as usize;
                res.read_backing(ram, u64::from(src), &mut pixels[dst..dst + row]);
            }
        } else {
            let dst = (rect.y * stride) as usize;
            let len = (stride * rect.height) as usize;
            res.read_backing(ram, u64::from(offset as u32), &mut pixels[dst..dst + len]);
        }
        self.resources[index].pixels = pixels;
        Reply::Done
    }

    /// QEMU `virtio_gpu_resource_attach_backing` and
    /// `virtio_gpu_create_mapping_iov`: the entries follow the command in
    /// the driver's buffers, and each must lie in guest RAM.
    fn attach_backing(
        &mut self,
        vdev: &Vdev<'_, '_>,
        chain: &Chain,
        resource_id: u32,
        entries: u32,
    ) -> Reply {
        let Some(index) = self.resource_index(resource_id) else {
            return Reply::Error(RESP_ERR_INVALID_RESOURCE_ID);
        };
        if !self.resources[index].backing.is_empty() || entries > MAX_BACKING_ENTRIES {
            return Reply::Error(RESP_ERR_UNSPEC);
        }
        let mut raw = vec![0u8; entries as usize * 16];
        if vdev.read(chain, 32, &mut raw) != raw.len() {
            return Reply::Error(RESP_ERR_UNSPEC);
        }
        let mut backing = Vec::with_capacity(entries as usize);
        for entry in raw.chunks_exact(16) {
            let (addr, len) = (le64(entry, 0), le32(entry, 8));
            if len == 0 || !vdev.ram().contains(addr, u64::from(len)) {
                return Reply::Error(RESP_ERR_UNSPEC);
            }
            backing.push(BackingEntry { addr, len });
        }
        self.resources[index].backing = backing;
        Reply::Done
    }

    /// QEMU `virtio_gpu_resource_detach_backing`.
    fn detach_backing(&mut self, resource_id: u32) -> Reply {
        let index = match self.backed_resource_index(resource_id) {
            Ok(index) => index,
            Err(reply) => return reply,
        };
        self.resources[index].backing.clear();
        Reply::Done
    }

    /// Show the host what changed since it last looked: the new size first,
    /// then the changed rectangle with the cursor drawn over it. While no
    /// scanout is set the display is blank at the requested size.
    pub fn refresh<D: DisplaySink>(&mut self, sink: &mut D) -> Refreshed {
        let Some(res_index) = self.resource_index(self.scanout.resource_id) else {
            if self.resized {
                self.resized = false;
                sink.dimension_update(Dimensions {
                    width: self.preferred.width,
                    height: self.preferred.height,
                    font_width: 0,
                    font_height: 0,
                    bits_per_pixel: 32,
                });
                sink.clear_screen();
                sink.flush();
                return Refreshed::Frame;
            }
            return Refreshed::Unchanged;
        };
        let s = self.scanout.rect;
        if self.resized {
            self.resized = false;
            sink.dimension_update(Dimensions {
                width: s.width,
                height: s.height,
                font_width: 0,
                font_height: 0,
                bits_per_pixel: 32,
            });
            self.dirty = Rect {
                x: 0,
                y: 0,
                width: s.width,
                height: s.height,
            };
        }
        let rect = self.dirty.intersect(Rect {
            x: 0,
            y: 0,
            width: s.width,
            height: s.height,
        });
        self.dirty = Rect::default();
        if rect.is_empty() {
            return Refreshed::Unchanged;
        }
        let res = &self.resources[res_index];
        let stride = res.stride() as usize;
        let cursor = self.cursor_rect();
        let origin = self.cursor_origin();
        self.frame.clear();
        self.frame
            .reserve(rect.width as usize * rect.height as usize * 4);
        for row in rect.y..rect.y + rect.height {
            let src_row = (s.y + row) as usize * stride;
            for col in rect.x..rect.x + rect.width {
                let at = src_row + (s.x + col) as usize * 4;
                let p = &res.pixels[at..at + 4];
                let mut rgba = res.format.rgba([p[0], p[1], p[2], p[3]]);
                rgba[3] = 0xFF;
                let in_cursor = !cursor.is_empty()
                    && col >= cursor.x
                    && u64::from(col) < u64::from(cursor.x) + u64::from(cursor.width)
                    && row >= cursor.y
                    && u64::from(row) < u64::from(cursor.y) + u64::from(cursor.height);
                if in_cursor {
                    let image_row = i64::from(row) - origin.top;
                    let image_col = i64::from(col) - origin.left;
                    let raw = self.cursor.image
                        [(image_row * i64::from(CURSOR_SIZE) + image_col) as usize];
                    let c = Format::B8G8R8A8.rgba(raw);
                    // The cursor's colours are premultiplied by its alpha.
                    let a = u16::from(c[3]);
                    for i in 0..3 {
                        rgba[i] =
                            (u16::from(c[i]) + u16::from(rgba[i]) * (255 - a) / 255).min(255) as u8;
                    }
                }
                self.frame.extend_from_slice(&rgba);
            }
        }
        sink.graphics_tile_update(
            &self.frame,
            TilePos {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            },
        );
        sink.flush();
        Refreshed::Frame
    }

    pub fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u32(self.preferred.width)?;
        w.write_u32(self.preferred.height)?;
        w.write_bool(self.output == Output::Enabled)?;
        w.write_u32(self.events_read)?;
        w.write_u32(self.resources.len() as u32)?;
        for res in &self.resources {
            w.write_u32(res.id)?;
            w.write_u32(res.format.to_wire())?;
            w.write_u32(res.width)?;
            w.write_u32(res.height)?;
            w.write_bytes(&res.pixels)?;
            w.write_u32(res.backing.len() as u32)?;
            for entry in &res.backing {
                w.write_u64(entry.addr)?;
                w.write_u32(entry.len)?;
            }
        }
        w.write_u32(self.scanout.resource_id)?;
        for v in [
            self.scanout.rect.x,
            self.scanout.rect.y,
            self.scanout.rect.width,
            self.scanout.rect.height,
        ] {
            w.write_u32(v)?;
        }
        w.write_u32(self.cursor.image.len() as u32)?;
        for p in &self.cursor.image {
            w.write_bytes(p)?;
        }
        for v in [
            self.cursor.hot_x,
            self.cursor.hot_y,
            self.cursor.x,
            self.cursor.y,
        ] {
            w.write_u32(v)?;
        }
        w.write_bool(self.cursor.shown)
    }

    pub fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let width = r.read_u32()?;
        let height = r.read_u32()?;
        self.preferred = Resolution::new(width, height);
        self.output = if r.read_bool()? {
            Output::Enabled
        } else {
            Output::Disabled
        };
        self.events_read = r.read_u32()?;
        let count = r.read_count(1 << 16)?;
        self.resources.clear();
        self.hostmem = 0;
        for _ in 0..count {
            let id = r.read_u32()?;
            let format = Format::from_wire(r.read_u32()?).ok_or(SnapError::Invalid(
                "virtio-gpu resource in an unknown format",
            ))?;
            let width = r.read_u32()?;
            let height = r.read_u32()?;
            let bytes = u64::from(width) * u64::from(height) * 4;
            if id == 0
                || self.resource(id).is_some()
                || bytes == 0
                || self.hostmem + bytes >= MAX_HOSTMEM
            {
                return Err(SnapError::Invalid(
                    "virtio-gpu resource the device could not have made",
                ));
            }
            let mut pixels = vec![0u8; bytes as usize];
            r.read_bytes(&mut pixels)?;
            let entries = r.read_count(MAX_BACKING_ENTRIES as usize)?;
            let mut backing = Vec::with_capacity(entries);
            for _ in 0..entries {
                let addr = r.read_u64()?;
                let len = r.read_u32()?;
                if len == 0 {
                    return Err(SnapError::Invalid("virtio-gpu backing entry of no bytes"));
                }
                backing.push(BackingEntry { addr, len });
            }
            self.hostmem += bytes;
            self.resources.push(Resource {
                id,
                format,
                width,
                height,
                pixels,
                backing,
            });
        }
        self.scanout.resource_id = r.read_u32()?;
        self.scanout.rect = Rect {
            x: r.read_u32()?,
            y: r.read_u32()?,
            width: r.read_u32()?,
            height: r.read_u32()?,
        };
        let cursor_pixels = r.read_count(CURSOR_PIXELS)?;
        if cursor_pixels != 0 && cursor_pixels != CURSOR_PIXELS {
            return Err(SnapError::Invalid(
                "virtio-gpu cursor that is neither 64 by 64 nor undefined",
            ));
        }
        self.cursor.image.clear();
        for _ in 0..cursor_pixels {
            let mut p = [0u8; 4];
            r.read_bytes(&mut p)?;
            self.cursor.image.push(p);
        }
        self.cursor.hot_x = r.read_u32()?;
        self.cursor.hot_y = r.read_u32()?;
        self.cursor.x = r.read_u32()?;
        self.cursor.y = r.read_u32()?;
        self.cursor.shown = r.read_bool()?;
        let shown = self.scanout.resource_id == 0
            || self.resource(self.scanout.resource_id).is_some_and(|res| {
                let rect = self.scanout.rect;
                rect.width >= 16 && rect.height >= 16 && rect.within(res.width, res.height)
            });
        if !shown {
            return Err(SnapError::Invalid(
                "virtio-gpu scanout the device could not have set",
            ));
        }
        self.cmdq.clear();
        self.resized = true;
        self.dirty = Rect::default();
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/virtio_gpu.rs"]
mod tests;
