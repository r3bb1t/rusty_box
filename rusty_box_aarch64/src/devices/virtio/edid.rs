//! The EDID a virtio-gpu reports for its display.
//!
//! Provenance (R7): QEMU `hw/display/edid-generate.c`, `qemu_edid_generate`
//! and the helpers it calls (`generate_timings`, `edid_fill_modes`,
//! `edid_std_mode`, `edid_desc_next`, `edid_desc_timing`,
//! `edid_desc_ranges`, `edid_desc_text`, `edid_colorspace`,
//! `init_displayid`, `qemu_displayid_generate`, `edid_checksum`), with the
//! inputs `virtio_gpu_base_generate_edid` gives it for a display with no
//! physical size, refresh rate or serial number: 75 Hz, 100 dpi, no largest
//! mode. VESA E-EDID 1.4 decides where QEMU departs from it: a name of 13
//! characters fills its descriptor with no line feed (3.10.3.4). What
//! names the display — its maker, product, name and date of manufacture,
//! which QEMU fixes as vendor "RHT", model 0x1234, "QEMU Monitor" and week
//! 42 of 2014 — is an input, the board's own
//! ([`crate::identity::BoardIdentity`]).

use crate::identity::DisplayIdentity;

/// Bytes of EDID a virtio-gpu response carries (`struct
/// virtio_gpu_resp_edid`'s `edid`).
pub const EDID_LEN: usize = 1024;

/// `modes[]`: the standard modes, by where each is advertised.
struct Mode {
    xres: u32,
    yres: u32,
    /// Byte of the established timings bitmap, 0 for none.
    byte: usize,
    /// Byte of the "additional standard timings 3" descriptor, 0 for none.
    xtra3: usize,
    bit: u8,
    /// CTA-861 video identification code, 0 for none.
    dta: u8,
}

const fn mode(xres: u32, yres: u32, byte: usize, xtra3: usize, bit: u8, dta: u8) -> Mode {
    Mode {
        xres,
        yres,
        byte,
        xtra3,
        bit,
        dta,
    }
}

const MODES: [Mode; 22] = [
    // dea/dta extension timings (all @ 50 Hz)
    mode(5120, 2160, 0, 0, 0, 125),
    mode(4096, 2160, 0, 0, 0, 101),
    mode(3840, 2160, 0, 0, 0, 96),
    mode(2560, 1080, 0, 0, 0, 89),
    mode(2048, 1152, 0, 0, 0, 0),
    mode(1920, 1080, 0, 0, 0, 31),
    // dea/dta extension timings (all @ 60 Hz)
    mode(3840, 2160, 0, 0, 0, 97),
    // additional standard timings 3 (all @ 60Hz)
    mode(1920, 1200, 0, 10, 0, 0),
    mode(1600, 1200, 0, 9, 2, 0),
    mode(1680, 1050, 0, 9, 5, 0),
    mode(1440, 900, 0, 8, 5, 0),
    mode(1280, 1024, 0, 7, 1, 0),
    mode(1280, 960, 0, 7, 3, 0),
    mode(1280, 768, 0, 7, 6, 0),
    mode(1920, 1440, 0, 11, 5, 0),
    mode(1856, 1392, 0, 10, 3, 0),
    mode(1792, 1344, 0, 10, 5, 0),
    mode(1440, 1050, 0, 8, 1, 0),
    mode(1360, 768, 0, 8, 7, 0),
    // established timings (all @ 60Hz)
    mode(1024, 768, 36, 0, 3, 0),
    mode(800, 600, 35, 0, 0, 0),
    mode(640, 480, 35, 0, 5, 0),
];

/// `Timings`: blanking and clock for the preferred mode.
struct Timings {
    xfront: u32,
    xsync: u32,
    xblank: u32,
    yfront: u32,
    ysync: u32,
    yblank: u32,
    /// In units of 10 kHz.
    clock: u64,
}

