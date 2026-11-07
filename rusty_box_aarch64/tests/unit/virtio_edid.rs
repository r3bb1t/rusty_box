//! Unit tests of `src/devices/virtio/edid.rs`: the display's EDID as a
//! guest's parser reads it — VESA E-EDID 1.4's base block, VESA CVT 1.2's
//! and DMT's timings, and a DisplayID 1.3 block where the base block
//! cannot hold the display's mode.

use std::vec::Vec;

use super::generate;
use crate::identity::{
    BoardIdentity, DisplayIdentity, EdidDate, EdidProductCode, EdidWeek, EdidYear, IdentityNames,
    PnpId,
};

/// What the default identity's EDID says of its display.
fn boards_display() -> DisplayIdentity {
    BoardIdentity::RUSTY_BOX.display()
}

fn edid(width: u32, height: u32) -> [u8; 1024] {
    generate(width, height, &boards_display())
}

/// The bytes of a block sum to 0 (E-EDID 1.4, 3.11).
fn sums_to_zero(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |s, &b| s.wrapping_add(b)) == 0
}

/// A detailed timing descriptor as a guest's parser unpacks it (E-EDID
/// 1.4, Tables 3.21 and 3.22).
#[derive(Debug, PartialEq, Eq)]
struct Detailed {
    clock_10khz: u32,
    h_active: u32,
    h_blank: u32,
    h_front: u32,
    h_sync: u32,
    v_active: u32,
    v_blank: u32,
    v_front: u32,
    v_sync: u32,
    width_mm: u32,
    height_mm: u32,
    flags: u8,
}

fn detailed(e: &[u8], at: usize) -> Detailed {
    let d = &e[at..at + 18];
    let b = |i: usize| u32::from(d[i]);
    Detailed {
        clock_10khz: b(0) | (b(1) << 8),
        h_active: b(2) | ((b(4) >> 4) << 8),
        h_blank: b(3) | ((b(4) & 0xF) << 8),
        v_active: b(5) | ((b(7) >> 4) << 8),
        v_blank: b(6) | ((b(7) & 0xF) << 8),
        h_front: b(8) | ((b(11) >> 6) << 8),
        h_sync: b(9) | (((b(11) >> 4) & 3) << 8),
        v_front: (b(10) >> 4) | (((b(11) >> 2) & 3) << 4),
        v_sync: (b(10) & 0xF) | ((b(11) & 3) << 4),
        width_mm: b(12) | ((b(14) >> 4) << 8),
        height_mm: b(13) | ((b(14) & 0xF) << 8),
        flags: d[17],
    }
}

/// The base block's four 18-byte descriptors' tags: `None` for a detailed
/// timing, the display descriptor's tag otherwise (Table 3.23).
fn descriptor_tags(e: &[u8]) -> [Option<u8>; 4] {
    [54, 72, 90, 108].map(|at| (e[at..at + 3] == [0, 0, 0]).then_some(e[at + 3]))
}

/// The display descriptor with `tag`, wherever it is.
fn descriptor(e: &[u8], tag: u8) -> Option<&[u8]> {
    [54, 72, 90, 108]
        .into_iter()
        .find(|&at| e[at..at + 5] == [0, 0, 0, tag, 0])
        .map(|at| &e[at..at + 18])
}

/// The display product name descriptor's 13 bytes (tag FCh).
fn edid_name(e: &[u8]) -> [u8; 13] {
    let d = descriptor(e, 0xFC).unwrap_or_else(|| panic!("no display product name descriptor"));
    let mut name = [0u8; 13];
    name.copy_from_slice(&d[5..]);
    name
}

/// The maker's PNP ID, as a guest's parser unpacks bytes 8 and 9: five bits
/// a letter, 1 for A (VESA E-EDID 1.4, 3.4.1).
fn edid_maker(e: &[u8]) -> [char; 3] {
    let id = u16::from_be_bytes([e[8], e[9]]);
    [10, 5, 0].map(|shift| (b'@' + ((id >> shift) & 0x1F) as u8) as char)
}

/// The eight standard timings' bytes (26h to 35h).
fn standard_timings(e: &[u8]) -> Vec<[u8; 2]> {
    e[0x26..0x36].chunks(2).map(|c| [c[0], c[1]]).collect()
}

/// A DisplayID Type I detailed timing as Linux's `drm_mode_displayid_detailed`
/// and edid-decode read it: every field stored less one.
#[derive(Debug, PartialEq, Eq)]
struct DisplayIdTiming {
    clock_10khz: u32,
    flags: u8,
    h_active: u32,
    h_blank: u32,
    h_front: u32,
    h_sync_positive: bool,
    h_sync: u32,
    v_active: u32,
    v_blank: u32,
    v_front: u32,
    v_sync_positive: bool,
    v_sync: u32,
}

