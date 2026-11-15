//! The EDID a virtio-gpu reports for its display.
//!
//! Provenance (R7): VESA E-EDID 1.4 (Release A, Revision 2,
//! `cpp_orig/refs/VESA-EEDID-A2.pdf`) lays out the base block and fixes
//! what it fixes. Where it leaves a value to the display's maker, the
//! value is the board's own, each taken from a published standard:
//!
//! - every mode is at 60 Hz ([`FIELD_RATE_HZ`]): the base video mode's rate
//!   (640 × 480 at 60 Hz, 3.8), the rate VESA DMT has a timing of nearly
//!   every mode at, and the one VESA CVT 1.2 defines reduced blanking for;
//! - the preferred mode is the display's own size with CVT 1.2's
//!   reduced-blanking timing ([`cvt_reduced_blanking`]), in the first
//!   18-byte descriptor, as 3.10.1 requires;
//! - the other modes are the VESA DMT modes at 60 Hz that fit within the
//!   display, and the base video mode always ([`BASE_VIDEO_MODE`],
//!   [`DMT_MODES`]), each named by the code E-EDID has for it:
//!   Established Timings I & II (3.8), Established Timings III
//!   (3.10.3.9), else a standard timing (3.9);
//! - the physical size is the size at 96 pixels an inch
//!   ([`PIXELS_PER_INCH`]), rounded to the nearest unit (3.6.2);
//! - the colour space is sRGB (IEC 61966-2-1): its primaries and D65 white
//!   point, its gamma of 2.2, and the feature bit that says so (3.6.3,
//!   3.6.4 note 3, 3.7);
//! - the input is digital at 8 bits a primary, the depth of every
//!   virtio-gpu format (virtio 1.2, 5.7.6.8), with no interface named, as
//!   a virtual display is attached through none (Table 3.11);
//! - a Display Range Limits descriptor holds exactly the listed modes
//!   (3.10.3.3, "Range Limits Only"), for a display that takes those modes
//!   and no others (3.6.4 bit 0 clear);
//! - the descriptors are the preferred timing, Established Timings III,
//!   the range limits and the display's name, in that order, each only
//!   when it has something to say, and dummy descriptors after them
//!   (3.10, Table 3.20 notes 1 and 3);
//! - an extension block is added only for a preferred mode no detailed
//!   timing descriptor can hold: a VESA DisplayID 1.3 section with a
//!   Type I detailed timing ([`displayid_block`]). The first descriptor
//!   then holds the largest listed mode and the feature byte says it is
//!   not the native one (3.6.4 note 4).
//!
//! What names the display — its maker, product, name and date of
//! manufacture — is an input, the board's own
//! ([`crate::identity::BoardIdentity`]).

use crate::identity::DisplayIdentity;

/// Bytes of EDID a virtio-gpu response carries (`struct
/// virtio_gpu_resp_edid`'s `edid`).
pub const EDID_LEN: usize = 1024;

/// An EDID block (VESA E-EDID 1.4 section 2.2): the base block and each
/// extension block.
pub const BLOCK_LEN: usize = 128;

/// Where in the base block the count of extension blocks that follow it
/// is kept (E-EDID 1.4, byte 7Eh).
pub const EXTENSION_COUNT_AT: usize = 126;

/// An 18-byte descriptor (E-EDID 1.4, 3.10).
const DESCRIPTOR_LEN: usize = 18;

/// Where the base block's four 18-byte descriptors start (36h).
const DESCRIPTORS_AT: usize = 0x36;

/// Where the base block's eight standard timings start (26h, Table 3.19).
const STANDARD_TIMINGS_AT: usize = 0x26;

/// How many standard timings the base block holds.
const STANDARD_TIMINGS: usize = 8;

/// The field rate of every mode the display lists, in Hz.
const FIELD_RATE_HZ: u64 = 60;

/// The display's density, in pixels an inch: CSS's reference pixel, of
/// which an inch holds 96 (W3C CSS Values and Units, absolute lengths),
/// the density desktop software draws at unscaled.
const PIXELS_PER_INCH: u64 = 96;

/// Which way a sync pulse goes, outside the other direction's sync
/// (E-EDID 1.4, Table 3.22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Polarity {
    Negative,
    Positive,
}

impl Polarity {
    const fn bit(self) -> u8 {
        match self {
            Polarity::Negative => 0,
            Polarity::Positive => 1,
        }
    }
}

/// One direction of a video timing (E-EDID 1.4, 3.12): the addressable
/// pixels or lines, then the front porch, the sync pulse and the back
/// porch, `total` counting all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Axis {
    active: u32,
    front_porch: u32,
    sync: u32,
    total: u32,
    polarity: Polarity,
}