/// QEMU `generate_timings`.
fn generate_timings(refresh_rate: u32, xres: u32, yres: u32) -> Timings {
    let xblank = xres * 35 / 100;
    let yblank = yres * 35 / 1000;
    Timings {
        xfront: xres * 25 / 100,
        xsync: xres * 3 / 100,
        xblank,
        yfront: yres * 5 / 1000,
        ysync: yres * 5 / 1000,
        yblank,
        clock: u64::from(refresh_rate) * u64::from(xres + xblank) * u64::from(yres + yblank)
            / 10_000_000,
    }
}

/// QEMU `edid_std_mode`: a standard timing, if the mode has one of the four
/// aspect ratios and fits; `(0, 0)` marks an unused slot.
fn std_mode(edid: &mut [u8], at: usize, xres: u32, yres: u32) -> bool {
    if xres == 0 || yres == 0 {
        edid[at] = 0x01;
        edid[at + 1] = 0x01;
        return true;
    }
    let aspect = if xres * 10 == yres * 16 {
        0
    } else if xres * 3 == yres * 4 {
        1
    } else if xres * 4 == yres * 5 {
        2
    } else if xres * 9 == yres * 16 {
        3
    } else {
        return false;
    };
    let code = (xres / 8).wrapping_sub(31);
    if code > 255 {
        return false;
    }
    edid[at] = code as u8;
    edid[at + 1] = aspect << 6;
    true
}

/// QEMU `edid_ext_dta_mode`.
fn dta_mode(edid: &mut [u8], dta: usize, vic: u8) {
    let at = dta + usize::from(edid[dta + 2]);
    edid[at] = vic;
    edid[dta + 2] += 1;
    edid[dta + 4] += 1;
}

/// QEMU `edid_fill_modes`, with no largest mode.
fn fill_modes(edid: &mut [u8], xtra3: Option<usize>, dta: Option<usize>) {
    let mut std = 38;
    for m in &MODES {
        if m.byte != 0 {
            edid[m.byte] |= 1 << m.bit;
        } else if std < 54 {
            if std_mode(edid, std, m.xres, m.yres) {
                std += 2;
            }
        } else if let (true, Some(xtra3)) = (m.xtra3 != 0, xtra3) {
            edid[xtra3 + m.xtra3] |= 1 << m.bit;
        }
        if let (Some(dta), true) = (dta, m.dta != 0) {
            dta_mode(edid, dta, m.dta);
        }
    }
    while std < 54 {
        std_mode(edid, std, 0, 0);
        std += 2;
    }
}

/// QEMU `edid_checksum`: make `len` bytes from `at` and the one after sum
/// to 0 mod 256.
fn checksum(edid: &mut [u8], at: usize, len: usize) {
    let sum = edid[at..at + len]
        .iter()
        .fold(0u32, |s, &b| s + u32::from(b))
        & 0xFF;
    if sum != 0 {
        edid[at + len] = (0x100 - sum) as u8;
    }
}

/// QEMU `edid_desc_next`: the descriptor after `desc`, in the base block
/// and then the CTA extension, or `None` when both are full.
fn desc_next(edid: &[u8], dta: Option<usize>, desc: Option<usize>) -> Option<usize> {
    let desc = desc?;
    if desc + 18 + 18 < 127 {
        return Some(desc + 18);
    }
    let dta = dta?;
    if desc < 127 {
        return Some(dta + usize::from(edid[dta + 2]));
    }
    if desc + 18 + 18 < dta + 127 {
        return Some(desc + 18);
    }
    None
}

/// QEMU `edid_desc_type`.
fn desc_type(edid: &mut [u8], desc: usize, kind: u8) {
    edid[desc..desc + 5].copy_from_slice(&[0, 0, 0, kind, 0]);
}

/// A string descriptor (VESA E-EDID 1.4, 3.10.3.1 to 3.10.3.4): up to 13
/// characters, a shorter string ended by a line feed and padded with
/// spaces. QEMU's `edid_desc_text` keeps 12 and always ends them so.
fn desc_text(edid: &mut [u8], desc: usize, kind: u8, text: &[u8]) {
    desc_type(edid, desc, kind);
    edid[desc + 5..desc + 18].fill(b' ');
    let len = text.len().min(13);
    edid[desc + 5..desc + 5 + len].copy_from_slice(&text[..len]);
    if len < 13 {
        edid[desc + 5 + len] = b'\n';
    }
}