fn displayid_timing(t: &[u8]) -> DisplayIdTiming {
    let w = |i: usize| u32::from(u16::from_le_bytes([t[i], t[i + 1]]));
    DisplayIdTiming {
        clock_10khz: (u32::from(t[0]) | (u32::from(t[1]) << 8) | (u32::from(t[2]) << 16)) + 1,
        flags: t[3],
        h_active: w(4) + 1,
        h_blank: w(6) + 1,
        h_front: (w(8) & 0x7FFF) + 1,
        h_sync_positive: t[9] & 0x80 != 0,
        h_sync: w(10) + 1,
        v_active: w(12) + 1,
        v_blank: w(14) + 1,
        v_front: (w(16) & 0x7FFF) + 1,
        v_sync_positive: t[17] & 0x80 != 0,
        v_sync: w(18) + 1,
    }
}

/// The sizes the tests below look at, from none to more than any
/// description holds.
const SIZES: [[u32; 2]; 17] = [
    [0, 0],
    [0, 768],
    [1, 1],
    [320, 200],
    [640, 480],
    [1023, 767],
    [1024, 768],
    [1280, 800],
    [1366, 768],
    [1920, 1080],
    [3840, 2160],
    [4096, 2160],
    [5120, 2880],
    [7680, 4320],
    [16000, 9000],
    [0x1_0000, 0x1_0000],
    [u32::MAX, u32::MAX],
];

/// Every EDID is a valid E-EDID 1.4 structure: its header and version, a
/// checksum in every block, the extension count byte 7Eh counting the
/// blocks that follow and nothing after them, and the preferred timing
/// first (3.10.1), detailed timings before display descriptors and no
/// fill pattern (Table 3.20 notes 1, 3 and 4).
#[test]
fn every_edid_is_well_formed() {
    for [w, h] in SIZES {
        let e = edid(w, h);
        assert_eq!(e[..8], [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0], "{w}x{h}");
        assert_eq!((e[0x12], e[0x13]), (1, 4), "EDID 1.4, {w}x{h}");
        let blocks = 1 + usize::from(e[126]);
        assert!(blocks <= 2, "{w}x{h}");
        for block in e[..blocks * 128].chunks(128) {
            assert!(sums_to_zero(block), "checksum, {w}x{h}");
        }
        assert!(e[blocks * 128..].iter().all(|&b| b == 0), "{w}x{h}");
        let tags = descriptor_tags(&e);
        assert_eq!(tags[0], None, "a detailed timing first, {w}x{h}");
        for tag in tags[1..].iter().flatten() {
            assert!(
                matches!(tag, 0xF7 | 0xFC | 0xFD | 0x10),
                "{tag:#x}, {w}x{h}"
            );
        }
        if let Some(dummy) = descriptor(&e, 0x10) {
            assert!(dummy[5..].iter().all(|&b| b == 0), "{w}x{h}");
        }
    }
}

/// The display's own size is its preferred mode: in the first detailed
/// timing where that holds it, the feature byte saying it is the native
/// one (3.6.4 bit 1); in a DisplayID block marked preferred where it does
/// not; nowhere for a display with no pixels or with more than DisplayID
/// holds.
#[test]
fn the_preferred_mode_is_the_displays_own_size() {
    for [w, h] in SIZES {
        let e = edid(w, h);
        let first = detailed(&e, 54);
        let in_first = (first.h_active, first.v_active) == (w, h);
        if e[126] == 1 {
            assert_eq!(e[128], 0x70, "{w}x{h}");
            let t = displayid_timing(&e[136..156]);
            assert_eq!((t.h_active, t.v_active), (w, h));
            assert_eq!(t.flags & 0x80, 0x80, "preferred, {w}x{h}");
            assert!(!in_first);
            assert_eq!(e[0x18] & 0x02, 0, "not native first, {w}x{h}");
        } else if in_first {
            assert_eq!(e[0x18] & 0x02, 0x02, "native first, {w}x{h}");
        } else {
            // No pixels, a clock past DisplayID's 24 bits, or more pixels
            // than its 16-bit counts.
            assert!(
                matches!(
                    [w, h],
                    [0, _] | [_, 0] | [0x1_0000, 0x1_0000] | [u32::MAX, u32::MAX]
                ),
                "{w}x{h}"
            );
            assert_eq!(e[0x18] & 0x02, 0, "{w}x{h}");
        }
    }
    for [w, h] in [[640, 480], [1024, 768], [1280, 800], [3840, 2160], [4095, 2160]] {
        assert_eq!(edid(w, h)[126], 0, "{w}x{h} fits a detailed timing");
    }
    // Past 4095 pixels, or past a 655.35 MHz clock (4095 by 4095 needs
    // 1074 MHz), a detailed timing cannot say it.
    for [w, h] in [[4096, 2160], [4095, 4095]] {
        assert_eq!(edid(w, h)[126], 1, "{w}x{h} needs DisplayID");
    }
}