impl Axis {
    /// From the four counts a VESA DMT table gives: addressable, sync
    /// start, sync end and total.
    const fn dmt(counts: [u32; 4], polarity: Polarity) -> Self {
        let [active, sync_start, sync_end, total] = counts;
        Axis {
            active,
            front_porch: sync_start - active,
            sync: sync_end - sync_start,
            total,
            polarity,
        }
    }

    const fn blank(&self) -> u32 {
        self.total - self.active
    }
}

/// A video timing: the pixel clock and both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Timing {
    clock_khz: u64,
    horizontal: Axis,
    vertical: Axis,
}

impl Timing {
    /// The clock in a detailed timing descriptor's unit, 10 kHz, to the
    /// nearest (DMT's 25.175 MHz is 25.18 MHz).
    const fn clock_10khz(&self) -> u64 {
        (self.clock_khz + 5) / 10
    }

    const fn area(&self) -> u64 {
        self.horizontal.active as u64 * self.vertical.active as u64
    }
}

/// A DMT timing at 60 Hz: its clock in kHz, its horizontal and vertical
/// counts (addressable, sync start, sync end, total) and polarities.
const fn dmt(clock_khz: u64, h: [u32; 4], v: [u32; 4], hpol: Polarity, vpol: Polarity) -> Timing {
    Timing {
        clock_khz,
        horizontal: Axis::dmt(h, hpol),
        vertical: Axis::dmt(v, vpol),
    }
}

/// How E-EDID 1.4 names a DMT mode without a detailed timing.
#[derive(Debug, Clone, Copy)]
enum Code {
    /// A bit of Established Timings I & II (3.8, Table 3.18): the byte,
    /// 23h or 24h, and the bit in it.
    Established { at: usize, bit: u8 },
    /// A bit of Established Timings III (3.10.3.9, Table 3.36): the byte
    /// of the descriptor and the bit in it.
    EstablishedIii { byte: usize, bit: u8 },
    /// A standard timing's two bytes, as DMT lists them (3.9).
    Standard([u8; 2]),
}

/// A mode the display lists: its DMT timing and its code.
struct DmtMode {
    timing: Timing,
    code: Code,
}

use Polarity::{Negative as N, Positive as P};

/// The base video mode, 640 × 480 at 60 Hz (DMT 04h), which every Plug &
/// Play display lists in Established Timing I (E-EDID 1.4, 3.8), whatever
/// its size.
const BASE_VIDEO_MODE: DmtMode = DmtMode {
    timing: dmt(25_175, [640, 656, 752, 800], [480, 490, 492, 525], N, N),
    code: Code::Established { at: 0x23, bit: 5 },
};