/// QEMU `edid_desc_ranges`: 50–125 Hz, 30–160 kHz, 2550 MHz.
fn desc_ranges(edid: &mut [u8], desc: usize) {
    desc_type(edid, desc, 0xFD);
    edid[desc + 5..desc + 12].copy_from_slice(&[50, 125, 30, 160, (2550 / 10) as u8, 0x01, b'\n']);
    edid[desc + 12..desc + 18].fill(b' ');
}

/// QEMU `edid_desc_timing`: the preferred mode's detailed timing.
fn desc_timing(
    edid: &mut [u8],
    desc: usize,
    t: &Timings,
    xres: u32,
    yres: u32,
    xmm: u32,
    ymm: u32,
) {
    let d = &mut edid[desc..desc + 18];
    d[..2].copy_from_slice(&(t.clock as u16).to_le_bytes());
    d[2] = xres as u8;
    d[3] = t.xblank as u8;
    d[4] = (((xres & 0xF00) >> 4) | ((t.xblank & 0xF00) >> 8)) as u8;
    d[5] = yres as u8;
    d[6] = t.yblank as u8;
    d[7] = (((yres & 0xF00) >> 4) | ((t.yblank & 0xF00) >> 8)) as u8;
    d[8] = t.xfront as u8;
    d[9] = t.xsync as u8;
    d[10] = (((t.yfront & 0x00F) << 4) | (t.ysync & 0x00F)) as u8;
    d[11] = (((t.xfront & 0x300) >> 2)
        | ((t.xsync & 0x300) >> 4)
        | ((t.yfront & 0x030) >> 2)
        | ((t.ysync & 0x030) >> 4)) as u8;
    d[12] = xmm as u8;
    d[13] = ymm as u8;
    d[14] = (((xmm & 0xF00) >> 4) | ((ymm & 0xF00) >> 8)) as u8;
    d[17] = 0x18;
}

/// QEMU `edid_to_10bit`: `(uint32_t)(value * 1024 + 0.5)` on a `float`.
fn to_10bit(value: f32) -> u32 {
    (f64::from(value * 1024.0) + 0.5) as u32
}

/// QEMU `edid_colorspace` for the sRGB primaries and white point.
fn colorspace(edid: &mut [u8]) {
    let [rx, ry, gx, gy, bx, by, wx, wy] = [
        0.6400f32, 0.3300, 0.3000, 0.6000, 0.1500, 0.0600, 0.3127, 0.3290,
    ]
    .map(to_10bit);
    edid[25] = (((rx & 0x03) << 6) | ((ry & 0x03) << 4) | ((gx & 0x03) << 2) | (gy & 0x03)) as u8;
    edid[26] = (((bx & 0x03) << 6) | ((by & 0x03) << 4) | ((wx & 0x03) << 2) | (wy & 0x03)) as u8;
    for (i, v) in [rx, ry, gx, gy, bx, by, wx, wy].into_iter().enumerate() {
        edid[27 + i] = (v >> 2) as u8;
    }
}

/// QEMU `init_displayid`.
fn init_displayid(edid: &mut [u8], did: usize) {
    edid[did..did + 4].copy_from_slice(&[0x70, 0x13, 4, 0x03]);
    let len = usize::from(edid[did + 2]) + 4;
    checksum(edid, did + 1, len);
}