/// The preferred timing is CVT 1.2 reduced blanking at 60 Hz: 160 pixels
/// of horizontal blank (front porch 48, sync 32, positive), a front porch
/// of 3 lines and a negative sync of 4 to 10 by the aspect ratio, at least
/// 460 µs of vertical blank, the clock rounded down to 0.25 MHz; digital
/// separate sync. The values VESA's CVT tables give these modes.
#[test]
fn the_preferred_timing_is_cvt_reduced_blanking() {
    let cases = [
        // size, clock (10 kHz), vertical blank, vertical sync
        ([1024, 768], 5600, 22, 4),
        ([1280, 800], 7100, 23, 6),
        ([1920, 1080], 13850, 31, 5),
        ([1920, 1200], 15400, 35, 6),
        ([1280, 1024], 9100, 30, 7),
        ([3840, 2160], 53325, 62, 5),
        ([1366, 768], 7225, 22, 10),
    ];
    for ([w, h], clock_10khz, v_blank, v_sync) in cases {
        let t = detailed(&edid(w, h), 54);
        assert_eq!(
            t,
            Detailed {
                clock_10khz,
                h_active: w,
                h_blank: 160,
                h_front: 48,
                h_sync: 32,
                v_active: h,
                v_blank,
                v_front: 3,
                v_sync,
                width_mm: t.width_mm,
                height_mm: t.height_mm,
                flags: 0x1A,
            },
            "{w}x{h}"
        );
    }
}

/// The base block's fixed parameters: digital input of 8 bits a primary
/// with no interface named, gamma 2.2, sRGB's chromaticity and white point
/// with the sRGB feature bit, RGB 4:4:4, no power states, a
/// non-continuous-frequency display.
#[test]
fn the_display_is_a_digital_srgb_display() {
    let e = edid(1024, 768);
    assert_eq!(e[0x14], 0xA0, "digital, 8 bits a primary, no interface");
    assert_eq!(e[0x17], 120, "gamma 2.2");
    assert_eq!(e[0x18], 0x06, "sRGB, native preferred, non-continuous");
    assert_eq!(
        e[0x19..0x23],
        [0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54],
        "sRGB: red 0.640 0.330, green 0.300 0.600, blue 0.150 0.060, white 0.3127 0.3290"
    );
}

/// The screen is 96 pixels an inch: its size in cm and the preferred
/// image's in mm, each to the nearest; a screen too large for 255 cm gives
/// its aspect ratio instead, and one with no pixels no size (3.6.2).
#[test]
fn the_screen_is_96_pixels_an_inch() {
    let e = edid(1024, 768);
    assert_eq!(e[0x15..0x17], [27, 20], "27.09 by 20.32 cm");
    let t = detailed(&e, 54);
    assert_eq!((t.width_mm, t.height_mm), (271, 203));
    let e = edid(1920, 1080);
    assert_eq!(e[0x15..0x17], [51, 29], "50.80 by 28.58 cm");
    let t = detailed(&e, 54);
    assert_eq!((t.width_mm, t.height_mm), (508, 286));

    assert_eq!(edid(10_000, 5_000)[0x15..0x17], [101, 0], "2:1 landscape");
    assert_eq!(edid(5_000, 10_000)[0x15..0x17], [0, 101], "1:2 portrait");
    assert_eq!(edid(10, 10)[0x15..0x17], [1, 0], "under a cm: 1:1");
    assert_eq!(edid(20_000, 1_000)[0x15..0x17], [0, 0], "20:1, past 3.54:1");
    assert_eq!(edid(0, 0)[0x15..0x17], [0, 0], "no pixels, no size");
}