/// The other VESA DMT (1.0, Revision 13) modes at 60 Hz that E-EDID 1.4
/// names without a detailed timing, each listed when it fits within the
/// display: the Established Timings I & II and III bits at 60 Hz (normal
/// blanking where DMT has both), and the standard timings DMT gives a code
/// of at 60 Hz that neither names (up to 2288 pixels wide, Table 3.19
/// note 3). Timings in DMT's own counts, its ID in each comment.
const DMT_MODES: [DmtMode; 19] = [
    // 09h 800 × 600
    DmtMode {
        timing: dmt(40_000, [800, 840, 968, 1056], [600, 601, 605, 628], P, P),
        code: Code::Established { at: 0x23, bit: 0 },
    },
    // 0Eh 848 × 480
    DmtMode {
        timing: dmt(33_750, [848, 864, 976, 1088], [480, 486, 494, 517], P, P),
        code: Code::EstablishedIii { byte: 6, bit: 3 },
    },
    // 10h 1024 × 768
    DmtMode {
        timing: dmt(65_000, [1024, 1048, 1184, 1344], [768, 771, 777, 806], N, N),
        code: Code::Established { at: 0x24, bit: 3 },
    },
    // 17h 1280 × 768
    DmtMode {
        timing: dmt(79_500, [1280, 1344, 1472, 1664], [768, 771, 778, 798], N, P),
        code: Code::EstablishedIii { byte: 7, bit: 6 },
    },
    // 1Ch 1280 × 800
    DmtMode {
        timing: dmt(83_500, [1280, 1352, 1480, 1680], [800, 803, 809, 831], N, P),
        code: Code::Standard([0x81, 0x00]),
    },
    // 20h 1280 × 960
    DmtMode {
        timing: dmt(
            108_000,
            [1280, 1376, 1488, 1800],
            [960, 961, 964, 1000],
            P,
            P,
        ),
        code: Code::EstablishedIii { byte: 7, bit: 3 },
    },
    // 23h 1280 × 1024
    DmtMode {
        timing: dmt(
            108_000,
            [1280, 1328, 1440, 1688],
            [1024, 1025, 1028, 1066],
            P,
            P,
        ),
        code: Code::EstablishedIii { byte: 7, bit: 1 },
    },
    // 27h 1360 × 768
    DmtMode {
        timing: dmt(85_500, [1360, 1424, 1536, 1792], [768, 771, 777, 795], P, P),
        code: Code::EstablishedIii { byte: 8, bit: 7 },
    },
    // 2Ah 1400 × 1050
    DmtMode {
        timing: dmt(
            121_750,
            [1400, 1488, 1632, 1864],
            [1050, 1053, 1057, 1089],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 8, bit: 1 },
    },
    // 2Fh 1440 × 900
    DmtMode {
        timing: dmt(
            106_500,
            [1440, 1520, 1672, 1904],
            [900, 903, 909, 934],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 8, bit: 5 },
    },
    // 33h 1600 × 1200
    DmtMode {
        timing: dmt(
            162_000,
            [1600, 1664, 1856, 2160],
            [1200, 1201, 1204, 1250],
            P,
            P,
        ),
        code: Code::EstablishedIii { byte: 9, bit: 2 },
    },
    // 3Ah 1680 × 1050
    DmtMode {
        timing: dmt(
            146_250,
            [1680, 1784, 1960, 2240],
            [1050, 1053, 1059, 1089],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 9, bit: 5 },
    },
    // 3Eh 1792 × 1344
    DmtMode {
        timing: dmt(
            204_750,
            [1792, 1920, 2120, 2448],
            [1344, 1345, 1348, 1394],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 10, bit: 5 },
    },
    // 41h 1856 × 1392
    DmtMode {
        timing: dmt(
            218_250,
            [1856, 1952, 2176, 2528],
            [1392, 1393, 1396, 1439],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 10, bit: 3 },
    },
    // 45h 1920 × 1200
    DmtMode {
        timing: dmt(
            193_250,
            [1920, 2056, 2256, 2592],
            [1200, 1203, 1209, 1245],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 10, bit: 0 },
    },
    // 49h 1920 × 1440
    DmtMode {
        timing: dmt(
            234_000,
            [1920, 2048, 2256, 2600],
            [1440, 1441, 1444, 1500],
            N,
            P,
        ),
        code: Code::EstablishedIii { byte: 11, bit: 5 },
    },
    // 52h 1920 × 1080
    DmtMode {
        timing: dmt(
            148_500,
            [1920, 2008, 2052, 2200],
            [1080, 1084, 1089, 1125],
            P,
            P,
        ),
        code: Code::Standard([0xD1, 0xC0]),
    },
    // 53h 1600 × 900, reduced blanking
    DmtMode {
        timing: dmt(
            108_000,
            [1600, 1624, 1704, 1800],
            [900, 901, 904, 1000],
            P,
            P,
        ),
        code: Code::Standard([0xA9, 0xC0]),
    },
    // 54h 2048 × 1152, reduced blanking
    DmtMode {
        timing: dmt(
            162_000,
            [2048, 2074, 2154, 2250],
            [1152, 1153, 1156, 1200],
            P,
            P,
        ),
        code: Code::Standard([0xE1, 0xC0]),
    },
];

/// Whether a detailed timing descriptor's fields hold `t` (E-EDID 1.4,
/// Table 3.21 notes 2 to 10).
const fn fits_detailed_timing(t: &Timing) -> bool {
    let clock = t.clock_10khz();
    let (h, v) = (&t.horizontal, &t.vertical);
    clock >= 1
        && clock <= 0xFFFF
        && h.active <= 0xFFF
        && h.blank() <= 0xFFF
        && v.active <= 0xFFF
        && v.blank() <= 0xFFF
        && h.front_porch <= 0x3FF
        && h.sync <= 0x3FF
        && v.front_porch <= 0x3F
        && v.sync <= 0x3F
}

/// Every listed mode has a detailed timing, so the largest of them can
/// stand first when the preferred mode cannot; and the standard timings
/// fit the base block's eight.
const _: () = {
    assert!(fits_detailed_timing(&BASE_VIDEO_MODE.timing));
    let mut i = 0;
    let mut standard = 0;
    while i < DMT_MODES.len() {
        assert!(fits_detailed_timing(&DMT_MODES[i].timing));
        if let Code::Standard(_) = DMT_MODES[i].code {
            standard += 1;
        }
        i += 1;
    }
    assert!(standard <= STANDARD_TIMINGS);
};

/// CVT 1.2's reduced-blanking constants (its first reduced blanking,
/// "RB"): the horizontal blank, sync and front porch in pixels, the least
/// vertical blanking time in µs, the vertical front porch and least back
/// porch in lines, and the clock's step in kHz.
const RB_H_BLANK: u64 = 160;
const RB_H_SYNC: u32 = 32;
const RB_H_FRONT_PORCH: u32 = 48;
const RB_MIN_V_BLANK_US: u64 = 460;
const RB_V_FRONT_PORCH: u64 = 3;
const RB_MIN_V_BACK_PORCH: u64 = 6;
const CVT_CLOCK_STEP_KHZ: u64 = 250;