/// QEMU `qemu_displayid_generate`: the preferred mode as a DisplayID
/// detailed timing, for a screen too large for the base block's.
fn displayid_generate(edid: &mut [u8], did: usize, t: &Timings, xres: u32, yres: u32) {
    edid[did..did + 4].copy_from_slice(&[0x70, 0x13, 23, 0x03]);
    edid[did + 5..did + 8].copy_from_slice(&[0x03, 0x00, 0x14]);
    edid[did + 8] = t.clock as u8;
    edid[did + 9] = (t.clock >> 8) as u8;
    edid[did + 10] = (t.clock >> 16) as u8;
    edid[did + 11] = 0x88;
    let fields = [
        xres.wrapping_sub(1),
        t.xblank.wrapping_sub(1),
        t.xfront.wrapping_sub(1),
        t.xsync.wrapping_sub(1),
        yres.wrapping_sub(1),
        t.yblank.wrapping_sub(1),
        t.yfront.wrapping_sub(1),
        t.ysync.wrapping_sub(1),
    ];
    for (i, f) in fields.into_iter().enumerate() {
        edid[did + 12 + 2 * i..did + 14 + 2 * i].copy_from_slice(&(f as u16).to_le_bytes());
    }
    let len = usize::from(edid[did + 2]) + 4;
    checksum(edid, did + 1, len);
}

/// QEMU `qemu_edid_dpi_to_mm`.
fn dpi_to_mm(dpi: u32, res: u32) -> u32 {
    res * 254 / 10 / dpi
}

/// QEMU `qemu_edid_generate` into `EDID_LEN` bytes, for a display whose
/// preferred mode is `prefx` by `prefy` and which `display` describes.
pub(crate) fn generate(prefx: u32, prefy: u32, display: &DisplayIdentity) -> [u8; EDID_LEN] {
    let mut edid = [0u8; EDID_LEN];
    let refresh_rate = 75_000;
    let dpi = 100;
    let (prefx, prefy) = (
        if prefx == 0 { 1280 } else { prefx },
        if prefy == 0 { 800 } else { prefy },
    );
    let width_mm = dpi_to_mm(dpi, prefx);
    let height_mm = dpi_to_mm(dpi, prefy);
    let timings = generate_timings(refresh_rate, prefx, prefy);
    let large_screen = prefx >= 4096 || prefy >= 4096 || timings.clock >= 65536;

    // Extensions: a CTA block always fits, a DisplayID block for a large
    // screen.
    let dta = 128;
    edid[126] += 1;
    edid[dta..dta + 5].copy_from_slice(&[0x02, 0x03, 0x05, 0x00, 0x40]);
    let did = large_screen.then_some(256);
    if let Some(did) = did {
        edid[126] += 1;
        init_displayid(&mut edid, did);
    }

    // Header, the display's maker and product, no serial, its date of
    // manufacture, EDID 1.4.
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

    // Basic display parameters: digital, 8 bits a colour, DisplayPort; the
    // size; gamma 2.2; sRGB with a preferred timing.
    edid[20] = 0xA5;
    edid[21] = (width_mm / 10) as u8;
    edid[22] = (height_mm / 10) as u8;
    edid[23] = 220 - 100;
    edid[24] = 0x06;
    colorspace(&mut edid);

    // Descriptors.
    let mut desc = Some(54);
    if !large_screen {
        desc_timing(&mut edid, 54, &timings, prefx, prefy, width_mm, height_mm);
        desc = desc_next(&edid, Some(dta), desc);
    }
    let xtra3 = desc;
    if let Some(at) = xtra3 {
        desc_type(&mut edid, at, 0xF7);
        edid[at + 5] = 10;
    }
    desc = desc_next(&edid, Some(dta), desc);
    fill_modes(&mut edid, xtra3, Some(dta));
    if let Some(at) = desc {
        desc_ranges(&mut edid, at);
    }
    desc = desc_next(&edid, Some(dta), desc);
    if let Some(at) = desc {
        desc_text(&mut edid, at, 0xFC, display.name.as_bytes());
        desc = desc_next(&edid, Some(dta), desc);
    }
    while let Some(at) = desc {
        desc_type(&mut edid, at, 0x10);
        desc = desc_next(&edid, Some(dta), desc);
    }

    if let Some(did) = did {
        displayid_generate(&mut edid, did, &timings, prefx, prefy);
    }

    checksum(&mut edid, 0, 127);
    checksum(&mut edid, dta, 127);
    if let Some(did) = did {
        checksum(&mut edid, did, 127);
    }
    edid
}