/// The listed modes are VESA DMT's at 60 Hz that fit within the display,
/// and the base video mode, 640 by 480, always: in Established Timings I &
/// II, Established Timings III and the standard timings, unused ones
/// 01h 01h (3.8, 3.9, 3.10.3.9).
#[test]
fn the_modes_are_the_dmt_modes_that_fit() {
    let e = edid(1024, 768);
    assert_eq!(e[0x23..0x26], [0x21, 0x08, 0], "640x480, 800x600, 1024x768");
    assert_eq!(standard_timings(&e), [[1, 1]; 8]);
    let iii = descriptor(&e, 0xF7).unwrap_or_else(|| panic!("Established Timings III"));
    assert_eq!(iii[5..12], [0x0A, 0x08, 0, 0, 0, 0, 0], "848x480");
    assert!(iii[12..].iter().all(|&b| b == 0));

    let e = edid(1920, 1080);
    assert_eq!(e[0x23..0x26], [0x21, 0x08, 0]);
    assert_eq!(
        standard_timings(&e),
        [[0x81, 0x00], [0xD1, 0xC0], [0xA9, 0xC0], [1, 1], [1, 1], [1, 1], [1, 1], [1, 1]],
        "1280x800, 1920x1080, 1600x900"
    );
    let iii = descriptor(&e, 0xF7).unwrap_or_else(|| panic!("Established Timings III"));
    assert_eq!(
        iii[6..12],
        [0x08, 0x4A, 0xA2, 0x20, 0, 0],
        "848x480; 1280x768, 1280x960, 1280x1024; 1360x768, 1440x900, 1400x1050; 1680x1050"
    );

    for [w, h] in [[640, 480], [320, 200], [0, 0]] {
        let e = edid(w, h);
        assert_eq!(e[0x23..0x26], [0x20, 0, 0], "the base video mode, {w}x{h}");
        assert_eq!(standard_timings(&e), [[1, 1]; 8]);
        assert_eq!(descriptor(&e, 0xF7), None, "{w}x{h}");
    }
}

/// The descriptors in order: the preferred timing, Established Timings
/// III, the range limits, the name, then dummies.
#[test]
fn the_descriptors_are_in_order() {
    assert_eq!(
        descriptor_tags(&edid(1024, 768)),
        [None, Some(0xF7), Some(0xFD), Some(0xFC)]
    );
    assert_eq!(
        descriptor_tags(&edid(640, 480)),
        [None, Some(0xFD), Some(0xFC), Some(0x10)]
    );
}

/// The range limits bound the listed modes and the preferred one, and no
/// more: field rates rounded down and up to a Hz, line rates to a kHz, the
/// fastest clock up to 10 MHz; "Range Limits Only" (Table 3.26).
#[test]
fn the_range_limits_hold_the_listed_modes() {
    // 640x480 at 59.94 Hz and 31.47 kHz, 800x600 at 60.32 Hz, 1024x768 at
    // 48.36 kHz and 65 MHz.
    let e = edid(1024, 768);
    let limits = descriptor(&e, 0xFD).unwrap_or_else(|| panic!("range limits"));
    assert_eq!(
        limits[4..],
        [0, 59, 61, 31, 49, 7, 0x01, 0x0A, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]
    );
    // The base video mode alone: 59.94 Hz, 31.47 kHz, 25.175 MHz.
    let e = edid(0, 0);
    let limits = descriptor(&e, 0xFD).unwrap_or_else(|| panic!("range limits"));
    assert_eq!(limits[4..10], [0, 59, 60, 31, 32, 3]);
    // A native mode in a DisplayID block counts: 5120x2880 at 177.70 kHz and
    // 938.25 MHz.
    let e = edid(5120, 2880);
    let limits = descriptor(&e, 0xFD).unwrap_or_else(|| panic!("range limits"));
    assert_eq!(limits[4..10], [0, 59, 61, 31, 178, 94]);
    // 16000x9000 at 555 kHz: past the 510 kHz a descriptor holds, so none.
    let e = edid(16_000, 9_000);
    assert_eq!(descriptor(&e, 0xFD), None);
    assert_eq!(e[126], 1, "the mode itself still in DisplayID");
}