/// The most pixels or lines any description here can give a mode, a
/// DisplayID Type I timing's 16 bits plus one.
const MAX_ACTIVE: u32 = 0x1_0000;

/// VESA CVT 1.2's reduced-blanking timing of a `width` by `height` image
/// at [`FIELD_RATE_HZ`], or `None` for an image of no pixels or one larger
/// than any description holds. The width is the display's own, not
/// rounded down to CVT's 8-pixel character cell: the preferred mode is the
/// display's size exactly.
fn cvt_reduced_blanking(width: u32, height: u32) -> Option<Timing> {
    if width == 0 || height == 0 || width > MAX_ACTIVE || height > MAX_ACTIVE {
        return None;
    }
    let (w, h) = (u64::from(width), u64::from(height));
    // V_SYNC_RND, by the image's aspect ratio; 10 for any other.
    let v_sync = if w * 3 == h * 4 {
        4
    } else if w * 9 == h * 16 {
        5
    } else if w * 10 == h * 16 {
        6
    } else if w * 4 == h * 5 || w * 9 == h * 15 {
        7
    } else {
        10
    };
    // VBI_LINES: RB_MIN_V_BLANK over H_PERIOD_EST, rounded down, plus one,
    // where H_PERIOD_EST = (1 / V_FIELD_RATE - RB_MIN_V_BLANK) / V_LINES;
    // at least RB_V_FPORCH + V_SYNC_RND + MIN_V_BPORCH.
    let vbi = (RB_MIN_V_BLANK_US * h * FIELD_RATE_HZ
        / (1_000_000 - RB_MIN_V_BLANK_US * FIELD_RATE_HZ)
        + 1)
    .max(RB_V_FRONT_PORCH + v_sync + RB_MIN_V_BACK_PORCH);
    let v_total = h + vbi;
    let h_total = w + RB_H_BLANK;
    // ACT_PIXEL_FREQ: the clock the totals need at the field rate, rounded
    // down to CLOCK_STEP; a mode so small it rounds to nothing takes one
    // step.
    let clock_khz = (FIELD_RATE_HZ * h_total * v_total / (CVT_CLOCK_STEP_KHZ * 1000)
        * CVT_CLOCK_STEP_KHZ)
        .max(CVT_CLOCK_STEP_KHZ);
    Some(Timing {
        clock_khz,
        horizontal: Axis {
            active: width,
            front_porch: RB_H_FRONT_PORCH,
            sync: RB_H_SYNC,
            total: u32::try_from(h_total).ok()?,
            polarity: Polarity::Positive,
        },
        vertical: Axis {
            active: height,
            front_porch: RB_V_FRONT_PORCH as u32,
            sync: v_sync as u32,
            total: u32::try_from(v_total).ok()?,
            polarity: Polarity::Negative,
        },
    })
}

/// A displayed image's size in mm (E-EDID 1.4, Table 3.21 bytes 12 to 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImageMm {
    width: u64,
    height: u64,
}

/// `px` pixels at [`PIXELS_PER_INCH`], in mm to the nearest.
const fn mm(px: u32) -> u64 {
    (px as u64 * 254 + 5 * PIXELS_PER_INCH) / (10 * PIXELS_PER_INCH)
}

/// `px` pixels at [`PIXELS_PER_INCH`], in cm to the nearest.
const fn cm(px: u32) -> u64 {
    (px as u64 * 254 + 50 * PIXELS_PER_INCH) / (100 * PIXELS_PER_INCH)
}

/// The most a detailed timing descriptor's image size gives, in mm.
const MAX_IMAGE_MM: u64 = 0xFFF;

/// Bytes 15h and 16h (E-EDID 1.4, 3.6.2, Table 3.12): the screen's size in
/// cm where both are 1 to 255 cm, else its aspect ratio where that is
/// 3.54 : 1 or less either way up, else unknown, as for a display of no
/// pixels.
fn screen_size(width: u32, height: u32) -> [u8; 2] {
    if width == 0 || height == 0 {
        return [0, 0];
    }
    if let (Ok(w @ 1..=u8::MAX), Ok(h @ 1..=u8::MAX)) =
        (u8::try_from(cm(width)), u8::try_from(cm(height)))
    {
        return [w, h];
    }
    // Stored Value = Aspect Ratio × 100 − 99, landscape; 100 ÷ Aspect
    // Ratio − 99, portrait; the ratio rounded to the hundredth.
    let (long, short) = (u64::from(width.max(height)), u64::from(width.min(height)));
    match u8::try_from((200 * long + short) / (2 * short) - 99) {
        Ok(stored) if width >= height => [stored, 0],
        Ok(stored) => [0, stored],
        Err(_) => [0, 0],
    }
}

/// The image a mode not the display's own makes, scaled to fill the screen
/// at its own aspect ratio (E-EDID 1.4, Table 3.21 notes 15 and 16), within
/// what the descriptor can give; none on a display of no size (note 17).
fn fitted_image(t: &Timing, width: u32, height: u32) -> ImageMm {
    if width == 0 || height == 0 {
        return ImageMm {
            width: 0,
            height: 0,
        };
    }
    let (sw, sh) = (mm(width).min(MAX_IMAGE_MM), mm(height).min(MAX_IMAGE_MM));
    let (mw, mh) = (u64::from(t.horizontal.active), u64::from(t.vertical.active));
    if mw * sh >= mh * sw {
        ImageMm {
            width: sw,
            height: (2 * sw * mh + mw) / (2 * mw),
        }
    } else {
        ImageMm {
            width: (2 * sh * mw + mh) / (2 * mh),
            height: sh,
        }
    }
}

/// A detailed timing descriptor (E-EDID 1.4, 3.10.2, Tables 3.21 and
/// 3.22) for `t` at `image`: no borders, not interlaced, no stereo,
/// digital separate sync. The caller has checked `t` with
/// [`fits_detailed_timing`] and kept `image` within [`MAX_IMAGE_MM`].
fn detailed_timing(t: &Timing, image: ImageMm) -> [u8; DESCRIPTOR_LEN] {
    let (h, v) = (&t.horizontal, &t.vertical);
    let (hblank, vblank) = (h.blank(), v.blank());
    let mut d = [0u8; DESCRIPTOR_LEN];
    d[..2].copy_from_slice(&(t.clock_10khz() as u16).to_le_bytes());
    d[2] = h.active as u8;
    d[3] = hblank as u8;
    d[4] = (((h.active >> 8) << 4) | (hblank >> 8)) as u8;
    d[5] = v.active as u8;
    d[6] = vblank as u8;
    d[7] = (((v.active >> 8) << 4) | (vblank >> 8)) as u8;
    d[8] = h.front_porch as u8;
    d[9] = h.sync as u8;
    d[10] = (((v.front_porch & 0xF) << 4) | (v.sync & 0xF)) as u8;
    d[11] = (((h.front_porch >> 8) << 6)
        | ((h.sync >> 8) << 4)
        | ((v.front_porch >> 4) << 2)
        | (v.sync >> 4)) as u8;
    d[12] = image.width as u8;
    d[13] = image.height as u8;
    d[14] = (((image.width >> 8) << 4) | (image.height >> 8)) as u8;
    d[17] = 0x18 | (v.polarity.bit() << 2) | (h.polarity.bit() << 1);
    d
}

/// The first five bytes of a display descriptor with `tag` (E-EDID 1.4,
/// Table 3.23), the rest zero; with tag 10h, the whole of a dummy
/// descriptor (3.10.3.11).
const fn display_descriptor(tag: u8) -> [u8; DESCRIPTOR_LEN] {
    let mut d = [0u8; DESCRIPTOR_LEN];
    d[3] = tag;
    d
}

/// A string descriptor (VESA E-EDID 1.4, 3.10.3.1 to 3.10.3.4): up to 13
/// characters, a shorter string ended by a line feed and padded with
/// spaces.
fn text_descriptor(tag: u8, text: &[u8]) -> [u8; DESCRIPTOR_LEN] {
    let mut d = display_descriptor(tag);
    d[5..].fill(b' ');
    let len = text.len().min(13);
    d[5..5 + len].copy_from_slice(&text[..len]);
    if len < 13 {
        d[5 + len] = b'\n';
    }
    d
}

/// The least and most of a rate over the listed modes, each rounded
/// outward to a whole unit.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    min: u64,
    max: u64,
}

impl Bounds {
    fn include(&mut self, numerator: u64, denominator: u64) {
        self.min = self.min.min(numerator / denominator);
        self.max = self.max.max(numerator.div_ceil(denominator));
    }
}

/// What a Display Range Limits descriptor says of the listed modes: their
/// field rates in Hz, line rates in kHz and the fastest clock.
#[derive(Debug, Clone, Copy)]
struct Ranges {
    field_hz: Bounds,
    line_khz: Bounds,
    max_clock_khz: u64,
}

impl Ranges {
    const NONE: Ranges = Ranges {
        field_hz: Bounds {
            min: u64::MAX,
            max: 0,
        },
        line_khz: Bounds {
            min: u64::MAX,
            max: 0,
        },
        max_clock_khz: 0,
    };

    fn include(&mut self, t: &Timing) {
        let h_total = u64::from(t.horizontal.total);
        let frame = h_total * u64::from(t.vertical.total);
        self.field_hz.include(t.clock_khz * 1000, frame);
        self.line_khz.include(t.clock_khz, h_total);
        self.max_clock_khz = self.max_clock_khz.max(t.clock_khz);
    }
}