/// A display whose mode no detailed timing holds keeps the largest listed
/// mode first, at its DMT timing and filling the screen's height or width
/// at its own aspect ratio, and gives its own in a DisplayID 1.3 extension:
/// a standalone display's section with one Type I detailed timing,
/// preferred, at its aspect ratio, each field stored less one.
#[test]
fn a_display_too_large_for_a_detailed_timing_gets_a_displayid_block() {
    let e = edid(5120, 2880);
    assert_eq!(e[126], 1);
    assert_eq!(e[0x15..0x17], [135, 76]);
    assert_eq!(e[0x18], 0x04, "the first timing not the native one");
    // DMT 49h, 1920x1440 at 60 Hz, on a 1355 by 762 mm screen.
    assert_eq!(
        detailed(&e, 54),
        Detailed {
            clock_10khz: 23400,
            h_active: 1920,
            h_blank: 680,
            h_front: 128,
            h_sync: 208,
            v_active: 1440,
            v_blank: 60,
            v_front: 1,
            v_sync: 3,
            width_mm: 1016,
            height_mm: 762,
            flags: 0x1C,
        }
    );

    let block = &e[128..256];
    assert_eq!(
        block[..8],
        [0x70, 0x13, 23, 0x03, 0, 0x03, 0, 20],
        "DisplayID 1.3, 23 bytes, a standalone display, one Type I timing"
    );
    assert!(sums_to_zero(&block[1..29]), "the section's checksum");
    assert!(block[29..127].iter().all(|&b| b == 0));
    assert_eq!(
        displayid_timing(&block[8..28]),
        DisplayIdTiming {
            clock_10khz: 93825,
            flags: 0x84,
            h_active: 5120,
            h_blank: 160,
            h_front: 48,
            h_sync_positive: true,
            h_sync: 32,
            v_active: 2880,
            v_blank: 82,
            v_front: 3,
            v_sync_positive: false,
            v_sync: 5,
        },
        "CVT reduced blanking, 938.25 MHz, 16:9, preferred"
    );
}

/// A display with no pixels lists the base video mode first, at no size,
/// and says nothing of a native mode.
#[test]
fn a_display_of_no_pixels_lists_the_base_video_mode() {
    let e = edid(0, 0);
    assert_eq!(e[126], 0);
    assert_eq!(e[0x18], 0x04);
    assert_eq!(
        detailed(&e, 54),
        Detailed {
            clock_10khz: 2518,
            h_active: 640,
            h_blank: 160,
            h_front: 16,
            h_sync: 96,
            v_active: 480,
            v_blank: 45,
            v_front: 10,
            v_sync: 2,
            width_mm: 0,
            height_mm: 0,
            flags: 0x18,
        },
        "DMT 04h, 25.175 MHz to the descriptor's 10 kHz"
    );
}

/// What a guest's EDID parser reads of the default identity's display: the
/// PNP ID "RBX" packed five bits a letter, product 1, no serial, no week
/// and the year 2026, and the name "Rusty Box" ended by a line feed and
/// padded with spaces.
#[test]
fn the_edid_names_the_boards_display() {
    let e = edid(1024, 768);
    assert_eq!(edid_maker(&e), ['R', 'B', 'X']);
    assert_eq!(e[10..12], [1, 0], "product 1");
    assert_eq!(e[12..16], [0; 4], "no serial");
    assert_eq!((e[16], e[17]), (0, 36), "no week, 1990 + 36 = 2026");
    assert_eq!(&edid_name(&e), b"Rusty Box\n   ");

    let host = BoardIdentity::new(IdentityNames {
        display_vendor: PnpId::new("ACM").unwrap_or_else(|err| panic!("{err}")),
        display_product: EdidProductCode::new(0x0042),
        display_name: "Acme Screen",
        display_date: EdidDate::Manufactured {
            week: EdidWeek::Unstated,
            year: EdidYear::new(2030).unwrap_or_else(|err| panic!("{err}")),
        },
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|err| panic!("{err}"));
    let e = generate(1024, 768, &host.display());
    assert_eq!(edid_maker(&e), ['A', 'C', 'M']);
    assert_eq!(e[10..12], [0x42, 0], "product 0x42");
    assert_eq!((e[16], e[17]), (0, 40), "no week, 1990 + 40 = 2030");
    assert_eq!(&edid_name(&e), b"Acme Screen\n ");
    assert!(sums_to_zero(&e[..128]));

    // A name of 13 characters fills the descriptor with no line feed (VESA
    // E-EDID 1.4, 3.10.3.4).
    let full = BoardIdentity::new(IdentityNames {
        display_name: "Thirteen chrs",
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|err| panic!("{err}"));
    let e = generate(1024, 768, &full.display());
    assert_eq!(&edid_name(&e), b"Thirteen chrs");
    assert!(sums_to_zero(&e[..128]));

    // A model year: the Model Year Flag, FFh, then the year (Table 3.8).
    let model_year = BoardIdentity::new(IdentityNames {
        display_date: EdidDate::ModelYear(
            EdidYear::new(2031).unwrap_or_else(|err| panic!("{err}")),
        ),
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|err| panic!("{err}"));
    let e = generate(1024, 768, &model_year.display());
    assert_eq!((e[16], e[17]), (0xFF, 41), "model year 1990 + 41 = 2031");
    assert!(sums_to_zero(&e[..128]));
}