/// A rate as a range limits byte (E-EDID 1.4, Table 3.26): 1 to 255 as it
/// is, 256 to 510 less the 255 its offset flag adds.
#[derive(Debug, Clone, Copy)]
enum RateByte {
    Plain(u8),
    Offset(u8),
}

impl RateByte {
    fn of(rate: u64) -> Option<RateByte> {
        match rate {
            1..=255 => Some(RateByte::Plain(rate as u8)),
            256..=510 => Some(RateByte::Offset((rate - 255) as u8)),
            _ => None,
        }
    }

    const fn byte(self) -> u8 {
        match self {
            RateByte::Plain(b) | RateByte::Offset(b) => b,
        }
    }

    const fn offset(self) -> u8 {
        match self {
            RateByte::Plain(_) => 0,
            RateByte::Offset(_) => 1,
        }
    }
}

/// A Display Range Limits descriptor, "Range Limits Only" (E-EDID 1.4,
/// 3.10.3.3, Table 3.26), bounding the listed modes (note 7), the clock
/// rounded up to a multiple of 10 MHz; `None` where a field cannot hold
/// a bound, the descriptor being optional.
fn range_limits(r: &Ranges) -> Option<[u8; DESCRIPTOR_LEN]> {
    let min_v = RateByte::of(r.field_hz.min)?;
    let max_v = RateByte::of(r.field_hz.max)?;
    let min_h = RateByte::of(r.line_khz.min)?;
    let max_h = RateByte::of(r.line_khz.max)?;
    let clock = match r.max_clock_khz.div_ceil(10_000) {
        c @ 1..=255 => c as u8,
        _ => return None,
    };
    let mut d = display_descriptor(0xFD);
    // A minimum is no more than its maximum, so a minimum is offset only
    // with its maximum: flags 00b, 10b or 11b.
    d[4] = (max_h.offset() << 3) | (min_h.offset() << 2) | (max_v.offset() << 1) | min_v.offset();
    d[5] = min_v.byte();
    d[6] = max_v.byte();
    d[7] = min_h.byte();
    d[8] = max_h.byte();
    d[9] = clock;
    d[10] = 0x01;
    d[11] = b'\n';
    d[12..].fill(b' ');
    Some(d)
}

/// The DisplayID 1.3 aspect ratio code of a `width` by `height` image
/// (Type I timing, byte 3 bits 3 to 0), 8 for one it does not name.
fn displayid_aspect(width: u32, height: u32) -> u8 {
    const RATIOS: [[u64; 2]; 8] = [
        [1, 1],
        [5, 4],
        [4, 3],
        [15, 9],
        [16, 9],
        [16, 10],
        [64, 27],
        [256, 135],
    ];
    let (w, h) = (u64::from(width), u64::from(height));
    RATIOS
        .iter()
        .position(|&[rw, rh]| w * rh == h * rw)
        .map_or(8, |code| code as u8)
}

/// The DisplayID extension block's tag (VESA E-EDID 1.4 extension tags,
/// DisplayID 1.3 Appendix B).
const DISPLAYID_EXTENSION_TAG: u8 = 0x70;

/// DisplayID's version 1.3.
const DISPLAYID_VERSION: u8 = 0x13;

/// The display product type the section describes: a standalone display
/// device (DisplayID 1.3, product type identifier 3).
const DISPLAYID_STANDALONE_DISPLAY: u8 = 0x03;

/// The Type I detailed timing data block's tag (DisplayID 1.3, 4.4.1).
const DISPLAYID_TYPE_I_TIMING: u8 = 0x03;

/// A Type I detailed timing's length.
const DISPLAYID_TYPE_I_LEN: usize = 20;

/// An extension block holding a DisplayID 1.3 section whose one data block
/// is `t` as a Type I detailed timing (4.4.1), marked preferred: every
/// count and the clock (in 10 kHz) stored less one, each sync's polarity
/// in its offset's top bit. `None` where a field cannot hold `t`.
fn displayid_block(t: &Timing) -> Option<[u8; BLOCK_LEN]> {
    let (h, v) = (&t.horizontal, &t.vertical);
    let less_one = |value: u32, bits: u32| {
        value
            .checked_sub(1)
            .filter(|&stored| stored < 1 << bits)
            .map(|stored| stored as u16)
    };
    let clock = t
        .clock_10khz()
        .checked_sub(1)
        .filter(|&stored| stored < 1 << 24)?;
    let fields = [
        less_one(h.active, 16)?,
        less_one(h.blank(), 16)?,
        less_one(h.front_porch, 15)? | (u16::from(h.polarity.bit()) << 15),
        less_one(h.sync, 16)?,
        less_one(v.active, 16)?,
        less_one(v.blank(), 16)?,
        less_one(v.front_porch, 15)? | (u16::from(v.polarity.bit()) << 15),
        less_one(v.sync, 16)?,
    ];

    let mut block = [0u8; BLOCK_LEN];
    block[0] = DISPLAYID_EXTENSION_TAG;
    // The section: version, the bytes of its data blocks, product type,
    // no extension sections, the data blocks and its checksum.
    let section_len = 3 + DISPLAYID_TYPE_I_LEN;
    block[1] = DISPLAYID_VERSION;
    block[2] = section_len as u8;
    block[3] = DISPLAYID_STANDALONE_DISPLAY;
    block[4] = 0;
    // The data block: tag, revision 0, its payload's length, the timing.
    block[5] = DISPLAYID_TYPE_I_TIMING;
    block[6] = 0;
    block[7] = DISPLAYID_TYPE_I_LEN as u8;
    let timing = &mut block[8..8 + DISPLAYID_TYPE_I_LEN];
    timing[..3].copy_from_slice(&(clock as u32).to_le_bytes()[..3]);
    timing[3] = 0x80 | displayid_aspect(h.active, v.active);
    for (i, field) in fields.into_iter().enumerate() {
        timing[4 + 2 * i..6 + 2 * i].copy_from_slice(&field.to_le_bytes());
    }
    let section_end = 5 + section_len;
    checksum(&mut block[1..=section_end]);
    checksum(&mut block);
    Some(block)
}

/// Make `bytes` sum to 0 mod 256 by their last (E-EDID 1.4, 3.11; DisplayID
/// 1.3's section checksum).
fn checksum(bytes: &mut [u8]) {
    let (last, summed) = match bytes.split_last_mut() {
        Some(split) => split,
        None => return,
    };
    *last = summed
        .iter()
        .fold(0u8, |s, &b| s.wrapping_add(b))
        .wrapping_neg();
}

/// sRGB's red, green and blue primaries and D65 white point, x then y, in
/// ten-thousandths (IEC 61966-2-1, the ITU-R BT.709 primaries).
const SRGB_CHROMATICITY: [u64; 8] = [6400, 3300, 3000, 6000, 1500, 600, 3127, 3290];

/// The ten-bit binary fraction nearest `ten_thousandths` (E-EDID 1.4, 3.7
/// note 3).
const fn ten_bit(ten_thousandths: u64) -> u64 {
    (ten_thousandths * 1024 + 5000) / 10_000
}

/// Bytes 19h to 22h (E-EDID 1.4, 3.7, Table 3.16): sRGB's chromaticity,
/// which the sRGB feature bit requires (3.6.4 note 3).
fn chromaticity(edid: &mut [u8]) {
    let [rx, ry, gx, gy, bx, by, wx, wy] = SRGB_CHROMATICITY.map(ten_bit);
    edid[0x19] = (((rx & 3) << 6) | ((ry & 3) << 4) | ((gx & 3) << 2) | (gy & 3)) as u8;
    edid[0x1A] = (((bx & 3) << 6) | ((by & 3) << 4) | ((wx & 3) << 2) | (wy & 3)) as u8;
    for (i, value) in [rx, ry, gx, gy, bx, by, wx, wy].into_iter().enumerate() {
        edid[0x1B + i] = (value >> 2) as u8;
    }
}

/// Video input definition (E-EDID 1.4, 3.6.1, Table 3.11): digital, 8
/// bits a primary colour, no interface named.
const VIDEO_INPUT: u8 = 0x80 | (0b010 << 4);

/// Gamma 2.2, sRGB's (E-EDID 1.4, 3.6.3: stored as gamma × 100 − 100).
const GAMMA_2_2: u8 = 120;

/// Feature support (E-EDID 1.4, 3.6.4, Table 3.14): no power management
/// states, RGB 4:4:4 alone, sRGB the default colour space, a
/// non-continuous-frequency display; and with [`FEATURE_NATIVE_PREFERRED`]
/// when the preferred timing is the display's own.
const FEATURES: u8 = 0x04;

/// Feature bit 1: the preferred timing mode includes the native pixel
/// format and preferred refresh rate.
const FEATURE_NATIVE_PREFERRED: u8 = 0x02;

/// Where the display's own mode is described.
#[derive(Debug, Clone, Copy)]
enum Native {
    /// In the first detailed timing descriptor, the preferred timing.
    FirstDescriptor(Timing),
    /// In a DisplayID extension block; the first descriptor holds the
    /// largest listed mode.
    DisplayId(Timing, [u8; BLOCK_LEN]),
    /// Nowhere: the display has no pixels, or more than any description
    /// holds.
    Undescribed,
}

/// The EDID, in `EDID_LEN` bytes, of a display whose own size is `width`
/// by `height` and which `display` describes.
pub(crate) fn generate(width: u32, height: u32, display: &DisplayIdentity) -> [u8; EDID_LEN] {
    let mut edid = [0u8; EDID_LEN];

    let fits =
        |m: &&DmtMode| m.timing.horizontal.active <= width && m.timing.vertical.active <= height;
    let listed = || core::iter::once(&BASE_VIDEO_MODE).chain(DMT_MODES.iter().filter(fits));

    let native = match cvt_reduced_blanking(width, height) {
        Some(t) if fits_detailed_timing(&t) => Native::FirstDescriptor(t),
        Some(t) => match displayid_block(&t) {
            Some(block) => Native::DisplayId(t, block),
            None => Native::Undescribed,
        },
        None => Native::Undescribed,
    };

    // Header, the display's maker and product, no serial, its date of
    // manufacture, EDID 1.4 (3.3 to 3.5).
    edid[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    let vendor = display.vendor.letters().map(|c| u16::from(c - b'@') & 0x1F);
    let vendor_id = (vendor[0] << 10) | (vendor[1] << 5) | vendor[2];
    edid[8..10].copy_from_slice(&vendor_id.to_be_bytes());
    edid[10..12].copy_from_slice(&display.product.get().to_le_bytes());
    edid[12..16].copy_from_slice(&0u32.to_le_bytes());
    edid[16] = display.date.week_byte();
    edid[17] = display.date.year_byte();
    edid[18] = 1;
    edid[19] = 4;

    // Basic display parameters and the colour space (3.6, 3.7).
    edid[0x14] = VIDEO_INPUT;
    edid[0x15..0x17].copy_from_slice(&screen_size(width, height));
    edid[0x17] = GAMMA_2_2;
    edid[0x18] = match native {
        Native::FirstDescriptor(_) => FEATURES | FEATURE_NATIVE_PREFERRED,
        Native::DisplayId(..) | Native::Undescribed => FEATURES,
    };
    chromaticity(&mut edid);

    // The listed modes, by their codes (3.8, 3.9, 3.10.3.9), and their
    // range.
    let mut established_iii = display_descriptor(0xF7);
    established_iii[5] = 0x0A;
    let mut has_established_iii = false;
    let mut ranges = Ranges::NONE;
    let mut standard = 0;
    for at in (STANDARD_TIMINGS_AT..STANDARD_TIMINGS_AT + 2 * STANDARD_TIMINGS).step_by(2) {
        edid[at..at + 2].copy_from_slice(&[0x01, 0x01]);
    }
    for mode in listed() {
        match mode.code {
            Code::Established { at, bit } => edid[at] |= 1 << bit,
            Code::EstablishedIii { byte, bit } => {
                established_iii[byte] |= 1 << bit;
                has_established_iii = true;
            }
            Code::Standard(code) => {
                let at = STANDARD_TIMINGS_AT + 2 * standard;
                edid[at..at + 2].copy_from_slice(&code);
                standard += 1;
            }
        }
        ranges.include(&mode.timing);
    }

    // The descriptors (3.10): the preferred timing, then the display
    // descriptors that have something to say, then dummies.
    let largest_listed = || {
        let largest = listed().fold(&BASE_VIDEO_MODE, |largest, mode| {
            if mode.timing.area() > largest.timing.area() {
                mode
            } else {
                largest
            }
        });
        detailed_timing(
            &largest.timing,
            fitted_image(&largest.timing, width, height),
        )
    };
    let mut descriptors = [display_descriptor(0x10); 4];
    descriptors[0] = match native {
        Native::FirstDescriptor(t) => {
            ranges.include(&t);
            detailed_timing(
                &t,
                ImageMm {
                    width: mm(width),
                    height: mm(height),
                },
            )
        }
        Native::DisplayId(t, _) => {
            ranges.include(&t);
            largest_listed()
        }
        Native::Undescribed => largest_listed(),
    };
    let mut next = 1;
    if has_established_iii {
        descriptors[next] = established_iii;
        next += 1;
    }
    if let Some(limits) = range_limits(&ranges) {
        descriptors[next] = limits;
        next += 1;
    }
    descriptors[next] = text_descriptor(0xFC, display.name.as_bytes());
    for (i, d) in descriptors.iter().enumerate() {
        let at = DESCRIPTORS_AT + i * DESCRIPTOR_LEN;
        edid[at..at + DESCRIPTOR_LEN].copy_from_slice(d);
    }

    if let Native::DisplayId(_, block) = native {
        edid[EXTENSION_COUNT_AT] = 1;
        edid[BLOCK_LEN..2 * BLOCK_LEN].copy_from_slice(&block);
    }
    checksum(&mut edid[..BLOCK_LEN]);
    edid
}

#[cfg(test)]
#[path = "../../../tests/unit/virtio_edid.rs"]
mod tests;
