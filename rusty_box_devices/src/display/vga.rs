#![allow(dead_code)]
//! VGA Display Controller
//!
//! Implements VGA text mode (80x25) and graphics mode memory access.
//! Based on Bochs vgacore.cc and vga.cc.
//!
//! ## Memory Layout
//!
//! VGA planar memory: 256KB (`vga_memory`), organized as `memory[offset * 4 + plane]`
//! matching Bochs vgacore.cc. The `text_memory` buffer (32KB) is maintained for
//! text mode rendering (interleaved char+attr), updated from planar memory on writes.
//!
//! ## Write Modes (Graphics Controller register 5, bits 0-1)
//!
//! Write mode 0 (default): data rotate + set/reset + logical op + bitmask + map mask
//! Write mode 1: latch copy (new_val = latch)
//! Write mode 2: per-plane from data bits + logical op + bitmask + map mask
//! Write mode 3: data rotate + bitmask AND value + set/reset + logical op
//!
//! ## Read Modes (Graphics Controller register 5, bit 3)
//!
//! Read mode 0: return plane selected by read_map_select (GFX reg 4)
//! Read mode 1: color compare (returns match bitmap)

#[cfg(feature = "alloc")]
use alloc::{string::String, vec, vec::Vec};

use crate::display::sink::{CursorPos, Dimensions, DisplaySink, Refreshed, Rgb};
use crate::display::card::VgaCard;
/// Only the graphics paths place tiles, and they need a buffer to convert into.
#[cfg(feature = "alloc")]
use crate::display::sink::TilePos;
use crate::api::BxPhyAddress;
use rusty_box_core::time::VmClock;

#[cfg(feature = "std")]
use rusty_box_core::snap::{
    checked_section_len_add, checked_section_len_mul, SnapError, SnapRead, SnapResult, SnapWrite,
    MAX_COUNT, MAX_SECTION_LEN, SECTION_VERSION,
};

/// This adapter's identity in the snapshot stream.
///
/// The tag lives with the device, which is what stops a body being written
/// under another device's identity — see `SnapshotSection`. The container
/// names it from here rather than keeping its own copy.
#[cfg(feature = "std")]
pub const SEC_VGA: u32 = 24;


/// VGA text mode information
#[derive(Debug, Clone)]
pub struct VgaTextModeInfo {
    pub start_address: u16,
    pub cs_start: u8,
    pub cs_end: u8,
    pub line_offset: u16,
    pub line_compare: u16,
    pub h_panning: u8,
    pub v_panning: u8,
    pub line_graphics: bool,
    pub split_hpanning: bool,
    pub blink_flags: u8,
    pub actl_palette: [u8; 16],
}

/// VGA text mode memory base address
const VGA_TEXT_MEM_BASE: BxPhyAddress = 0xB8000;
const VGA_TEXT_MEM_SIZE: usize = 0x8000; // 32KB
const VGA_TEXT_MEM_BASE_MONO: BxPhyAddress = 0xB0000;

/// VGA planar memory size: 256KB (0x40000), matching Bochs vgacore.cc
/// Layout: memory[offset * 4 + plane], where plane = 0..3
const VGA_MEM_SIZE: usize = 0x40000;

/// Number of DAC (PEL) colour registers.
const PEL_COLOR_COUNT: usize = 256;

/// Shift applied to 6-bit DAC components to reach 8-bit host colour — the
/// standard VGA's, and every card's until it widens its DAC.
/// Bochs: `s.dac_shift = 2` (vgacore.cc init_standard_vga).
const DAC_SHIFT: u8 = 2;

/// The shift for a DAC of the width the DISPI enable register selects: an
/// 8-bit DAC already holds host-width components.
/// Bochs vga.cc, the VBE_DISPI_INDEX_ENABLE write:
/// `s.dac_shift = new_vbe_8bit_dac ? 0 : 2`.
const fn dac_shift_for(dac_8bit: bool) -> u8 {
    if dac_8bit { 0 } else { DAC_SHIFT }
}

/// Size of one character generator: 256 glyphs x 32 bytes.
/// Bochs: the `Bit8u charmap[0x2000]` in `update_charmap` (vgacore.cc).
const CHARMAP_SIZE: usize = 0x2000;

/// Plane-2 offsets selected by sequencer register 3 (character map select).
/// Bochs: `static const Bit16u charmap_offset[8]` (vgacore.cc).
const CHARMAP_OFFSET: [u16; 8] = [
    0x0000, 0x4000, 0x8000, 0xC000, 0x2000, 0x6000, 0xA000, 0xE000,
];

/// `vga_mem_updated` bit meaning "the character generator changed".
/// Bochs: `s.vga_mem_updated |= 4` / `if ((s.vga_mem_updated & 4) > 0) update_charmap()`.
const VGA_MEM_UPDATED_CHARMAP: u8 = 4;

/// VGA clock frequencies in Hz (matching Bochs vgacore.cc)
const VGA_VCLK: [u32; 4] = [25_175_000, 28_322_000, 25_175_000, 25_175_000];

/// Color compare lookup table matching Bochs ccdat[16][4]
/// For each 4-bit color value, provides the per-plane expansion (0x00 or 0xFF)
const CCDAT: [[u8; 4]; 16] = [
    [0x00, 0x00, 0x00, 0x00],
    [0xff, 0x00, 0x00, 0x00],
    [0x00, 0xff, 0x00, 0x00],
    [0xff, 0xff, 0x00, 0x00],
    [0x00, 0x00, 0xff, 0x00],
    [0xff, 0x00, 0xff, 0x00],
    [0x00, 0xff, 0xff, 0x00],
    [0xff, 0xff, 0xff, 0x00],
    [0x00, 0x00, 0x00, 0xff],
    [0xff, 0x00, 0x00, 0xff],
    [0x00, 0xff, 0x00, 0xff],
    [0xff, 0xff, 0x00, 0xff],
    [0x00, 0x00, 0xff, 0xff],
    [0xff, 0x00, 0xff, 0xff],
    [0x00, 0xff, 0xff, 0xff],
    [0xff, 0xff, 0xff, 0xff],
];

/// Text snapshot sizes per memory mapping mode (Bochs vgacore.cc)
const TEXT_SNAP_SIZE: [usize; 4] = [0x20000, 0x10000, 0x8000, 0x8000];

/// VGA I/O ports
const VGA_CRTC_INDEX: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;
const VGA_STATUS: u16 = 0x3DA;
const VGA_CRTC_INDEX_MONO: u16 = 0x3B4;
const VGA_CRTC_DATA_MONO: u16 = 0x3B5;
const VGA_STATUS_MONO: u16 = 0x3BA;
const VGA_ATTRIB_ADDR: u16 = 0x3C0;
const VGA_ATTRIB_DATA: u16 = 0x3C1;
const VGA_MISC_OUTPUT: u16 = 0x3CC;
const VGA_SEQ_INDEX: u16 = 0x3C4;
const VGA_SEQ_DATA: u16 = 0x3C5;
const VGA_GRAPHICS_INDEX: u16 = 0x3CE;
const VGA_GRAPHICS_DATA: u16 = 0x3CF;

// ---- Additional VGA I/O ports ----
const VGA_MISC_OUTPUT_WRITE: u16 = 0x3C2;
const VGA_ENABLE: u16 = 0x3C3;
const VGA_PEL_MASK: u16 = 0x3C6;
const VGA_DAC_STATE: u16 = 0x3C7;
const VGA_PEL_ADDR_WRITE: u16 = 0x3C8;
const VGA_PEL_DATA: u16 = 0x3C9;

// ---- VBE (Bochs VGA Extension) I/O ports and constants ----
const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16 = 0x01CF;

const VBE_DISPI_INDEX_ID: u16 = 0x0;
const VBE_DISPI_INDEX_XRES: u16 = 0x1;
const VBE_DISPI_INDEX_YRES: u16 = 0x2;
const VBE_DISPI_INDEX_BPP: u16 = 0x3;
const VBE_DISPI_INDEX_ENABLE: u16 = 0x4;
const VBE_DISPI_INDEX_BANK: u16 = 0x5;
const VBE_DISPI_INDEX_VIRT_WIDTH: u16 = 0x6;
const VBE_DISPI_INDEX_VIRT_HEIGHT: u16 = 0x7;
const VBE_DISPI_INDEX_X_OFFSET: u16 = 0x8;
const VBE_DISPI_INDEX_Y_OFFSET: u16 = 0x9;
const VBE_DISPI_INDEX_VIDEO_MEMORY_64K: u16 = 0xA;
const VBE_DISPI_INDEX_DDC: u16 = 0xB;

const VBE_DISPI_ID0: u16 = 0xB0C0;
const VBE_DISPI_ID5: u16 = 0xB0C5;

const VBE_DISPI_DISABLED: u16 = 0x00;
const VBE_DISPI_ENABLED: u16 = 0x01;
const VBE_DISPI_GETCAPS: u16 = 0x02;
const VBE_DISPI_8BIT_DAC: u16 = 0x20;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;
const VBE_DISPI_NOCLEARMEM: u16 = 0x80;

const VBE_DISPI_BPP_4: u16 = 0x04;
const VBE_DISPI_BPP_8: u16 = 0x08;
const VBE_DISPI_BPP_15: u16 = 0x0F;
const VBE_DISPI_BPP_16: u16 = 0x10;
const VBE_DISPI_BPP_24: u16 = 0x18;
const VBE_DISPI_BPP_32: u16 = 0x20;

const VBE_DISPI_BANK_GRANULARITY_32K: u16 = 0x10;
const VBE_DISPI_BANK_WR: u16 = 0x4000;
const VBE_DISPI_BANK_RD: u16 = 0x8000;
const VBE_DISPI_BANK_RW: u16 = 0xC000;

const VBE_DISPI_LFB_PHYSICAL_ADDRESS: u32 = 0xE000_0000;

const VGA_X_TILESIZE: u32 = 16;
const VGA_Y_TILESIZE: u32 = 24;
/// One tile of RGBA pixels. A frame pushes tiles one at a time through a buffer
/// of this size, so no part of the graphics path allocates.
const TILE_RGBA_BYTES: usize = (VGA_X_TILESIZE * VGA_Y_TILESIZE * 4) as usize;

/// QEMU-compatible MMIO BAR2 size (4KB)
pub const PCI_VGA_MMIO_SIZE: u32 = 0x1000;
/// Offset within BAR2 MMIO for Bochs VBE extension registers
const PCI_VGA_BOCHS_OFFSET: u32 = 0x500;
/// Size of the Bochs VBE extension register region within BAR2
const PCI_VGA_BOCHS_SIZE: u32 = 0x16;

// ---- CRTC register indices ----
const CRTC_HORIZ_TOTAL: usize = 0x00;
const CRTC_HORIZ_DISPLAY_END: usize = 0x01;
const CRTC_START_HORIZ_BLANK: usize = 0x02;
const CRTC_END_HORIZ_BLANK: usize = 0x03;
const CRTC_START_HORIZ_RETRACE: usize = 0x04;
const CRTC_END_HORIZ_RETRACE: usize = 0x05;
const CRTC_VERT_TOTAL: usize = 0x06;
const CRTC_OVERFLOW: usize = 0x07;
const CRTC_PRESET_ROW_SCAN: usize = 0x08;
const CRTC_MAX_SCAN_LINE: usize = 0x09;
const CRTC_CURSOR_START: usize = 0x0A;
const CRTC_CURSOR_END: usize = 0x0B;
const CRTC_START_ADDR_HIGH: usize = 0x0C;
const CRTC_START_ADDR_LOW: usize = 0x0D;
const CRTC_CURSOR_LOC_HIGH: usize = 0x0E;
const CRTC_CURSOR_LOC_LOW: usize = 0x0F;
const CRTC_VERT_RETRACE_START: usize = 0x10;
const CRTC_VERT_RETRACE_END: usize = 0x11;
const CRTC_VERT_DISPLAY_END: usize = 0x12;
const CRTC_OFFSET: usize = 0x13;
const CRTC_UNDERLINE_LOC: usize = 0x14;
const CRTC_VERT_BLANK_START: usize = 0x15;
const CRTC_VERT_BLANK_END: usize = 0x16;
const CRTC_MODE_CONTROL: usize = 0x17;
const CRTC_LINE_COMPARE: usize = 0x18;

// ---- CRTC register bit masks ----
const CRTC_OVERFLOW_VDE_BIT8: u8 = 0x02;
const CRTC_OVERFLOW_VDE_BIT9: u8 = 0x40;
const CRTC_CURSOR_START_MASK: u8 = 0x3F;
const CRTC_CURSOR_END_MASK: u8 = 0x1F;
const CRTC_MSL_MASK: u8 = 0x1F;
const CRTC_PRESET_ROW_MASK: u8 = 0x1F;

// ---- Sequencer register indices ----
const SEQ_REG_RESET: usize = 0;
const SEQ_REG_CLOCKING_MODE: usize = 1;
const SEQ_REG_MAP_MASK: usize = 2;
const SEQ_REG_CHAR_MAP_SELECT: usize = 3;
const SEQ_REG_MEMORY_MODE: usize = 4;

// Clocking mode bits (sequencer reg 1)
const SEQ_CLOCKING_8DOT_CHAR: u8 = 0x01;
const SEQ_CLOCKING_DOTCLOCKDIV2: u8 = 0x08;

// Map mask bits (sequencer reg 2)
const SEQ_MAP_MASK_PLANES: u8 = 0x0F;
const SEQ_MAP_MASK_TEXT_PLANES: u8 = 0x03;

// ---- Graphics controller register indices ----
const GFX_REG_SET_RESET: usize = 0;
const GFX_REG_ENABLE_SET_RESET: usize = 1;
const GFX_REG_COLOR_COMPARE: usize = 2;
const GFX_REG_DATA_ROTATE: usize = 3;
const GFX_REG_READ_MAP_SELECT: usize = 4;
const GFX_REG_GRAPHICS_MODE: usize = 5;
const GFX_REG_MISC: usize = 6;
const GFX_REG_COLOR_DONT_CARE: usize = 7;
const GFX_REG_BIT_MASK: usize = 8;

// Miscellaneous Graphics register bits (reg 6)
const GFX_MISC_GRAPHICS_ALPHA: u8 = 0x01;
const GFX_MISC_MEMORY_MAP_SHIFT: u8 = 2;
const GFX_MISC_MEMORY_MAP_MASK: u8 = 0x03;

// ---- Attribute controller register indices ----
const ATTR_REG_MODE_CONTROL: usize = 0x10;
const ATTR_REG_OVERSCAN_COLOR: usize = 0x11;
const ATTR_REG_COLOR_PLANE_EN: usize = 0x12;
const ATTR_REG_HORIZ_PIXEL_PAN: usize = 0x13;
const ATTR_REG_COLOR_SELECT: usize = 0x14;

// Attribute mode control bits (reg 0x10)
const ATTR_MODE_LINE_GRAPHICS: u8 = 0x04;
const ATTR_MODE_SPLIT_HPANNING: u8 = 0x20;
const ATTR_HPANNING_MASK: u8 = 0x0F;

// ---- VGA memory mapping values (from graphics reg 6, bits 2-3) ----
/// Memory mapping mode selected by Graphics Controller register 6 bits 2-3.
///
/// Determines which address range maps to VGA memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum VgaMemoryMapping {
    /// 128KB at 0xA0000-0xBFFFF (EGA graphics)
    Ega128k = 0,
    /// 64KB at 0xA0000-0xAFFFF (VGA graphics)
    Vga64k = 1,
    /// 32KB at 0xB0000-0xB7FFF (monochrome text)
    MonoText32k = 2,
    /// 32KB at 0xB8000-0xBFFFF (color text)
    ColorText32k = 3,
}

impl VgaMemoryMapping {
    fn from_u8(val: u8) -> Self {
        match val & 0x03 {
            0 => Self::Ega128k,
            1 => Self::Vga64k,
            2 => Self::MonoText32k,
            3 => Self::ColorText32k,
            _ => unreachable!("VGA memory mapping val & 0x03 cannot exceed 3"),
        }
    }

    /// Returns the base address of the VGA memory window for this mapping mode.
    fn window_base(self) -> BxPhyAddress {
        match self {
            Self::MonoText32k => VGA_WINDOW_MONO_BASE,
            Self::ColorText32k => VGA_WINDOW_COLOR_BASE,
            Self::Vga64k | Self::Ega128k => VGA_WINDOW_GRAPHICS_BASE,
        }
    }

    /// Returns true if the given address falls within the VGA memory window for this mapping mode.
    fn contains_addr(self, addr: BxPhyAddress) -> bool {
        match self {
            Self::MonoText32k => (VGA_WINDOW_MONO_BASE..=VGA_WINDOW_MONO_END).contains(&addr),
            Self::ColorText32k => (VGA_WINDOW_COLOR_BASE..=VGA_WINDOW_COLOR_END).contains(&addr),
            Self::Vga64k => (VGA_WINDOW_GRAPHICS_BASE..=VGA_WINDOW_VGA64K_END).contains(&addr),
            Self::Ega128k => (VGA_WINDOW_GRAPHICS_BASE..=VGA_WINDOW_GRAPHICS_END).contains(&addr),
        }
    }
}

// ---- VGA memory window addresses ----
const VGA_WINDOW_MONO_BASE: BxPhyAddress = 0xB0000;
const VGA_WINDOW_MONO_END: BxPhyAddress = 0xB7FFF;
const VGA_WINDOW_COLOR_BASE: BxPhyAddress = 0xB8000;
const VGA_WINDOW_COLOR_END: BxPhyAddress = 0xBFFFF;
const VGA_WINDOW_GRAPHICS_BASE: BxPhyAddress = 0xA0000;
const VGA_WINDOW_GRAPHICS_END: BxPhyAddress = 0xBFFFF;
const VGA_WINDOW_VGA64K_END: BxPhyAddress = 0xAFFFF;

/// The physical ranges this adapter answers.
///
/// Three disjoint windows under one device, which is why the machine reports
/// which one an access landed in: Bochs decides the same question by comparing
/// the address against `vbe.base_address` and the BAR2 base, and a device that
/// routes on its own bases has to keep them in step with wherever the machine
/// actually mapped it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VgaWindow {
    /// The legacy `A0000-BFFFF` aperture — planar, latched, chain-4.
    Legacy,
    /// The linear framebuffer behind PCI BAR0.
    Lfb,
    /// The Bochs DISPI register block behind PCI BAR2.
    Registers,
}

impl VgaWindow {
    #[inline]
    pub const fn id(self) -> crate::api::WindowId {
        crate::api::WindowId(self as u8)
    }

    /// The inverse of [`Self::id`]. `None` for a window this adapter never
    /// minted a token for, which the machine cannot produce and a test can.
    #[inline]
    pub(crate) fn from_id(window: crate::api::WindowId) -> Option<Self> {
        match window.0 {
            0 => Some(Self::Legacy),
            1 => Some(Self::Lfb),
            2 => Some(Self::Registers),
            _ => None,
        }
    }
}

// ---- Misc output register bits ----
const MISC_OUT_COLOR_EMULATION: u8 = 0x01;
const MISC_OUT_ENABLE_RAM: u8 = 0x02;
const MISC_OUT_CLOCK_SEL_SHIFT: u8 = 2;
const MISC_OUT_CLOCK_SEL_MASK: u8 = 0x03;
const MISC_OUT_HIGH_BANK: u8 = 0x20;
const MISC_OUT_HORIZ_POL: u8 = 0x40;
const MISC_OUT_VERT_POL: u8 = 0x80;

// ---- Status register bits ----
const VGA_STATUS_DISPLAY_ENABLE: u8 = 0x01;
const VGA_STATUS_VERT_RETRACE: u8 = 0x08;
const VGA_STATUS_TOGGLE_MASK: u8 = VGA_STATUS_DISPLAY_ENABLE | VGA_STATUS_VERT_RETRACE;

// ---- DAC state values ----
const DAC_STATE_WRITE_MODE: u8 = 0x00;
const DAC_STATE_READ_MODE: u8 = 0x03;
const PEL_CYCLES_PER_COLOR: u8 = 3;

// ---- Register index masks ----
// Bochs vgacore.cc write: CRTC index is masked with `& 0x3f` (case 0x03d4/0x03b4).
// The Sequencer and Graphics Controller indices are stored UNMASKED; out-of-range
// DATA writes to those two are no-ops instead (see the `read_port`/`write_port`
// match arms, which guard every register-array access by valid range).
const CRTC_INDEX_MASK: u8 = 0x3F;
const ATTR_INDEX_MASK: u8 = 0x1F;

/// Text mode dimensions
const TEXT_COLS: usize = 80;
const TEXT_ROWS: usize = 25;
const BYTES_PER_CHAR: usize = 2;
const BYTES_PER_ROW: usize = TEXT_COLS * BYTES_PER_CHAR;

/// The character grid a text-mode frame is laid out on, read off the CRTC and
/// sequencer registers exactly as Bochs vgacore.cc `update()` does.
///
/// One computation with two readers — the frame renderer and the automation
/// text view — so a screen scrape can never disagree with what was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VgaTextGeometry {
    /// Character columns per row.
    pub(crate) cols: usize,
    /// Character rows on screen.
    pub(crate) rows: usize,
    /// Byte offset of the first displayed character cell.
    pub(crate) start_address: u16,
    /// Bytes between the starts of consecutive rows.
    pub(crate) line_offset: u16,
    /// Byte offset of the cursor cell, or [`Self::CURSOR_OFF`].
    pub(crate) cursor_address: u16,
    /// Pixel width of one character cell.
    pub(crate) char_width: u32,
    /// Scan lines per character cell — Bochs `MSL + 1`.
    pub(crate) char_height: u32,
    /// Displayed pixel width.
    pub(crate) pixel_width: u32,
    /// Displayed pixel height.
    pub(crate) pixel_height: u32,
}

impl VgaTextGeometry {
    /// The `cursor_address` Bochs substitutes when the cursor lies outside the
    /// displayed page.
    pub(crate) const CURSOR_OFF: u16 = 0x7FFF;
}

/// A plain VGA with the Bochs VBE extension — Bochs `bx_vga_c`, which derives
/// from `bx_vgacore_c` and adds exactly this.
///
/// It is also the PCI device: upstream is
/// `bx_vga_c : public bx_vgacore_c, public bx_pci_device_c`, and BAR0 *is* the
/// linear framebuffer base, so the config space cannot be separated from the
/// VBE state that it programs. A card that is not this one — the GeForce, or
/// one you write — composes with [`VgaCore`] directly and inherits none of it;
/// `bx_geforce_c` likewise derives from `bx_vgacore_c`, not from `bx_vga_c`,
/// and carries a config space entirely its own.
#[derive(Debug)]
pub struct StdVga {
    /// DISPI register file — resolution, depth, banking, virtual desktop.
    pub(crate) vbe: VbeState,
    /// The framebuffer size this card declares. The core allocates from it and
    /// keeps its own copy of the figure; this one is the declaration, which the
    /// PCI BAR0 size mask is derived from.
    pub(crate) vbe_memsize: u32,
    /// DDC monitor (EDID over I2C via VBE_DISPI register 0xB) — Bochs vga.h
    /// `bx_ddc_c ddc`. Its I2C state is not snapshotted; Bochs persists only
    /// `vbe.ddc_enabled` (vga.cc `register_state`).
    pub(crate) ddc: crate::display::ddc::BxDdcC,
    /// PCI configuration space (256 bytes). Only meaningful when `pci_enabled`.
    /// Bochs bx_vga_c::pci_conf. Mirrors `init_pci_conf(0x1234,0x1111,0,0x030000,0,0)`.
    pub(crate) pci_conf: [u8; 256],
    /// Whether this VGA registers as a PCI device (`1234:1111`, class `0300`).
    /// Config-gated (`[display] pci_vga`), default off. Preserved across reset.
    pub(crate) pci_enabled: bool,
    /// Committed BAR2 (VBE MMIO) base, or 0 when unmapped.
    pub(crate) mmio_base: u32,
    /// A BAR0 relocation awaiting successful LFB handler re-registration:
    /// `(old_base, new_base)`.
    pub(crate) pending_lfb_relocate: Option<(u32, u32)>,
    /// A BAR2 relocation awaiting successful MMIO handler registration. Its old
    /// base remains in `mmio_base` until commit.
    pub(crate) pending_mmio_base: Option<u32>,
}

impl Default for StdVga {
    fn default() -> Self {
        Self::new()
    }
}

impl StdVga {
    fn init_pci_conf(&mut self) {
        self.pci_conf = [0u8; 256];
        // Vendor 0x1234 / device 0x1111 (Bochs "experimental PCI VGA").
        self.pci_conf[0x00] = 0x34;
        self.pci_conf[0x01] = 0x12;
        self.pci_conf[0x02] = 0x11;
        self.pci_conf[0x03] = 0x11;
        // Command = io + mem enable; status = devsel medium.
        self.pci_conf[0x04] = 0x03;
        self.pci_conf[0x07] = 0x02;
        // Revision 0, class code 0x030000 (display controller, VGA-compatible).
        self.pci_conf[0x0A] = 0x00;
        self.pci_conf[0x0B] = 0x03;
        // BAR0 = LFB, 32-bit prefetchable memory (low nibble 0x08). Seed the base
        // to the fixed init LFB address so BAR0 is consistent before the BIOS
        // reassigns it; a differing BAR0 write relocates the framebuffer.
        let base = VBE_DISPI_LFB_PHYSICAL_ADDRESS;
        self.pci_conf[0x10] = (base as u8 & 0xf0) | 0x08;
        self.pci_conf[0x11] = (base >> 8) as u8;
        self.pci_conf[0x12] = (base >> 16) as u8;
        self.pci_conf[0x13] = (base >> 24) as u8;
        // BAR2 = VBE MMIO, 32-bit non-prefetchable memory, base 0 until assigned.
    }

    pub fn new() -> Self {
        Self {
            vbe: VbeState::default(),
            // Bochs vga.cc: 16 MB of VBE memory unless configured otherwise.
            vbe_memsize: 16 << 20,
            ddc: crate::display::ddc::BxDdcC::new(),
            pci_conf: [0u8; 256],
            pci_enabled: false,
            mmio_base: 0,
            pending_lfb_relocate: None,
            pending_mmio_base: None,
        }
    }
}

impl crate::display::card::VgaExtension for StdVga {
    fn vga_vram_bytes(&self) -> u32 {
        self.vbe_memsize
    }

    fn vga_apply_preferred_mode(&mut self, cx: &mut crate::display::card::ResetCtx<'_>) {
        self.apply_preferred_mode(cx.core());
    }

    fn vga_max_resolution(&self) -> (u32, u32) {
        (u32::from(self.vbe.max_xres), u32::from(self.vbe.max_yres))
    }

    /// A DISPI-enabled card draws its own framebuffer; otherwise the core draws
    /// the standard modes, exactly as `bx_geforce_c::update()` calls
    /// `bx_vgacore_c::update()` when the card is in a legacy mode.
    #[cfg(feature = "alloc")]
    fn vga_refresh<S: DisplaySink>(
        &mut self,
        cx: &mut crate::display::card::RefreshCtx<'_, S>,
    ) -> Option<Refreshed> {
        if self.vbe.enabled == 0 {
            return None;
        }
        let parts = cx.parts();
        Some(self.refresh_vbe_graphics(parts.core, parts.sink, parts.dirt))
    }

    /// Once DISPI is enabled at a depth other than 4bpp, video memory is the
    /// linear framebuffer and the planar core no longer answers for it. Bochs
    /// `bx_vga_c::mem_read` makes the same test before deferring to its base.
    fn vga_mem_read(&mut self, cx: &mut crate::display::card::MemCtx<'_>) -> Option<u8> {
        #[cfg(feature = "alloc")]
        {
            if self.vbe.enabled != 0 && self.vbe.bpp != VBE_DISPI_BPP_4 {
                let offset = cx.offset().get();
                return Some(match cx.window() {
                    VgaWindow::Legacy => {
                        let addr = VGA_WINDOW_GRAPHICS_BASE + offset;
                        self.vbe_mem_read_byte(cx.core(), addr)
                    }
                    VgaWindow::Lfb => {
                        let addr = self.vbe.base_address as BxPhyAddress + offset;
                        self.vbe_mem_read_byte(cx.core(), addr)
                    }
                    VgaWindow::Registers => return None,
                });
            }
        }
        let _ = cx;
        None
    }

    fn vga_mem_write(
        &mut self,
        cx: &mut crate::display::card::MemCtx<'_>,
        value: u8,
    ) -> crate::display::card::Written {
        #[cfg(feature = "alloc")]
        {
            if self.vbe.enabled != 0 && self.vbe.bpp != VBE_DISPI_BPP_4 {
                let offset = cx.offset().get();
                let addr = match cx.window() {
                    VgaWindow::Legacy => VGA_WINDOW_GRAPHICS_BASE + offset,
                    VgaWindow::Lfb => self.vbe.base_address as BxPhyAddress + offset,
                    VgaWindow::Registers => return crate::display::card::Written::FallThrough,
                };
                self.vbe_mem_write_byte(cx.core(), addr, value);
                return crate::display::card::Written::Done;
            }
        }
        let _ = (cx, value);
        crate::display::card::Written::FallThrough
    }

    /// BAR2 is this card's register block — Bochs `bx_vga_c`'s
    /// `vbe_mmio_read_handler`, which decodes to a DISPI register and formats
    /// to the access width, so it is never split into bytes.
    fn vga_regs_read(
        &mut self,
        cx: &mut crate::display::card::MemCtx<'_>,
        len: u32,
        data: &mut [u8],
    ) -> crate::display::card::Written {
        if cx.window() != VgaWindow::Registers {
            return crate::display::card::Written::FallThrough;
        }
        self.vbe_mmio_read(cx.offset(), len, data);
        crate::display::card::Written::Done
    }

    fn vga_regs_write(
        &mut self,
        cx: &mut crate::display::card::MemCtx<'_>,
        len: u32,
        data: &[u8],
    ) -> crate::display::card::Written {
        if cx.window() != VgaWindow::Registers {
            return crate::display::card::Written::FallThrough;
        }
        let at = cx.offset();
        self.vbe_mmio_write(cx.core(), at, len, data);
        crate::display::card::Written::Done
    }

    /// The DISPI index and data ports — Bochs vga.cc, not vgacore.cc.
    fn vga_pio_read(&mut self, cx: &mut crate::display::card::PortCtx<'_>) -> Option<u32> {
        match cx.port() {
            VBE_DISPI_IOPORT_INDEX => Some(u32::from(self.vbe.curindex)),
            VBE_DISPI_IOPORT_DATA => Some(u32::from(self.vbe_read_index(self.vbe.curindex))),
            _ => None,
        }
    }

    fn vga_pio_write(
        &mut self,
        cx: &mut crate::display::card::PortCtx<'_>,
        value: u32,
    ) -> crate::display::card::Written {
        match cx.port() {
            VBE_DISPI_IOPORT_INDEX => {
                self.vbe.curindex = value as u16;
                crate::display::card::Written::Done
            }
            VBE_DISPI_IOPORT_DATA => {
                let index = self.vbe.curindex;
                self.vbe_write_index(cx.core(), index, value as u16);
                crate::display::card::Written::Done
            }
            _ => crate::display::card::Written::FallThrough,
        }
    }

    /// A DISPI-enabled card reports the mode it was programmed with; otherwise
    /// the CRTC is the authority and the core answers.
    fn vga_resolution(
        &self,
        _cx: &crate::display::card::CoreRef<'_>,
    ) -> Option<crate::display::Resolution> {
        (self.vbe.enabled != 0).then(|| {
            crate::display::Resolution::new(u32::from(self.vbe.xres), u32::from(self.vbe.yres))
        })
    }

    /// The linear framebuffer is the card's window, at the card's base — Bochs
    /// registers it from `bx_vga_c`, which is what `init_vga_extension()` is
    /// for.
    fn vga_windows(
        &self,
        out: &mut crate::api::WindowDecls,
    ) -> crate::api::Declared {
        // Without an allocator there is no framebuffer to answer for — the
        // DISPI backing store is the one part of this card that needs one — so
        // the card declares only what it can serve.
        #[cfg(feature = "alloc")]
        {
            let base = u64::from(self.vbe.base_address);
            return out.push(crate::api::WindowDecl {
                id: VgaWindow::Lfb.id(),
                base,
                end: base + u64::from(self.vbe_memsize) - 1,
            });
        }
        #[cfg(not(feature = "alloc"))]
        {
            let _ = out;
            crate::api::Declared::Accepted
        }
    }

    /// Bochs `bx_vga_c::reset` runs after `bx_vgacore_c::reset`, keeping the
    /// BARs and the committed framebuffer base: they describe where the machine
    /// mapped the card, which a guest reset does not undo.
    fn vga_reset(&mut self, cx: &mut crate::display::card::ResetCtx<'_>) {
        let pci_enabled = self.pci_enabled;
        let pci_conf = self.pci_conf;
        let mmio_base = self.mmio_base;
        let lfb_base = self.vbe.base_address;
        *self = Self::new();
        self.pci_enabled = pci_enabled;
        if pci_enabled {
            self.pci_conf = pci_conf;
            self.mmio_base = mmio_base;
            self.vbe.base_address = lfb_base;
            // Bochs reset_vals: command = io+mem enable, status = devsel medium.
            self.pci_conf[0x04] = 0x03;
            self.pci_conf[0x05] = 0x00;
            self.pci_conf[0x06] = 0x00;
            self.pci_conf[0x07] = 0x02;
        }
        self.apply_preferred_mode(cx.core());
    }
}

/// VBE (Bochs VGA Extension) state, matching Bochs `bx_vga_c::vbe`.
#[derive(Debug, Clone)]
pub(crate) struct VbeState {
    /// Current DISPI ID (VBE_DISPI_ID0..ID5)
    cur_dispi: u16,
    /// LFB base address
    base_address: u32,
    /// Horizontal resolution
    xres: u16,
    /// Vertical resolution
    yres: u16,
    /// Bits per pixel
    bpp: u16,
    /// Maximum horizontal resolution (capability)
    max_xres: u16,
    /// Maximum vertical resolution (capability)
    max_yres: u16,
    /// Maximum bits per pixel (capability)
    max_bpp: u16,
    /// Bank registers [write, read]
    bank: [u16; 2],
    /// Bank granularity in KB
    bank_granularity_kb: u16,
    /// VBE enabled flag
    enabled: u16,
    /// Current VBE index register
    curindex: u16,
    /// Visible screen size in bytes
    visible_screen_size: u32,
    /// Virtual screen X offset in pixels
    offset_x: u16,
    /// Virtual screen Y offset in pixels
    offset_y: u16,
    /// Virtual horizontal resolution
    virtual_xres: u16,
    /// Virtual vertical resolution
    virtual_yres: u16,
    /// Virtual screen start offset (for bpp>8)
    virtual_start: u32,
    /// BPP multiplier
    bpp_multiplier: u8,
    /// Line offset in bytes
    line_offset: u16,
    /// Get-capabilities mode active
    get_capabilities: bool,
    /// 8-bit DAC mode
    dac_8bit: bool,
    /// DDC enabled
    ddc_enabled: bool,
}

impl Default for VbeState {
    fn default() -> Self {
        Self {
            cur_dispi: VBE_DISPI_ID0,
            base_address: VBE_DISPI_LFB_PHYSICAL_ADDRESS,
            xres: 640,
            yres: 480,
            bpp: 8,
            max_xres: 1600,
            max_yres: 1200,
            max_bpp: 32,
            bank: [0; 2],
            bank_granularity_kb: 64,
            enabled: 0,
            curindex: 0,
            visible_screen_size: 0,
            offset_x: 0,
            offset_y: 0,
            virtual_xres: 640,
            virtual_yres: 480,
            virtual_start: 0,
            bpp_multiplier: 1,
            line_offset: 640,
            get_capabilities: false,
            dac_8bit: false,
            ddc_enabled: false,
        }
    }
}

/// The desired VGA BAR bases decoded from a snapshot.
///
/// The decoder deliberately leaves the live handler identity committed at its
/// existing bases.  The machine-level restore path must relocate the handlers
/// atomically, then call [`VgaCore::commit_snapshot_v3_mapping_target`].
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VgaSnapshotRestoreTarget {
    pub lfb_base: u32,
    pub mmio_base: u32,
}

#[cfg(feature = "std")]
#[derive(Clone, Copy)]
struct VgaSnapshotVbeState {
    cur_dispi: u16,
    max_xres: u16,
    max_yres: u16,
    max_bpp: u16,
    xres: u16,
    yres: u16,
    bpp: u16,
    bank: [u16; 2],
    bank_granularity_kb: u16,
    enabled: u16,
    curindex: u16,
    offset_x: u16,
    offset_y: u16,
    virtual_xres: u16,
    virtual_yres: u16,
    get_capabilities: bool,
    dac_8bit: bool,
    ddc_enabled: bool,
}

#[cfg(feature = "std")]
impl From<&VbeState> for VgaSnapshotVbeState {
    fn from(vbe: &VbeState) -> Self {
        Self {
            cur_dispi: vbe.cur_dispi,
            max_xres: vbe.max_xres,
            max_yres: vbe.max_yres,
            max_bpp: vbe.max_bpp,
            xres: vbe.xres,
            yres: vbe.yres,
            bpp: vbe.bpp,
            bank: vbe.bank,
            bank_granularity_kb: vbe.bank_granularity_kb,
            enabled: vbe.enabled,
            curindex: vbe.curindex,
            offset_x: vbe.offset_x,
            offset_y: vbe.offset_y,
            virtual_xres: vbe.virtual_xres,
            virtual_yres: vbe.virtual_yres,
            get_capabilities: vbe.get_capabilities,
            dac_8bit: vbe.dac_8bit,
            ddc_enabled: vbe.ddc_enabled,
        }
    }
}

/// VGA controller state.
///
/// Public only as an identity: it names the adapter a `StandardPc` machine
/// drives, so [`crate::display::Display`] can be spelled without erasing it.
/// Every field and inherent method stays crate-private — the surface a caller
/// gets is the role traits the adapter implements, and nothing else.
#[derive(Debug)]
pub struct VgaCore {
    /// CRTC index register
    crtc_index: u8,
    /// CRTC registers (25 registers)
    crtc_regs: [u8; 25],
    /// Attribute controller index
    attr_index: u8,
    /// Attribute controller flip-flop (toggles between index and data)
    attr_flip_flop: bool,
    /// Attribute controller registers
    attr_regs: [u8; 21],
    /// Sequencer index
    seq_index: u8,
    /// Sequencer registers
    seq_regs: [u8; 5],
    /// Graphics controller index
    graphics_index: u8,
    /// Graphics controller registers
    pub graphics_regs: [u8; 9],
    /// Status register value
    status_reg: u8,
    /// Misc output register
    misc_output: u8,
    /// VGA text aperture backing store (Bochs: `s.memory` aliased by mapping window).
    ///
    /// Bochs does *not* keep separate B0000 vs B8000 buffers; instead, the Graphics
    /// Controller `memory_mapping` selects which address range maps to the same memory.
    /// See `cpp_orig/bochs/iodev/display/vgacore.cc` `mem_read`/`mem_write` mapping switch.
    text_memory: [u8; VGA_TEXT_MEM_SIZE],
    /// Current cursor position (row, col)
    cursor_pos: (usize, usize),
    /// Flag indicating text memory has changed (dirty)
    text_dirty: bool,
    /// Text buffer for GUI updates (new state)
    /// This is extracted from text_memory when update() is called
    text_buffer: [u8; VGA_TEXT_MEM_SIZE],
    /// Text snapshot for comparison (old state)
    /// Used to detect what changed between updates
    text_snapshot: [u8; VGA_TEXT_MEM_SIZE],
    /// Flag indicating VGA memory has been updated (matching vgacore.cc vga_mem_updated)
    vga_mem_updated: u8,
    /// Flag indicating text buffer needs to be updated from VGA memory
    /// Set when text mode parameters change
    text_buffer_update: bool,


    // =====================================================================
    // VGA Enable and PEL/DAC registers (ports 0x3C3, 0x3C6-0x3C9)
    // See vgacore.cc state variables in bx_vgacore_s struct
    // =====================================================================
    /// VGA enable (port 0x3C3) - bit 0 enables VGA display
    vga_enabled: bool,

    /// Feature Control register: written via port 0x3BA/0x3DA (mono/color
    /// emulation), read back at 0x3CA. Only bit 3 is retained.
    /// Bochs: `s.feature_control` (vgacore.h); write `feature_control = value & 0x08`
    /// and read `RETURN(s.feature_control)` (vgacore.cc).
    feature_control: u8,

    /// CRTC start address latched for the current frame.
    /// Bochs: `s.CRTC.start_addr`, refreshed in `vertical_timer()` from CRTC
    /// registers 0x0C/0x0D — the write handlers deliberately do nothing, so a
    /// mid-frame change cannot tear the picture.
    crtc_start_addr: u16,

    /// Host microsecond stamp of the last vertical retrace, used as the phase
    /// anchor for the 0x3DA status register.
    /// Bochs: `s.display_start_usec`, re-anchored in `vertical_timer()`.
    display_start_usec: u64,

    /// DAC entries whose colour changed and have not yet been published to the
    /// GUI. Bochs calls `bx_gui->palette_change_common(index, r << dac_shift,
    /// ...)` synchronously from the PEL data write (vgacore.cc); the GUI is not
    /// reachable from here, so the indices are queued and drained at the frame
    /// boundary. A snapshot restore marks every entry, which republishes the
    /// whole table (Bochs vgacore.cc `after_restore_state`).
    dac_dirty: [bool; PEL_COLOR_COUNT],
    dac_any_dirty: bool,

    /// Sequencer "screen off / clear screen" request (register 1 bit 5).
    /// Bochs: `s.sequencer.clear_screen` (vgacore.h), raised in the register-1
    /// write and consumed by `skip_update()`.
    seq_clear_screen: bool,

    /// A `clear_screen()` owed to the GUI. `skip_update` returns early, so this
    /// carries Bochs's `bx_gui->clear_screen()` call out to the frontend even on
    /// frames that produce no update result.
    pending_clear_screen: bool,

    /// Plane-2 offsets of the two selectable character generators, derived from
    /// sequencer register 3 through `CHARMAP_OFFSET`.
    /// Bochs: `s.charmap_address1` / `s.charmap_address2` (vgacore.h).
    charmap_address1: u16,
    charmap_address2: u16,

    /// The two extracted character generators (8KB each = 256 glyphs x 32
    /// bytes). Bochs keeps these on the GUI side (`bx_gui_c::vga_charmap[2]`,
    /// filled by `update_charmap()` -> `set_text_charmap`); here the device owns
    /// the extraction and the GUI copies them when `charmap_updated` is set.
    /// Derived entirely from planar memory + the two addresses, so they are not
    /// snapshotted — a restore re-extracts them.
    charmap: [[u8; CHARMAP_SIZE]; 2],

    /// Doubled scanlines in classic graphics modes, derived from CRTC register
    /// 0x09 (Maximum Scan Line). Bochs: `s.y_doublescan = ((value & 0x9f) > 0)`
    /// (vgacore.cc CRTC write case 0x09); consumed when rendering rows and when
    /// halving the line-compare (split screen).
    y_doublescan: bool,

    /// PEL mask register (port 0x3C6)
    pel_mask: u8,
    /// Left-shift applied to a DAC entry on the way to the front end: 2 for the
    /// standard VGA's 6-bit DAC, 0 once a card widens it to 8. Bochs keeps
    /// `s.dac_shift` on `bx_vgacore_c` and lets `bx_vga_c` set it, because the
    /// renderer needs it and the DISPI register that changes it does not.
    pub(crate) dac_shift: u8,

    /// DAC state (port 0x3C7 read): 0x00 = write mode, 0x03 = read mode
    dac_state: u8,

    /// PEL write address register (port 0x3C8)
    pel_write_addr: u8,

    /// PEL read address register (port 0x3C7 write)
    pel_read_addr: u8,

    /// PEL write cycle counter (0, 1, 2 for R, G, B)
    pel_write_cycle: u8,

    /// PEL read cycle counter (0, 1, 2 for R, G, B)
    pel_read_cycle: u8,

    /// PEL data (256 colors × [R, G, B])
    pel_data: [[u8; 3]; 256],

    // =====================================================================
    // Misc output register parsed fields (for easier access)
    // Written via port 0x3C2, read via port 0x3CC
    // =====================================================================
    /// Bit 0: color_emulation - 1=color (CRTC at 0x3D4), 0=mono (CRTC at 0x3B4)
    misc_color_emulation: bool,

    /// Bit 1: enable_ram - 1=VGA memory access enabled
    misc_enable_ram: bool,

    /// Bits 2-3: clock_select
    misc_clock_select: u8,

    /// Bit 5: select_high_bank (ODD/EVEN page select)
    misc_select_high_bank: bool,

    /// Bit 6: horiz_sync_pol - horizontal sync polarity
    misc_horiz_sync_pol: bool,

    /// Bit 7: vert_sync_pol - vertical sync polarity
    misc_vert_sync_pol: bool,

    /// Sequencer chain-four mode (seq reg 4 bit 3, Bochs vgacore.cc)
    pub(crate) seq_chain_four: bool,
    /// Sequencer odd/even disable (seq reg 4 bit 2, Bochs vgacore.cc)
    pub(crate) seq_odd_even_dis: bool,

    // =====================================================================
    // VGA planar memory and graphics latch (Bochs vgacore.cc)
    // =====================================================================
    /// VGA planar memory (256KB). Layout: memory[offset * 4 + plane]
    /// Matches Bochs `s.memory` with `s.memsize = 0x40000`.
    vga_memory: [u8; VGA_MEM_SIZE],

    /// Graphics controller latch register (one byte per plane).
    /// Loaded on every VGA memory read. Used by write modes 0-3.
    /// Matches Bochs `s.graphics_ctrl.latch[4]`.
    pub(crate) latch: [u8; 4],

    // =====================================================================
    // Retrace timing (Bochs vgacore.cc calculate_retrace_timing)
    // =====================================================================
    /// Horizontal total period in microseconds (Bochs s.htotal_usec)
    htotal_usec: u32,
    /// Horizontal blanking start in microseconds (Bochs s.hbstart_usec)
    hbstart_usec: u32,
    /// Horizontal blanking end in microseconds (Bochs s.hbend_usec)
    hbend_usec: u32,
    /// Vertical total period in microseconds (Bochs s.vtotal_usec)
    vtotal_usec: u32,
    /// Vertical blanking start in microseconds (Bochs s.vblank_usec)
    vblank_usec: u32,
    /// Vertical retrace start in microseconds (Bochs s.vrstart_usec)
    vrstart_usec: u32,
    /// Vertical retrace end in microseconds (Bochs s.vrend_usec)
    vrend_usec: u32,

    /// Whether icount-based timing has been initialized.
    /// When false, falls back to toggle behavior for retrace.
    has_icount_sync: bool,
    /// Instructions per second, used to convert icount to microseconds.
    ips: u64,

    /// Attribute controller: video_enabled (PAS = Palette Address Source)
    /// Bit 5 of the value written to port 0x3C0 when flip_flop=0
    /// Bochs: s.attribute_ctrl.video_enabled
    video_enabled: bool,

    // =====================================================================
    // VBE (Bochs VGA Extension) state
    // =====================================================================
    #[cfg(feature = "alloc")]
    /// Dirty state for Bochs 16x24 graphics tiles.
    vga_tile_updated: Vec<bool>,
    /// Total linear-framebuffer size in bytes, as the card declared it.
    pub(crate) vbe_memsize: u32,
    /// Whether planar accesses address the linear framebuffer instead of the
    /// planar array. A DISPI mode at 4bpp banks the planar path into the
    /// framebuffer, so the two are one store; only the card knows when, so the
    /// card sets it.
    pub(crate) vbe_planar_alias: bool,
    /// The linear framebuffer. Owned here because it is video memory and the
    /// core owns video memory — and because a DISPI mode at 4bpp addresses the
    /// planar path into these same bytes with a bank offset, so the two cannot
    /// live in different objects.
    #[cfg(feature = "alloc")]
    pub(crate) vbe_memory: Vec<u8>,
    /// Number of horizontal graphics tiles.
    num_x_tiles: u16,
    /// Number of vertical graphics tiles.
    num_y_tiles: u16,
    /// Bochs extension offset added to legacy VGA memory offsets.
    ext_offset: u32,
    /// Bochs extension offset added to legacy VGA memory read offsets.
    ext_read_offset: u32,
    /// Active VGA memory mask (0x3ffff in legacy VGA, VBE memory size - 1 in VBE).
    vga_mem_mask: u32,
    /// Bochs extension start address added to CRTC start address.
    ext_start_addr: u32,
    /// Bochs extension vertical double-size flag.
    ext_y_dblsize: bool,

    // =====================================================================
    // Dimension tracking (matching Bochs vgacore.cc s.last_xres etc.)
    // Used to detect when dimension_update needs to be called on the GUI.
    // =====================================================================
    last_xres: u32,
    last_yres: u32,
    last_fw: u32,
    last_fh: u32,
    last_bpp: u32,

    /// Optional pre-boot VBE mode (xres, yres, bpp). When set, raises the DISPI
    /// capability ceiling and seeds the power-on VBE dimensions. Preserved across
    /// `reset()`.
    preferred_mode: Option<(u16, u16, u16)>,

}

impl Default for VgaCore {
    fn default() -> Self {
        Self::new()
    }
}

impl VgaCore {
    /// Video memory a standard VGA has — Bochs `bx_vgacore_c` allocates this
    /// much when no extension asks for more.
    pub const LEGACY_VRAM_BYTES: usize = VGA_MEM_SIZE;

    /// The largest picture a standard VGA can display — mode 12h, 640x480.
    pub const LEGACY_MAX_XRES: u32 = 640;
    /// See [`Self::LEGACY_MAX_XRES`].
    pub const LEGACY_MAX_YRES: u32 = 480;

    /// Size the dirty-tile grid for the largest mode the card can reach.
    ///
    /// Called once, by the card, before anything can dirty a tile: the grid's
    /// stride is baked into every tile index, so this is not a runtime knob.
    /// Bochs sizes the same arrays in `bx_vgacore_c::init` from the maxima the
    /// derived class set before calling it.
    /// Allocate video memory to the size the card declares — Bochs
    /// `bx_vgacore_c::init`, which allocates only what its extension asked for.
    pub(crate) fn size_vram(&mut self, bytes: u32) {
        self.vbe_memsize = bytes;
        #[cfg(feature = "alloc")]
        {
            self.vbe_memory = alloc::vec![0; bytes as usize];
        }
    }

    pub(crate) fn size_tile_grid_for(&mut self, max_xres: u32, max_yres: u32) {
        self.num_x_tiles = max_xres.div_ceil(VGA_X_TILESIZE) as u16;
        self.num_y_tiles = max_yres.div_ceil(VGA_Y_TILESIZE) as u16;
        #[cfg(feature = "alloc")]
        {
            self.vga_tile_updated =
                alloc::vec![false; self.num_x_tiles as usize * self.num_y_tiles as usize];
        }
    }

    /// Create a new VGA controller
    pub(crate) fn new() -> Self {
        // The dirty-tile grid is a placeholder until the card sizes it for the
        // largest mode it can reach — `VgaCard::with_extension` does that before
        // anything can dirty a tile.
        let num_x_tiles = Self::LEGACY_MAX_XRES.div_ceil(VGA_X_TILESIZE) as u16;
        let num_y_tiles = Self::LEGACY_MAX_YRES.div_ceil(VGA_Y_TILESIZE) as u16;
        let mut vga = Self {
            crtc_index: 0,
            crtc_regs: [0; 25],
            attr_index: 0,
            attr_flip_flop: false,
            attr_regs: [0; 21],
            seq_index: 0,
            // Bochs init_standard_vga(): s.sequencer.reset1 = reset2 = 1, which
            // reads back from sequencer register 0 as 0x03. skip_update() gates
            // on both, so they must start released.
            seq_regs: [0x03, 0, 0, 0, 0],
            graphics_index: 0,
            graphics_regs: [0; 9],
            status_reg: 0x00,
            // Bochs init_standard_vga(): color_emulation=1, enable_ram=1,
            // horiz_sync_pol=1, vert_sync_pol=1, clock_select=0, select_high_bank=0
            // = 0b11000011 = 0xC3
            misc_output: 0xC3,
            text_memory: [0u8; VGA_TEXT_MEM_SIZE],
            cursor_pos: (0, 0),
            text_dirty: false,
            // Bochs keeps text buffers sized for the whole aperture (0x8000 for mapping 2/3).
            text_buffer: [0u8; VGA_TEXT_MEM_SIZE],
            text_snapshot: [0u8; VGA_TEXT_MEM_SIZE],
            vga_mem_updated: 0,
            text_buffer_update: true, // Initial update needed

            // VGA Enable and PEL/DAC registers
            vga_enabled: true, // VGA enabled by default
            // Bochs init_standard_vga(): s.feature_control = 0
            feature_control: 0,
            crtc_start_addr: 0,
            display_start_usec: 0,
            dac_dirty: [false; PEL_COLOR_COUNT],
            dac_any_dirty: false,
            seq_clear_screen: false,
            pending_clear_screen: false,
            charmap_address1: 0,
            charmap_address2: 0,
            charmap: [[0u8; CHARMAP_SIZE]; 2],
            y_doublescan: false,
            pel_mask: 0xFF,
            dac_shift: DAC_SHIFT,
            dac_state: 0x01,   // Initial state
            pel_write_addr: 0,
            pel_read_addr: 0,
            pel_write_cycle: 0,
            pel_read_cycle: 0,
            pel_data: [[0; 3]; 256], // Will be initialized by BIOS

            // Misc output parsed fields (matching misc_output = 0xC3)
            // Bochs init_standard_vga(): color_emulation=1, enable_ram=1,
            // clock_select=0, select_high_bank=0, horiz_sync_pol=1, vert_sync_pol=1
            misc_color_emulation: true, // Bit 0: color mode (use 0x3D4/0x3D5)
            misc_enable_ram: true,      // Bit 1: RAM enabled
            misc_clock_select: 0,       // Bits 2-3: Bochs default = 0
            misc_select_high_bank: false, // Bit 5: Bochs default = 0
            misc_horiz_sync_pol: true,  // Bit 6: Bochs = 1
            misc_vert_sync_pol: true,   // Bit 7: Bochs = 1

            seq_chain_four: false,
            seq_odd_even_dis: false,

            // VGA planar memory and latch
            vga_memory: [0u8; VGA_MEM_SIZE],
            latch: [0u8; 4],

            // Retrace timing defaults (matching Bochs vgacore.cc)
            htotal_usec: 31,
            hbstart_usec: 25,
            hbend_usec: 28,
            vtotal_usec: 14268,
            vblank_usec: 12688,
            vrstart_usec: 13000,
            vrend_usec: 13155,

            has_icount_sync: false,
            ips: 15_000_000, // Default 15 MIPS

            // Bochs init_standard_vga(): s.attribute_ctrl.video_enabled = 1.
            // skip_update() gates on this, so a `false` default would blank the
            // console until the guest first wrote 0x3C0.
            video_enabled: true,

            #[cfg(feature = "alloc")]
            vga_tile_updated: vec![true; num_x_tiles as usize * num_y_tiles as usize],
            num_x_tiles,
            num_y_tiles,
            ext_offset: 0,
            ext_read_offset: 0,
            vga_mem_mask: (VGA_MEM_SIZE - 1) as u32,
            ext_start_addr: 0,
            ext_y_dblsize: false,

            last_xres: 0,
            last_yres: 0,
            last_fw: 0,
            last_fh: 0,
            last_bpp: 8, // Bochs: s.last_bpp = 8
            preferred_mode: None,
            vbe_memsize: 0,
            vbe_planar_alias: false,
            #[cfg(feature = "alloc")]
            vbe_memory: Vec::new(),
        };

        // CRTC registers: Bochs zeroes them via memset; the VGA BIOS programs them.
        // No explicit initialization needed — array is already zeroed above.

        // Initialize sequencer — only fields explicitly set by Bochs init_standard_vga()
        vga.seq_regs[SEQ_REG_RESET] = 0x03; // reset1=1, reset2=1
                                            // seq_regs[1..3] stay 0 from array init (Bochs: zeroed by memset)
                                            // Bochs: extended_mem=1 (bit 1) + odd_even_dis=1 (bit 2) = 0x06
        vga.seq_regs[SEQ_REG_MEMORY_MODE] = 0x06;

        // Initialize graphics controller — only fields explicitly set by Bochs
        // All regs 0 from array init except memory_mapping=2 in GFX_REG_MISC
        // Bochs init_standard_vga(): graphics_ctrl.memory_mapping = 2
        vga.graphics_regs[GFX_REG_MISC] = 0x08; // memory_mapping=2 (bits 2-3)
                                                // graphics_regs[0..5,7,8] stay 0 from array init (Bochs: zeroed by memset)

        // Initialize attribute controller
        // Bochs: palette regs 0-15 are zeroed by memset (not explicitly set)
        // They get programmed by the BIOS during VGA init
        // Bochs init_standard_vga() attribute_ctrl fields:
        //   mode_ctrl.enable_line_graphics = 1 (bit 2 of reg 0x10)
        //   color_plane_enable = 0x0f (reg 0x12)
        //   All others stay 0 from memset
        vga.attr_regs[ATTR_REG_MODE_CONTROL] = 0x04;
        vga.attr_regs[ATTR_REG_COLOR_PLANE_EN] = 0x0F;
        // attr_regs[0x11, 0x13, 0x14] stay 0 from array init

        vga
    }

    /// The ports this adapter answers on, as Bochs `bx_vgacore_c::init`
    /// registers them — byte and word (`mask` 0x3), word writes split into two
    /// byte writes in `write_port`.
    ///
    /// Stated rather than registered: a display model that called the I/O bus
    /// from its own `init` would need the bus to exist before the model does.
    pub(crate) const PORTS: &'static [crate::api::PortDecl] = {
        use crate::api::PortDecl;
        const fn p(port: u16, name: &'static str) -> PortDecl {
            PortDecl {
                port,
                name,
                widths: 0x3,
            }
        }
        &[
            p(VGA_CRTC_INDEX_MONO, "VGA CRTC Index (mono)"),
            p(VGA_CRTC_DATA_MONO, "VGA CRTC Data (mono)"),
            p(VGA_CRTC_INDEX, "VGA CRTC Index"),
            p(VGA_CRTC_DATA, "VGA CRTC Data"),
            p(VGA_STATUS, "VGA Status"),
            p(VGA_STATUS_MONO, "VGA Status (mono)"),
            p(VGA_ATTRIB_ADDR, "VGA Attribute Address"),
            p(VGA_ATTRIB_DATA, "VGA Attribute Data"),
            p(VGA_SEQ_INDEX, "VGA Sequencer Index"),
            p(VGA_SEQ_DATA, "VGA Sequencer Data"),
            p(VGA_GRAPHICS_INDEX, "VGA Graphics Index"),
            p(VGA_GRAPHICS_DATA, "VGA Graphics Data"),
            p(VGA_MISC_OUTPUT, "VGA Misc Output Read"),
            p(VGA_MISC_OUTPUT_WRITE, "VGA Misc Output Write"),
            p(VGA_ENABLE, "VGA Enable"),
            p(VGA_PEL_MASK, "VGA PEL Mask"),
            p(VGA_DAC_STATE, "VGA DAC State"),
            p(VGA_PEL_ADDR_WRITE, "VGA PEL Address Write"),
            p(VGA_PEL_DATA, "VGA PEL Data"),
            p(VBE_DISPI_IOPORT_INDEX, "Bochs VBE Index"),
            p(VBE_DISPI_IOPORT_DATA, "Bochs VBE Data"),
            p(0x3CA, "VGA EGA Compat"),
            p(0x3CB, "VGA EGA Compat"),
            p(0x3CD, "VGA EGA Compat"),
        ]
    };

    /// The physical window the core answers on, independent of any card: the
    /// legacy aperture Bochs registers in `bx_vgacore_c::init`.
    pub(crate) fn core_window() -> crate::api::WindowDecl {
        crate::api::WindowDecl {
            id: VgaWindow::Legacy.id(),
            base: VGA_WINDOW_GRAPHICS_BASE as u64,
            end: VGA_WINDOW_GRAPHICS_END as u64,
        }
    }


    /// Reset the standard VGA — Bochs `bx_vgacore_c::reset`, which the card's
    /// own reset calls before doing its half.
    pub(crate) fn reset(&mut self) {
        // Configuration survives a reset; it describes the machine, not the
        // adapter's programming.
        let has_icount_sync = self.has_icount_sync;
        let ips = self.ips;
        let preferred_mode = self.preferred_mode;
        *self = Self::new();
        self.has_icount_sync = has_icount_sync;
        self.ips = ips;
        self.preferred_mode = preferred_mode;
    }

    /// Set the pre-boot VBE mode: raise the DISPI capability ceiling so the guest
    /// may select up to this resolution, and seed the power-on dimensions.
    /// Preserved across `reset()`. Mirrors the DISPI MAX_XRES/MAX_YRES/MAX_BPP
    /// capability registers Bochs exposes (vga.cc). Reallocates the dirty-tile
    /// grid when the ceiling grows.
    pub(crate) fn set_preferred_mode(&mut self, xres: u16, yres: u16, bpp: u16) {
        self.preferred_mode = Some((xres, yres, bpp));
    }


    /// Initialize icount-based timing for retrace computation.
    /// Must be called after CPU initialization.
    pub(crate) fn set_icount_sync(&mut self, ips: u64) {
        self.has_icount_sync = true;
        self.ips = if ips > 0 { ips } else { 15_000_000 };
    }

}


impl VgaCore {








    /// Calculate retrace timing from CRTC registers.
    /// Matches Bochs vgacore.cc `calculate_retrace_timing()`.
    fn calculate_retrace_timing(&mut self) {
        // get_crtc_params (Bochs vgacore.cc)
        let clock_select = self.misc_clock_select as usize;
        let mut vclock = VGA_VCLK[clock_select.min(3)];
        let x_dotclockdiv2 = (self.seq_regs[SEQ_REG_CLOCKING_MODE] & 0x08) != 0;
        if x_dotclockdiv2 {
            vclock >>= 1;
        }
        if vclock == 0 {
            return; // Invalid clock
        }

        // Character width: 8 or 9 dots
        let cwidth: u32 = if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & 0x01) != 0 {
            8
        } else {
            9
        };

        // htotal from CRTC reg 0 + 5 (Bochs get_crtc_params)
        let htotal = self.crtc_regs[0x00] as u32 + 5;
        // vtotal from CRTC regs 6 + overflow bits in reg 7
        let vtotal = self.crtc_regs[0x06] as u32
            + ((self.crtc_regs[0x07] as u32 & 0x01) << 8)
            + ((self.crtc_regs[0x07] as u32 & 0x20) << 4)
            + 2;
        // vbstart from CRTC regs 0x15 + overflow bits
        let vbstart = self.crtc_regs[0x15] as u32
            + ((self.crtc_regs[0x07] as u32 & 0x08) << 5)
            + ((self.crtc_regs[0x09] as u32 & 0x20) << 4);
        // vrstart from CRTC regs 0x10 + overflow bits
        let vrstart = self.crtc_regs[0x10] as u32
            + ((self.crtc_regs[0x07] as u32 & 0x04) << 6)
            + ((self.crtc_regs[0x07] as u32 & 0x80) << 2);

        // vrend from CRTC reg 0x11 low 4 bits, relative to vrstart
        let vrend_raw = ((self.crtc_regs[0x11] as u32 & 0x0F).wrapping_sub(vrstart)) & 0x0F;
        let vrend = vrstart + vrend_raw;

        // Horizontal frequency and period
        let hfreq = vclock as f32 / (htotal * cwidth) as f32;
        let f_htotal_usec = 1_000_000.0f32 / hfreq;
        self.htotal_usec = f_htotal_usec as u32;

        // Horizontal blanking
        let hbstart = self.crtc_regs[0x02] as u32;
        self.hbstart_usec = ((1_000_000.0 * hbstart as f64 * cwidth as f64) / vclock as f64) as u32;
        let hbend_raw =
            (self.crtc_regs[0x03] as u32 & 0x1F) + ((self.crtc_regs[0x05] as u32 & 0x80) >> 2);
        let hbend = hbstart + ((hbend_raw.wrapping_sub(hbstart)) & 0x3F);
        self.hbend_usec = ((1_000_000.0 * hbend as f64 * cwidth as f64) / vclock as f64) as u32;

        // Vertical frequency and period
        if vtotal > 0 {
            let vfreq = hfreq / vtotal as f32;
            if vfreq > 0.0 {
                self.vtotal_usec = (1_000_000.0f32 / vfreq) as u32;
            }
        }
        self.vblank_usec = (f_htotal_usec * vbstart as f32) as u32;
        self.vrstart_usec = (f_htotal_usec * vrstart as f32) as u32;
        self.vrend_usec = (f_htotal_usec * vrend as f32) as u32;

        // Sanity clamps matching Bochs vgacore.cc
        if self.vtotal_usec < 8000 {
            self.vtotal_usec = 14268;
        }
        if self.vrend_usec < 7000 {
            self.vrend_usec = self.vtotal_usec.saturating_sub(1113);
        }
    }

    /// Emulated microseconds at the access, for the retrace computation.
    ///
    /// The conversion is the clock's own — the card no longer keeps a rate to
    /// divide by. `has_icount_sync` still gates it: until the machine has told
    /// the card its rate, Bochs reports no elapsed time and so does this.
    fn current_usec(&self, clock: VmClock) -> u64 {
        if !self.has_icount_sync {
            return 0;
        }
        clock.micros()
    }

    /// Initialize VGA to standard text mode 3 (80x25 color text).
    /// Used for direct kernel boot where no BIOS/VGA BIOS runs.
    /// Programs CRTC, Sequencer, Graphics, and Attribute registers to
    /// standard mode 3 values so the kernel's vgacon driver works.
    pub(crate) fn init_text_mode3(&mut self) {
        // Standard VGA mode 3 CRTC register values (80x25, 16-pixel font, 400 scanlines)
        let crtc_mode3: [u8; 25] = [
            0x5F, // 00: Horizontal Total
            0x4F, // 01: Horizontal Display End (80 columns - 1 = 79)
            0x50, // 02: Start Horizontal Blanking
            0x82, // 03: End Horizontal Blanking
            0x55, // 04: Start Horizontal Retrace
            0x81, // 05: End Horizontal Retrace
            0xBF, // 06: Vertical Total
            0x1F, // 07: Overflow (VDE bit 8 = 1, bit 9 from 0x40)
            0x00, // 08: Preset Row Scan
            0x4F, // 09: Maximum Scan Line (16-1=15, bit 6=0x40 for VDE bit 9)
            0x0D, // 0A: Cursor Start (line 13)
            0x0E, // 0B: Cursor End (line 14)
            0x00, // 0C: Start Address High
            0x00, // 0D: Start Address Low
            0x00, // 0E: Cursor Location High
            0x00, // 0F: Cursor Location Low
            0x9C, // 10: Vertical Retrace Start
            0x8E, // 11: Vertical Retrace End
            0x8F, // 12: Vertical Display End (400-1=399 low 8 bits)
            0x28, // 13: Offset (80/2 = 40)
            0x1F, // 14: Underline Location
            0x96, // 15: Start Vertical Blanking
            0xB9, // 16: End Vertical Blanking
            0xA3, // 17: Mode Control
            0xFF, // 18: Line Compare
        ];
        self.crtc_regs[..25].copy_from_slice(&crtc_mode3);

        // Sequencer registers for mode 3
        self.seq_regs[0] = 0x03; // Reset: both resets deasserted
        self.seq_regs[1] = 0x00; // Clocking Mode: 9-dot chars, no shift
        self.seq_regs[2] = 0x03; // Map Mask: planes 0+1 enabled (text)
        self.seq_regs[3] = 0x00; // Character Map Select: font A=B=0
        self.seq_regs[4] = 0x02; // Memory Mode: extended memory, odd/even

        // Graphics controller for color text mode
        self.graphics_regs[0] = 0x00; // Set/Reset
        self.graphics_regs[1] = 0x00; // Enable Set/Reset
        self.graphics_regs[2] = 0x00; // Color Compare
        self.graphics_regs[3] = 0x00; // Data Rotate
        self.graphics_regs[4] = 0x00; // Read Map Select
        self.graphics_regs[5] = 0x10; // Mode: odd/even addressing
        self.graphics_regs[6] = 0x0E; // Misc: color text mode (bits 2-3=11), not graphics
        self.graphics_regs[7] = 0x00; // Color Don't Care
        self.graphics_regs[8] = 0xFF; // Bit Mask

        // Attribute controller for mode 3 (standard 16-color palette + mode)
        // Palette registers 0-15: standard EGA/VGA color mapping
        let palette: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
            0x3E, 0x3F,
        ];
        self.attr_regs[..16].copy_from_slice(&palette);
        self.attr_regs[0x10] = 0x0C; // Mode Control: blink enable + line graphics
        self.attr_regs[0x11] = 0x00; // Overscan Color
        self.attr_regs[0x12] = 0x0F; // Color Plane Enable: all 4 planes
        self.attr_regs[0x13] = 0x08; // Horizontal Pixel Panning
        self.attr_regs[0x14] = 0x00; // Color Select

        // Misc output register fields
        self.misc_color_emulation = true;
        self.misc_enable_ram = true;
        self.misc_clock_select = 0;
        self.misc_horiz_sync_pol = true;
        self.misc_vert_sync_pol = false; // 400-line mode (negative vsync)

        // Enable video output
        self.video_enabled = true;

        // Initialize standard VGA DAC palette (first 16 entries for text mode)
        let dac_colors: [[u8; 3]; 16] = [
            [0x00, 0x00, 0x00], // 0: black
            [0x00, 0x00, 0x2A], // 1: blue
            [0x00, 0x2A, 0x00], // 2: green
            [0x00, 0x2A, 0x2A], // 3: cyan
            [0x2A, 0x00, 0x00], // 4: red
            [0x2A, 0x00, 0x2A], // 5: magenta
            [0x2A, 0x15, 0x00], // 6: brown
            [0x2A, 0x2A, 0x2A], // 7: light gray
            [0x15, 0x15, 0x15], // 8: dark gray
            [0x15, 0x15, 0x3F], // 9: light blue
            [0x15, 0x3F, 0x15], // A: light green
            [0x15, 0x3F, 0x3F], // B: light cyan
            [0x3F, 0x15, 0x15], // C: light red
            [0x3F, 0x15, 0x3F], // D: light magenta
            [0x3F, 0x3F, 0x15], // E: yellow
            [0x3F, 0x3F, 0x3F], // F: white
        ];
        for (i, color) in dac_colors.iter().enumerate() {
            self.pel_data[i] = *color;
        }
        // Also set entries for bright colors (palette indices 0x38-0x3F)
        self.pel_data[0x38..0x40].copy_from_slice(&dac_colors[8..16]);

        // Force text buffer refresh
        self.text_buffer_update = true;
        self.vga_mem_updated = 1;
    }

    /// Read from I/O port
    pub(crate) fn read_port(&mut self, port: u16, io_len: u8, clock: VmClock) -> u32 {
        // Bochs vgacore.cc: port gating based on color_emulation
        if (0x3B0..=0x3BF).contains(&port) && self.misc_color_emulation {
            return 0xFF; // mono ports disabled in color mode
        }
        if (0x3D0..=0x3DF).contains(&port) && !self.misc_color_emulation {
            return 0xFF; // color ports disabled in mono mode
        }
        // Bochs vgacore.cc read: a 16-bit access is two byte reads combined
        // (low | high<<8) — e.g. inw(0x3D4) returns index | data<<8. The VBE
        // dispi ports return their full 16-bit value and must not be split.
        if io_len == 2 && port != VBE_DISPI_IOPORT_INDEX && port != VBE_DISPI_IOPORT_DATA {
            let lo = self.read_port(port, 1, clock);
            let hi = self.read_port(port.wrapping_add(1), 1, clock);
            return lo | (hi << 8);
        }
        match port {
            VGA_CRTC_INDEX | VGA_CRTC_INDEX_MONO => self.crtc_index as u32,
            VGA_CRTC_DATA | VGA_CRTC_DATA_MONO => {
                // Bochs vgacore.cc read: CRTC index 0x22 reads back the graphics
                // controller's data latch for the currently selected read-map plane,
                // instead of a CRTC register.
                if self.crtc_index == 0x22 {
                    self.latch[self.graphics_regs[GFX_REG_READ_MAP_SELECT] as usize & 3] as u32
                } else if self.crtc_index < 25 {
                    self.crtc_regs[self.crtc_index as usize] as u32
                } else {
                    0
                }
            }
            VGA_STATUS | VGA_STATUS_MONO => {
                // Input Status Register 1 (0x3DA / 0x3BA)
                // Matching Bochs vgacore.cc
                // bit 0: Display Enable (1 = in blanking period)
                // bit 3: Vertical Retrace (1 = in vertical retrace)
                let retval = if self.has_icount_sync && self.vtotal_usec > 0 {
                    // Timing-based retrace matching Bochs vgacore.cc:
                    //   display_usec = time_usec() - s.display_start_usec;
                    //   display_usec %= s.vtotal_usec;
                    // The anchor is re-set at each vertical retrace by
                    // vertical_timer(), phase-locking the waveform to the frame.
                    let time_usec = self.current_usec(clock);
                    let display_usec = time_usec.wrapping_sub(self.display_start_usec)
                        % self.vtotal_usec as u64;
                    let mut r = 0u8;
                    // Vertical retrace (bit 3)
                    if display_usec >= self.vrstart_usec as u64
                        && display_usec <= self.vrend_usec as u64
                    {
                        r |= 0x08;
                    }
                    // Display enable / blanking (bit 0)
                    if display_usec >= self.vblank_usec as u64 {
                        r |= 0x01;
                    } else if self.htotal_usec > 0 {
                        let line_usec = display_usec % self.htotal_usec as u64;
                        if line_usec >= self.hbstart_usec as u64
                            && line_usec <= self.hbend_usec as u64
                        {
                            r |= 0x01;
                        }
                    }
                    r
                } else {
                    // Fallback: toggle bits when no timing source available
                    self.status_reg ^= VGA_STATUS_TOGGLE_MASK;
                    self.status_reg
                };
                // Reading this port resets the attribute flip-flop (Bochs line 529)
                self.attr_flip_flop = false;
                retval as u32
            }
            VGA_ATTRIB_ADDR => {
                // Bochs vgacore.cc: read returns (video_enabled<<5)|address
                // Only valid when flip_flop==0 (address mode)
                // Does NOT toggle flip-flop on read
                if !self.attr_flip_flop {
                    let ve = if self.video_enabled { 0x20u8 } else { 0 };
                    (ve | self.attr_index) as u32
                } else {
                    0
                }
            }
            VGA_ATTRIB_DATA => {
                // Bochs vgacore.cc: read attribute data register
                if self.attr_index < 21 {
                    self.attr_regs[self.attr_index as usize] as u32
                } else {
                    0
                }
            }
            VGA_SEQ_INDEX => self.seq_index as u32,
            VGA_SEQ_DATA => {
                if self.seq_index < 5 {
                    self.seq_regs[self.seq_index as usize] as u32
                } else {
                    0
                }
            }
            VGA_GRAPHICS_INDEX => self.graphics_index as u32,
            VGA_GRAPHICS_DATA => {
                if self.graphics_index < 9 {
                    self.graphics_regs[self.graphics_index as usize] as u32
                } else {
                    0
                }
            }
            VGA_MISC_OUTPUT => self.misc_output as u32,

            // 0x3C2 is Input Status 0 on read (the Misc Output *write* port).
            // Bochs vgacore.cc read: RETURN(0).
            VGA_MISC_OUTPUT_WRITE => 0x00,

            // VGA Enable
            VGA_ENABLE => self.vga_enabled as u32,

            // PEL Mask
            VGA_PEL_MASK => self.pel_mask as u32,

            // DAC State
            VGA_DAC_STATE => self.dac_state as u32,

            // PEL Address Write
            VGA_PEL_ADDR_WRITE => self.pel_write_addr as u32,

            // PEL Data - read palette data
            VGA_PEL_DATA => {
                if self.dac_state == DAC_STATE_READ_MODE {
                    let color = self.pel_data[self.pel_read_addr as usize];
                    let val = color[self.pel_read_cycle as usize];
                    self.pel_read_cycle += 1;
                    if self.pel_read_cycle >= PEL_CYCLES_PER_COLOR {
                        self.pel_read_cycle = 0;
                        self.pel_read_addr = self.pel_read_addr.wrapping_add(1);
                    }
                    val as u32
                } else {
                    0x3F // Return 0x3F if not in read mode
                }
            }

            // Feature Control read-back. Bochs vgacore.cc read case 0x03ca:
            // RETURN(s.feature_control).
            0x3CA => self.feature_control as u32,

            // EGA compatibility ports - return 0
            0x3CB | 0x3CD => 0x00,

            // Bochs vgacore.cc read case 0x03db: RETURN(0) — the high byte of a
            // 16-bit read from 0x03DA lands here and must read 0, not 0xFF.
            0x3DB => 0x00,

            _ => 0xFF,
        }
    }

    /// Write to I/O port
    pub(crate) fn write_port(&mut self, port: u16, value: u32, io_len: u8) {
        // Bochs vgacore.cc: port gating based on color_emulation
        if (0x3B0..=0x3BF).contains(&port) && self.misc_color_emulation {
            return; // mono ports disabled in color mode
        }
        if (0x3D0..=0x3DF).contains(&port) && !self.misc_color_emulation {
            return; // color ports disabled in mono mode
        }
        // Word writes: split into two byte writes (Bochs vgacore.cc)
        if io_len == 2 {
            self.write_port(port, value & 0xFF, 1);
            self.write_port(port + 1, (value >> 8) & 0xFF, 1);
            return;
        }
        let value = value as u8;
        match port {
            VGA_CRTC_INDEX | VGA_CRTC_INDEX_MONO => {
                self.crtc_index = value & CRTC_INDEX_MASK;
            }
            VGA_CRTC_DATA | VGA_CRTC_DATA_MONO if self.crtc_index < 25 => {
                let index = self.crtc_index as usize;

                // Bochs vgacore.cc write: CR11 bit 7 (write_protect) locks CRTC
                // registers 0x00-0x06 against writes entirely; a write to 0x07
                // while protected updates only bit 4 (line-compare bit 8),
                // leaving the rest of the register untouched. CR11 itself
                // (index 0x11) is not protected, so it can always be cleared.
                if (self.crtc_regs[CRTC_VERT_RETRACE_END] & 0x80) != 0 && index < 0x08 {
                    if index == CRTC_OVERFLOW {
                        self.crtc_regs[CRTC_OVERFLOW] =
                            (self.crtc_regs[CRTC_OVERFLOW] & !0x10) | (value & 0x10);
                        self.vga_mem_updated = 1;
                        #[cfg(feature = "alloc")]
                        self.redraw_current_legacy_area();
                    }
                    return;
                }

                let old_value = self.crtc_regs[index];
                if old_value != value {
                    self.crtc_regs[index] = value;

                    // Update cursor position if cursor location registers changed
                    if index == CRTC_CURSOR_LOC_HIGH {
                        let cursor_addr =
                            ((value as u16) << 8) | (self.crtc_regs[CRTC_CURSOR_LOC_LOW] as u16);
                        self.cursor_pos = (
                            (cursor_addr as usize / BYTES_PER_ROW),
                            (cursor_addr as usize % BYTES_PER_ROW) / BYTES_PER_CHAR,
                        );
                        self.vga_mem_updated |= 1;
                    } else if index == CRTC_CURSOR_LOC_LOW {
                        let cursor_addr =
                            ((self.crtc_regs[CRTC_CURSOR_LOC_HIGH] as u16) << 8) | (value as u16);
                        self.cursor_pos = (
                            (cursor_addr as usize / BYTES_PER_ROW),
                            (cursor_addr as usize % BYTES_PER_ROW) / BYTES_PER_CHAR,
                        );
                        self.vga_mem_updated |= 1;
                    }
                    // CRTC 0x0C/0x0D deliberately have no immediate effect:
                    // Bochs vgacore.cc notes "Start address change handled in
                    // vertical_timer()", which latches it once per frame.

                    // Recalculate retrace timing and force redraws for register-only
                    // display shape changes. Bochs vgacore.cc write_handler marks
                    // needs_update for these CRTC writes and redraws the visible area.
                    match index {
                        // Bochs vgacore.cc recalcs on CR0 (htotal) and CR2 (hbstart)
                        // too — get_crtc_params/calculate_retrace_timing read them.
                        CRTC_HORIZ_TOTAL
                        | CRTC_START_HORIZ_BLANK
                        | CRTC_END_HORIZ_BLANK
                        | CRTC_END_HORIZ_RETRACE
                        | CRTC_VERT_TOTAL
                        | CRTC_OVERFLOW
                        | CRTC_VERT_RETRACE_START
                        | CRTC_VERT_RETRACE_END
                        | CRTC_VERT_DISPLAY_END => {
                            self.calculate_retrace_timing();
                        }
                        _ => {}
                    }

                    // Bochs vgacore.cc CRTC write case 0x09:
                    //   s.y_doublescan = ((value & 0x9f) > 0);
                    // (bit 7 = line-compare bit 9 and bit 5 = start-address bit
                    // are excluded; any of the max-scan-line bits or bit 7's
                    // 0x80 companion doubles the rows).
                    if index == CRTC_MAX_SCAN_LINE {
                        self.y_doublescan = (value & 0x9F) > 0;
                    }

                    match index {
                        CRTC_OVERFLOW
                        | CRTC_PRESET_ROW_SCAN
                        | CRTC_MAX_SCAN_LINE
                        | CRTC_OFFSET
                        | CRTC_UNDERLINE_LOC
                        | CRTC_MODE_CONTROL
                        | CRTC_LINE_COMPARE => {
                            self.vga_mem_updated = 1;
                            #[cfg(feature = "alloc")]
                            self.redraw_current_legacy_area();
                        }
                        _ => {}
                    }
                }
            }
            VGA_ATTRIB_ADDR => {
                // Writing to 0x3C0 toggles flip-flop
                // Bochs vgacore.cc
                if !self.attr_flip_flop {
                    // Address mode (flip_flop=false): Bochs flip_flop==0
                    // Bit 5 = video_enabled (PAS = Palette Address Source)
                    // Bits 0-4 = attribute index
                    let prev_video_enabled = self.video_enabled;
                    self.video_enabled = (value & 0x20) != 0;

                    if self.video_enabled && !prev_video_enabled {
                        self.text_buffer_update = true;
                    }

                    self.attr_index = value & ATTR_INDEX_MASK; // bits 0-4 only

                // If index is in palette range, write happens on NEXT flip (data mode)
                } else {
                    // Data mode (flip_flop=true): Bochs flip_flop==1
                    // Write to the attribute register selected by attr_index
                    // Bochs vgacore.cc write case 0x03c0 data-write mode: each
                    // register keeps only its defined bits, and a change to the
                    // palette / plane-enable / pel-panning / color-select
                    // registers sets needs_update, which ends in a full
                    // vga_redraw_area(0, 0, last_xres, last_yres).
                    let index = self.attr_index as usize;
                    if index < 21 {
                        let old_value = self.attr_regs[index];
                        let (stored, redraw) = match index {
                            // Internal palette registers 0x00-0x0F.
                            0x00..=0x0F => (value, value != old_value),
                            // 0x10 mode control: bit 7 (internal palette size)
                            // change forces a redraw; bit 2 (line graphics) marks
                            // the charmap dirty, which rusty folds into the same
                            // redraw since it has no separate charmap channel.
                            0x10 => (value, (value ^ old_value) & 0x84 != 0),
                            // 0x11 overscan color: 6 bits, no redraw in Bochs.
                            0x11 => (value & 0x3F, false),
                            // 0x12 color plane enable, 0x13 horizontal pel
                            // panning, 0x14 color select: 4 bits, always redraw.
                            0x12 | 0x13 | 0x14 => (value & 0x0F, true),
                            _ => (value, false),
                        };
                        self.attr_regs[index] = stored;
                        if redraw {
                            self.vga_mem_updated = 1;
                            self.text_buffer_update = true;
                            #[cfg(feature = "alloc")]
                            self.redraw_current_legacy_area();
                        }
                    }
                }
                self.attr_flip_flop = !self.attr_flip_flop;
            }
            // Bochs vgacore.cc write: 0x3C1 (Attribute Data READ port) is not a
            // write target — writes fall through to the ignore path. Attribute
            // registers are written only via the 0x3C0 flip-flop data phase above.
            VGA_ATTRIB_DATA => {}
            VGA_SEQ_INDEX => {
                // Bochs vgacore.cc write: sequencer index is stored unmasked
                // (`s.sequencer.index = value;`). Out-of-range DATA writes are
                // no-ops below, not aliased into the register array.
                self.seq_index = value;
            }
            VGA_SEQ_DATA
                if self.seq_index < 5 => {
                    // Bochs vgacore.cc write case 0x03c5 keeps each sequencer
                    // register as decomposed fields, so a read-back only exposes
                    // the bits it retained. Reproduce that by masking on store.
                    let old_value = self.seq_regs[self.seq_index as usize];
                    match self.seq_index {
                        0 => {
                            // Reset register. Bochs: on the reset1 falling edge
                            // (bit 0 going 1 -> 0) the character-map selection is
                            // reset and the charmap is marked dirty.
                            if (old_value & 0x01) != 0 && (value & 0x01) == 0 {
                                self.seq_regs[SEQ_REG_CHAR_MAP_SELECT] = 0;
                                self.charmap_address1 = 0;
                                self.charmap_address2 = 0;
                                self.vga_mem_updated |= VGA_MEM_UPDATED_CHARMAP;
                            }
                            // Read-back is reset1 | reset2<<1.
                            self.seq_regs[0] = value & 0x03;
                        }
                        1 => {
                            // Clocking mode. Bochs recalculates the retrace timing
                            // and forces a redraw only when one of the bits in
                            // 0x29 changes (dot-clock/2, screen-off, 8/9 dot).
                            if (value ^ old_value) & 0x29 != 0 {
                                self.seq_regs[1] = value & 0x3D;
                                // Bochs: s.sequencer.clear_screen = ((value & 0x20) > 0)
                                self.seq_clear_screen = (value & 0x20) != 0;
                                self.calculate_retrace_timing();
                                self.vga_mem_updated = 1;
                                #[cfg(feature = "alloc")]
                                self.redraw_current_legacy_area();
                            } else {
                                self.seq_regs[1] = value & 0x3D;
                            }
                        }
                        // Map mask: only the 4 plane-enable bits are kept.
                        2 => self.seq_regs[2] = value & 0x0F,
                        3 => {
                            // Character map select. Bochs derives two 3-bit map
                            // indices from the interleaved bit layout and looks
                            // their plane-2 offsets up in charmap_offset[].
                            self.seq_regs[3] = value & 0x3F;
                            let mut charmap1 = value & 0x13;
                            if charmap1 > 3 {
                                charmap1 = (charmap1 & 3) + 4;
                            }
                            let mut charmap2 = (value & 0x2C) >> 2;
                            if charmap2 > 3 {
                                charmap2 = (charmap2 & 3) + 4;
                            }
                            // Bochs only applies the selection when the CRTC
                            // maximum-scan-line register is non-zero (i.e. a text
                            // mode with a real character height).
                            if self.crtc_regs[CRTC_MAX_SCAN_LINE] > 0 {
                                self.charmap_address1 = CHARMAP_OFFSET[charmap1 as usize];
                                self.charmap_address2 = CHARMAP_OFFSET[charmap2 as usize];
                                self.vga_mem_updated |= VGA_MEM_UPDATED_CHARMAP;
                            }
                        }
                        4 => {
                            // Memory mode. Bochs keeps only extended_mem (bit 1),
                            // odd_even_dis (bit 2) and chain_four (bit 3), and its
                            // read-back recomposes exactly those.
                            self.seq_regs[4] = value & 0x0E;
                            self.seq_chain_four = (value & 0x08) != 0;
                            self.seq_odd_even_dis = (value & 0x04) != 0;
                        }
                        _ => self.seq_regs[self.seq_index as usize] = value,
                    }
                }
            VGA_GRAPHICS_INDEX => {
                // Bochs vgacore.cc write: graphics controller index is stored
                // unmasked (`s.graphics_ctrl.index = value;`). Out-of-range
                // DATA writes are no-ops below, not aliased into the register array.
                self.graphics_index = value;
            }
            VGA_GRAPHICS_DATA
                if self.graphics_index < 9 => {
                    let old_value = self.graphics_regs[self.graphics_index as usize];
                    self.graphics_regs[self.graphics_index as usize] = value;

                    // Special handling for Miscellaneous Graphics register.
                    // Bochs vgacore.cc write_handler marks needs_update when
                    // graphics/text alpha or memory mapping changes; alpha also
                    // invalidates the text snapshot and last_yres.
                    if self.graphics_index as usize == GFX_REG_MISC {
                        let old_mapping =
                            (old_value >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK;
                        let new_mapping =
                            (value >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK;
                        let old_graphics_alpha = (old_value & GFX_MISC_GRAPHICS_ALPHA) != 0;
                        let new_graphics_alpha = (value & GFX_MISC_GRAPHICS_ALPHA) != 0;
                        if old_mapping != new_mapping || old_graphics_alpha != new_graphics_alpha {
                            tracing::debug!(
                                "VGA misc changed: mapping {:?}->{:?}, graphics_alpha {}->{}",
                                VgaMemoryMapping::from_u8(old_mapping),
                                VgaMemoryMapping::from_u8(new_mapping),
                                old_graphics_alpha,
                                new_graphics_alpha
                            );
                            self.vga_mem_updated = 1;
                            #[cfg(feature = "alloc")]
                            self.redraw_current_legacy_area();
                            if old_graphics_alpha != new_graphics_alpha {
                                self.text_buffer_update = true;
                                self.last_yres = 0;
                            }
                        }
                    }
                }

            // Misc Output Read port (0x3CC). Bochs vgacore.cc write: `case 0x03cc:
            // /* Graphics 1 Position (EGA) */ // ignore, EGA only???` — the real
            // Misc Output write port is 0x3C2 (VGA_MISC_OUTPUT_WRITE below).
            VGA_MISC_OUTPUT => {}

            // Misc Output Write port - CRITICAL for BIOS color mode setup
            VGA_MISC_OUTPUT_WRITE => {
                self.misc_color_emulation = (value & MISC_OUT_COLOR_EMULATION) != 0;
                self.misc_enable_ram = (value & MISC_OUT_ENABLE_RAM) != 0;
                self.misc_clock_select =
                    (value >> MISC_OUT_CLOCK_SEL_SHIFT) & MISC_OUT_CLOCK_SEL_MASK;
                self.misc_select_high_bank = (value & MISC_OUT_HIGH_BANK) != 0;
                self.misc_horiz_sync_pol = (value & MISC_OUT_HORIZ_POL) != 0;
                self.misc_vert_sync_pol = (value & MISC_OUT_VERT_POL) != 0;
                // Update combined misc_output for reads at 0x3CC
                self.misc_output = value;
                // Bochs vgacore.cc
                self.calculate_retrace_timing();
                tracing::debug!(
                    "VGA Misc Output Write: {:#04x} (color_emulation={}, enable_ram={})",
                    value,
                    self.misc_color_emulation,
                    self.misc_enable_ram
                );
            }

            // VGA Enable
            VGA_ENABLE => {
                self.vga_enabled = (value & 0x01) != 0;
                tracing::trace!("VGA Enable: {}", self.vga_enabled);
            }

            // PEL Mask
            VGA_PEL_MASK => {
                if self.pel_mask != value {
                    self.pel_mask = value;
                    #[cfg(feature = "alloc")]
                    self.redraw_area(0, 0, self.last_xres, self.last_yres);
                }
            }

            // PEL Address Read Mode
            VGA_DAC_STATE => {
                self.pel_read_addr = value;
                self.pel_read_cycle = 0;
                self.dac_state = DAC_STATE_READ_MODE;
            }

            // PEL Address Write Mode
            VGA_PEL_ADDR_WRITE => {
                self.pel_write_addr = value;
                self.pel_write_cycle = 0;
                self.dac_state = DAC_STATE_WRITE_MODE;
            }

            // PEL Data - write palette data
            VGA_PEL_DATA => {
                let color_index = self.pel_write_addr;
                self.pel_data[color_index as usize][self.pel_write_cycle as usize] = value;
                self.pel_write_cycle += 1;
                if self.pel_write_cycle >= PEL_CYCLES_PER_COLOR {
                    self.pel_write_cycle = 0;
                    // Bochs vgacore.cc publishes the completed DAC entry to the
                    // GUI here: palette_change_common(write_data_register,
                    // red << dac_shift, green << dac_shift, blue << dac_shift).
                    self.dac_dirty[color_index as usize] = true;
                    self.dac_any_dirty = true;
                    self.pel_write_addr = self.pel_write_addr.wrapping_add(1);
                    #[cfg(feature = "alloc")]
                    self.redraw_area(0, 0, self.last_xres, self.last_yres);
                }
            }

            // Feature Control (mono/color emulation). Bochs vgacore.cc write
            // cases 0x03ba/0x03da: `s.feature_control = value & 0x08` — the
            // register is otherwise inert ("ignoring: feature ctrl & vert sync").
            VGA_STATUS | VGA_STATUS_MONO => {
                self.feature_control = value & 0x08;
            }

            // EGA compatibility ports - ignore writes
            0x3CA | 0x3CB | 0x3CD => {
                // Ignore (EGA compatibility)
            }

            _ => {
            }
        }
    }

    #[cfg(feature = "alloc")]
    /// Read from text mode memory
    pub(crate) fn read_memory(&self, addr: BxPhyAddress, len: usize) -> Vec<u8> {
        // Debug helper: expose the backing text memory (no window gating).
        // The actual emulated mapping behavior is enforced by mem_{read,write}_handler.
        let offset = (addr as usize) & (VGA_TEXT_MEM_SIZE - 1);
        let end = (offset + len).min(self.text_memory.len());
        if offset < self.text_memory.len() && end > offset {
            let mut out = vec![0u8; len];
            out[..(end - offset)].copy_from_slice(&self.text_memory[offset..end]);
            out
        } else {
            vec![0; len]
        }
    }

    /// Write to text mode memory
    pub(crate) fn write_memory(&mut self, addr: BxPhyAddress, data: &[u8]) {
        // Debug helper: write into backing text memory (no window gating).
        let offset = (addr as usize) & (VGA_TEXT_MEM_SIZE - 1);
        let end = (offset + data.len()).min(self.text_memory.len());
        if offset < self.text_memory.len() && end > offset {
            self.text_memory[offset..end].copy_from_slice(&data[..(end - offset)]);
        }
    }

    /// Whether the adapter is presenting a character grid rather than pixels.
    ///
    /// Bochs vgacore.cc `update()` decides on `graphics_alpha` alone. The
    /// memory map selects the window the CPU reaches video memory through, not
    /// what the CRTC scans out — which is why Linux's vgacon can move the map to
    /// A0000 to load a font into plane 2 while the screen stays a text screen.
    pub(crate) fn in_text_mode(&self) -> bool {
        (self.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) == 0
    }

    /// The character grid the current CRTC/sequencer programming describes, or
    /// `None` when the adapter is in a graphics mode or the grid would not fit
    /// the text aperture.
    ///
    /// Bochs vgacore.cc `update()`, text branch.
    pub(crate) fn text_geometry(&self) -> Option<VgaTextGeometry> {
        if !self.in_text_mode() {
            return None;
        }

        // The CRTC offset register counts dwords; the text aperture stores
        // char/attribute pairs, so a row spans four bytes per unit.
        let mut line_offset = u16::from(self.crtc_regs[CRTC_OFFSET]) * 4;
        if line_offset == 0 {
            line_offset = (TEXT_COLS * BYTES_PER_CHAR) as u16;
        }

        let mut cols = usize::from(self.crtc_regs[CRTC_HORIZ_DISPLAY_END]) + 1;
        let mut msl = usize::from(self.crtc_regs[CRTC_MAX_SCAN_LINE] & CRTC_MSL_MASK);
        let vde = usize::from(self.crtc_regs[CRTC_VERT_DISPLAY_END])
            + ((usize::from(self.crtc_regs[CRTC_OVERFLOW] & CRTC_OVERFLOW_VDE_BIT8)) << 7)
            + ((usize::from(self.crtc_regs[CRTC_OVERFLOW] & CRTC_OVERFLOW_VDE_BIT9)) << 3);

        // Bochs workaround for update() calls before the VGABIOS has programmed
        // the CRTC: both values are replaced together, so a register pair that
        // is half-programmed still describes the standard grid.
        if cols == 1 || msl == 0 {
            cols = TEXT_COLS;
            msl = 15;
        }
        // The emulated CGA 160x100x16 mode drives a two-scanline cell through
        // a 400-line display; Bochs widens the cell to four so the row count
        // comes out at 100.
        if msl == 1 && vde == 399 {
            msl = 3;
        }

        let rows = (vde + 1) / (msl + 1);
        // Bochs reports "text mode: out of memory" and draws no frame when the
        // grid would run past the 128 KiB text aperture.
        if rows.saturating_mul(usize::from(line_offset)) > (1 << 17) {
            return None;
        }

        let start_address = self.crtc_start_addr << 1;
        let cursor_cell = (u16::from(self.crtc_regs[CRTC_CURSOR_LOC_HIGH]) << 8)
            | u16::from(self.crtc_regs[CRTC_CURSOR_LOC_LOW]);
        let cursor_address = cursor_cell * 2;
        let last_address = start_address.wrapping_add(line_offset.wrapping_mul(rows as u16));
        let cursor_address = if cursor_address < start_address || cursor_address > last_address {
            VgaTextGeometry::CURSOR_OFF
        } else {
            cursor_address
        };

        let mut char_width =
            if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_8DOT_CHAR) != 0 {
                8u32
            } else {
                9u32
            };
        if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_DOTCLOCKDIV2) != 0 {
            char_width <<= 1;
        }

        Some(VgaTextGeometry {
            cols,
            rows,
            start_address,
            line_offset,
            cursor_address,
            char_width,
            char_height: (msl + 1) as u32,
            pixel_width: char_width * cols as u32,
            pixel_height: (vde + 1) as u32,
        })
    }

    /// The character at `(row, col)` of the displayed page, rendered for a
    /// screen scrape: printable ASCII as itself, a blank cell as a space, and
    /// anything else as `?`.
    ///
    /// The text aperture is flat — `[char0, attr0, char1, attr1, …]` at
    /// `physical_addr & 0x7FFF` — and the walk wraps inside it exactly as the
    /// renderer's does.
    pub(crate) fn text_char_at(&self, geometry: &VgaTextGeometry, row: usize, col: usize) -> char {
        let offset = (usize::from(geometry.start_address)
            + row * usize::from(geometry.line_offset)
            + col * BYTES_PER_CHAR)
            & (VGA_TEXT_MEM_SIZE - 1);
        match self.text_memory.get(offset).copied().unwrap_or(0) {
            0 => ' ',
            byte if (0x20..0x7F).contains(&byte) => byte as char,
            _ => '?',
        }
    }

    #[cfg(feature = "alloc")]
    /// Get text mode screen contents as a string
    pub(crate) fn get_text_screen(&self) -> String {
        let Some(geometry) = self.text_geometry() else {
            return String::new();
        };
        let mut result = String::new();
        for row in 0..geometry.rows {
            let start = result.len();
            for col in 0..geometry.cols {
                result.push(self.text_char_at(&geometry, row, col));
            }
            // Trim trailing spaces
            let trim_len = result[start..].trim_end_matches(' ').len();
            result.truncate(start + trim_len);
            result.push('\n');
        }
        result
    }

    #[cfg(feature = "alloc")]
    /// Scan all 32KB of VGA text memory and return summary: CRTC start address,
    /// graphics mode flag, and any non-space printable chars found anywhere.
    pub(crate) fn scan_all_text_memory(&self) -> String {
        use core::fmt::Write;
        let mut s = String::new();
        let start_addr_words = ((self.crtc_regs[CRTC_START_ADDR_HIGH] as u16) << 8)
            | (self.crtc_regs[CRTC_START_ADDR_LOW] as u16);
        let graphics_alpha = (self.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) != 0;
        writeln!(
            s,
            "CRTC_start={:#x} graphics_alpha={} text_mem_len={}",
            start_addr_words,
            graphics_alpha,
            self.text_memory.len()
        )
        .ok();
        // Collect up to 256 printable non-space chars from ALL of text_memory
        let mut chars = String::new();
        for chunk in self.text_memory.chunks_exact(2) {
            let ch = chunk[0];
            if (0x20..0x7F).contains(&ch) && ch != b' ' {
                chars.push(ch as char);
                if chars.len() >= 256 {
                    break;
                }
            }
        }
        if chars.is_empty() {
            write!(s, "text_memory: all blank").ok();
        } else {
            write!(s, "text_memory chars: {}", chars).ok();
        }
        s
    }

    #[cfg(feature = "alloc")]
    /// Return all rows from VGA text memory as a Vec of Strings (for diagnostics).
    /// Scans the entire 32KB text_memory buffer row by row (80-col rows).
    pub(crate) fn get_all_text_rows(&self) -> alloc::vec::Vec<alloc::string::String> {
        let total_bytes = self.text_memory.len();
        let total_rows = total_bytes / BYTES_PER_ROW;
        let mut rows = alloc::vec::Vec::with_capacity(total_rows);
        for row in 0..total_rows {
            let row_base = row * BYTES_PER_ROW;
            let mut row_str = alloc::string::String::with_capacity(TEXT_COLS);
            for col in 0..TEXT_COLS {
                let off = row_base + col * BYTES_PER_CHAR;
                let ch = self.text_memory.get(off).copied().unwrap_or(0);
                if (0x20..0x7F).contains(&ch) {
                    row_str.push(ch as char);
                } else {
                    row_str.push(' ');
                }
            }
            rows.push(row_str);
        }
        rows
    }

    /// Get text mode memory buffer (for GUI updates)
    /// Get cursor position (row, col) for text mode
    pub(crate) fn get_cursor_position(&self) -> (u32, u32) {
        (self.cursor_pos.0 as u32, self.cursor_pos.1 as u32)
    }

    pub(crate) fn get_text_memory(&self) -> &[u8] {
        &self.text_memory
    }

    /// Check if text memory has changed (dirty)
    pub(crate) fn is_text_dirty(&self) -> bool {
        self.text_dirty
    }

    /// Clear the text dirty flag (call after updating GUI)
    pub(crate) fn clear_text_dirty(&mut self) {
        self.text_dirty = false;
    }

    /// Force text dirty flag (for initial display)
    pub(crate) fn force_text_dirty(&mut self) {
        self.text_dirty = true;
    }

    /// Force initial update (for first GUI render)
    pub(crate) fn force_initial_update(&mut self) {
        self.vga_mem_updated = 1;
        self.text_buffer_update = true;
    }

    #[cfg(feature = "alloc")]
    fn mark_tile_updated(&mut self, x_tile: u32, y_tile: u32) {
        if x_tile >= self.num_x_tiles as u32 || y_tile >= self.num_y_tiles as u32 {
            return;
        }
        let index = y_tile as usize * self.num_x_tiles as usize + x_tile as usize;
        if let Some(tile) = self.vga_tile_updated.get_mut(index) {
            *tile = true;
            self.vga_mem_updated = 1;
        }
    }

    #[cfg(feature = "alloc")]
    fn redraw_area(&mut self, x0: u32, y0: u32, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        let x1 = x0.saturating_add(width.saturating_sub(1));
        let y1 = y0.saturating_add(height.saturating_sub(1));
        let start_x = x0 / VGA_X_TILESIZE;
        let start_y = y0 / VGA_Y_TILESIZE;
        let end_x = (x1 / VGA_X_TILESIZE).min(self.num_x_tiles.saturating_sub(1) as u32);
        let end_y = (y1 / VGA_Y_TILESIZE).min(self.num_y_tiles.saturating_sub(1) as u32);

        for y_tile in start_y..=end_y {
            for x_tile in start_x..=end_x {
                self.mark_tile_updated(x_tile, y_tile);
            }
        }
    }
    #[cfg(feature = "alloc")]
    fn redraw_current_legacy_area(&mut self) {
        let (width, height) = self.determine_screen_dimensions();
        self.redraw_area(0, 0, width, height);
    }


    fn determine_screen_dimensions(&self) -> (u32, u32) {
        let width = (self.crtc_regs[0x01] as u32 + 1) * 8;
        let vde = self.crtc_regs[0x12] as u32
            | (((self.crtc_regs[0x07] & 0x02) as u32) << 7)
            | (((self.crtc_regs[0x07] & 0x40) as u32) << 3);
        let vblank = self.crtc_regs[0x15] as u32
            | (((self.crtc_regs[0x07] & 0x08) as u32) << 5)
            | (((self.crtc_regs[0x09] & 0x20) as u32) << 4);
        let mut height = (vde + 1).min(vblank + 1);
        let mut width = width;
        if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_DOTCLOCKDIV2) != 0 {
            width <<= 1;
        }
        if self.ext_y_dblsize {
            height <<= 1;
        }
        (width, height)
    }

    fn legacy_line_offset(&self) -> u32 {
        let mut line_offset = (self.crtc_regs[0x13] as u32) << 1;
        if (self.crtc_regs[0x14] & 0x40) != 0 {
            line_offset <<= 2;
        } else if (self.crtc_regs[0x17] & 0x40) == 0 {
            line_offset <<= 1;
        }
        line_offset
    }

    fn dac_index_to_rgba(&self, index: u8) -> [u8; 4] {
        let color = self.pel_data[(index & self.pel_mask) as usize];
        let shift = self.dac_shift;
        [
            color[0] << shift,
            color[1] << shift,
            color[2] << shift,
            0xff,
        ]
    }

    fn get_vga_pixel(
        &self,
        x: u16,
        y: u16,
        row_addr: u32,
        line_compare: u16,
        blink_state: bool,
    ) -> u8 {
        let mut x = x as u32;
        if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_DOTCLOCKDIV2) != 0 {
            x >>= 1;
        }
        let pixel_panning_compat =
            (self.attr_regs[ATTR_REG_MODE_CONTROL] & ATTR_MODE_SPLIT_HPANNING) != 0;
        if (y <= line_compare) || !pixel_panning_compat {
            x += (self.attr_regs[ATTR_REG_HORIZ_PIXEL_PAN] & ATTR_HPANNING_MASK) as u32;
        }
        let bit_no = 7 - (x % 8);
        let byte_offset = (((row_addr + (x / 8)) << 2) & self.vga_mem_mask) as usize;
        let attribute = (((vga_storage_get(self, byte_offset) >> bit_no) & 0x01) << 0)
            | (((vga_storage_get(self, byte_offset + 1) >> bit_no) & 0x01) << 1)
            | (((vga_storage_get(self, byte_offset + 2) >> bit_no) & 0x01) << 2)
            | (((vga_storage_get(self, byte_offset + 3) >> bit_no) & 0x01) << 3);
        let mut attribute = attribute & self.attr_regs[ATTR_REG_COLOR_PLANE_EN];
        if (self.attr_regs[ATTR_REG_MODE_CONTROL] & 0x08) != 0 {
            if blink_state {
                attribute |= 0x08;
            } else {
                attribute ^= 0x08;
            }
        }
        let palette_reg_val = self.attr_regs[(attribute & 0x0f) as usize];
        let color_select = self.attr_regs[ATTR_REG_COLOR_SELECT];
        let dac_reg = if (self.attr_regs[ATTR_REG_MODE_CONTROL] & 0x80) != 0 {
            (palette_reg_val & 0x0f) | (color_select << 4)
        } else {
            (palette_reg_val & 0x3f) | ((color_select & 0x0c) << 4)
        };
        dac_reg & self.pel_mask
    }

    #[cfg(feature = "alloc")]
    fn mark_legacy_dirty_offset(&mut self, offset: u32) {
        let graphics_alpha = (self.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) != 0;
        if !graphics_alpha {
            return;
        }
        // Bochs uses the per-frame latch here too (s.CRTC.start_addr).
        let start_addr = (self.crtc_start_addr as u32).wrapping_add(self.ext_start_addr);
        let shift = (self.graphics_regs[GFX_REG_GRAPHICS_MODE] >> 5) & 0x03;
        let mut line_offset = self.legacy_line_offset();
        if shift >= 2 && (self.crtc_regs[0x17] & 0x40) != 0 {
            line_offset <<= 2;
        }
        if line_offset == 0 {
            return;
        }
        let rel = offset.wrapping_sub(start_addr);
        let pixels_per_byte = if shift == 0 { 8 } else { 1 };
        let x = (rel % line_offset) * pixels_per_byte;
        let y = rel / line_offset;
        self.mark_tile_updated(x / VGA_X_TILESIZE, y / VGA_Y_TILESIZE);
    }

    #[cfg(feature = "alloc")]
    fn refresh_legacy_graphics<S: DisplaySink>(&mut self, sink: &mut S) -> Refreshed {
        if self.vga_mem_updated == 0 {
            return Refreshed::Unchanged;
        }

        let (width, height) = self.determine_screen_dimensions();
        if width == 0 || height == 0 {
            return Refreshed::Unchanged;
        }
        // Bochs vgacore.cc update(): the graphics branch also bails out through
        // skip_update() once the dimensions are known.
        if self.skip_update() {
            return Refreshed::Unchanged;
        }
        let dimension_changed =
            width != self.last_xres || height != self.last_yres || self.last_bpp > 8;
        if dimension_changed {
            self.last_xres = width;
            self.last_yres = height;
            self.last_fw = 0;
            self.last_fh = 0;
            self.last_bpp = 8;
            self.redraw_area(0, 0, width, height);
            sink.dimension_update(Dimensions {
                width,
                height,
                font_width: 0,
                font_height: 0,
                bits_per_pixel: 8,
            });
        }

        // Bochs uses the per-frame latch here too (s.CRTC.start_addr).
        let start_addr = (self.crtc_start_addr as u32).wrapping_add(self.ext_start_addr);
        let line_offset = self.legacy_line_offset().max(1);
        let line_compare = {
            let lc = self.crtc_regs[CRTC_LINE_COMPARE] as u16
                | if self.crtc_regs[CRTC_OVERFLOW] & 0x10 != 0 {
                    0x100
                } else {
                    0
                }
                | if self.crtc_regs[CRTC_MAX_SCAN_LINE] & 0x40 != 0 {
                    0x200
                } else {
                    0
                };
            // Bochs vgacore.cc update(): `if (s.y_doublescan) line_compare >>= 1;`
            // — the split-screen line compare is in doubled rows.
            if self.y_doublescan {
                lc >> 1
            } else {
                lc
            }
        };
        let shift = (self.graphics_regs[GFX_REG_GRAPHICS_MODE] >> 5) & 0x03;
        // One tile's worth of pixels, reused across the frame. The tile size is
        // a constant, so this is a fixed buffer rather than the vector of
        // vectors a frame used to allocate.
        let mut tile_rgba = [0u8; TILE_RGBA_BYTES];

        for yc in (0..height).step_by(VGA_Y_TILESIZE as usize) {
            let y_tile = yc / VGA_Y_TILESIZE;
            for xc in (0..width).step_by(VGA_X_TILESIZE as usize) {
                let x_tile = xc / VGA_X_TILESIZE;
                let tile_index = y_tile as usize * self.num_x_tiles as usize + x_tile as usize;
                if !self
                    .vga_tile_updated
                    .get(tile_index)
                    .copied()
                    .unwrap_or(false)
                {
                    continue;
                }

                let tile_width = VGA_X_TILESIZE.min(width - xc);
                let tile_height = VGA_Y_TILESIZE.min(height - yc);
                let rgba = &mut tile_rgba[..(tile_width * tile_height * 4) as usize];
                for r in 0..tile_height {
                    let mut y = yc + r;
                    // Bochs vgacore.cc update(): `if (s.y_doublescan) y >>= 1;`
                    // — two consecutive screen rows share one memory row.
                    if self.y_doublescan {
                        y >>= 1;
                    }
                    for c in 0..tile_width {
                        let x = xc + c;
                        let dac_index = match shift {
                            0 => {
                                let row_addr = if (self.crtc_regs[0x17] & 1) == 0 {
                                    (start_addr & 0xdfff) + ((y & 1) << 13) + (320 / 4) * (y / 2)
                                } else if y > line_compare as u32 {
                                    (y - line_compare as u32 - 1) * line_offset
                                } else {
                                    start_addr + y * line_offset
                                };
                                self.get_vga_pixel(
                                    x as u16,
                                    y as u16,
                                    row_addr,
                                    line_compare,
                                    false,
                                )
                            }
                            1 => {
                                let mut src_x = x;
                                if (self.seq_regs[SEQ_REG_CLOCKING_MODE]
                                    & SEQ_CLOCKING_DOTCLOCKDIV2)
                                    != 0
                                {
                                    src_x >>= 1;
                                }
                                let mut byte_offset =
                                    (start_addr << 1) + (320 / 4) * (y / 2) + (src_x / 4);
                                byte_offset &= 0x1fff;
                                byte_offset += (y & 1) << 13;
                                let attribute = 6 - 2 * (src_x % 4);
                                let memory_index =
                                    (((byte_offset & !1) << 2) | (byte_offset & 1)) as usize;
                                let palette_reg_val =
                                    (vga_storage_get(self, memory_index) >> attribute) & 0x03;
                                self.attr_regs[palette_reg_val as usize] & self.pel_mask
                            }
                            _ => {
                                let byte_offset = if (self.crtc_regs[0x14] & 0x40) != 0 {
                                    let row_addr = (start_addr << 2) + y * line_offset;
                                    (row_addr + (x >> 1)) & 0xffff
                                } else if (self.crtc_regs[0x17] & 0x40) != 0 {
                                    let h_panning =
                                        (self.attr_regs[ATTR_REG_HORIZ_PIXEL_PAN] >> 1) as u32;
                                    let row_addr = (start_addr << 2) + y * (line_offset << 2);
                                    (row_addr + (x >> 1) + h_panning) & 0x3ffff
                                } else {
                                    let row_addr = start_addr + y * line_offset;
                                    (row_addr + (((x >> 1) & !1) << 1) + ((x >> 1) & 1)) & 0x3ffff
                                };
                                vga_storage_get(self, byte_offset as usize) & self.pel_mask
                            }
                        };
                        let pixel = self.dac_index_to_rgba(dac_index);
                        let dst = ((r * tile_width + c) * 4) as usize;
                        rgba[dst..dst + 4].copy_from_slice(&pixel);
                    }
                }

                if let Some(tile) = self.vga_tile_updated.get_mut(tile_index) {
                    *tile = false;
                }
                sink.graphics_tile_update(
                    rgba,
                    TilePos {
                        x: xc,
                        y: yc,
                        width: tile_width,
                        height: tile_height,
                    },
                );
            }
        }

        self.vga_mem_updated = 0;
        Refreshed::Frame
    }


    /// Vertical retrace: latch the frame's start address and re-anchor the
    /// 0x3DA phase.
    ///
    /// Bochs `bx_vgacore_c::vertical_timer()` (vgacore.cc):
    ///   prev = s.CRTC.start_addr;
    ///   s.CRTC.start_addr = (CRTC.reg[0x0c] << 8) | CRTC.reg[0x0d];
    ///   if changed -> redraw (graphics: vga_redraw_area, text: vga_mem_updated |= 1)
    ///   s.display_start_usec = current time
    ///
    /// Returns whether the start address moved, so the caller can force the
    /// redraw Bochs performs for the graphics path.
    pub(crate) fn vertical_timer(&mut self, now_usec: u64) -> bool {
        let previous = self.crtc_start_addr;
        self.crtc_start_addr = ((self.crtc_regs[CRTC_START_ADDR_HIGH] as u16) << 8)
            | self.crtc_regs[CRTC_START_ADDR_LOW] as u16;
        let changed = self.crtc_start_addr != previous;
        if changed {
            self.vga_mem_updated |= 1;
            self.text_buffer_update = true;
        }
        self.display_start_usec = now_usec;
        changed
    }

    /// Period of the vertical retrace in microseconds, for arming the vertical
    /// timer (Bochs `s.vtotal_usec`). Zero before the retrace timing is known.
    pub(crate) fn vertical_period_usec(&self) -> u32 {
        self.vtotal_usec
    }

    /// Whether this frame's screen update must be skipped.
    ///
    /// Bochs `bx_vgacore_c::skip_update()` (vgacore.cc): services a pending
    /// sequencer clear-screen request, then skips while the VGA or the video
    /// output is disabled, while the attribute controller and graphics
    /// controller disagree about graphics-vs-alpha (a mode set in progress),
    /// while either sequencer reset line is asserted, or while the screen-off
    /// bit (register 1 bit 5) is set.
    ///
    /// Bochs's additional "skip during the vertical retrace window" test is
    /// guarded by `if (!update_mode_vsync)`, and `update_mode_vsync` is true in
    /// its default configuration (`vga_update_freq` = 0), where the update is
    /// driven by the vertical timer instead. rusty_box drives `update()` from
    /// the GUI frame loop, i.e. the same vsync-driven shape, so that branch is
    /// bypassed here exactly as it is upstream.
    fn skip_update(&mut self) -> bool {
        // Bochs: handle clear screen request from the sequencer.
        if self.seq_clear_screen {
            self.pending_clear_screen = true;
            self.seq_clear_screen = false;
        }

        let reset1 = (self.seq_regs[SEQ_REG_RESET] & 0x01) != 0;
        let reset2 = (self.seq_regs[SEQ_REG_RESET] & 0x02) != 0;
        let screen_off = (self.seq_regs[SEQ_REG_CLOCKING_MODE] & 0x20) != 0;
        // attribute_ctrl.mode_ctrl.graphics_alpha is bit 0 of attribute reg 0x10.
        let actl_graphics_alpha = (self.attr_regs[0x10] & 0x01) != 0;
        let gfx_graphics_alpha = (self.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) != 0;

        !self.vga_enabled
            || !self.video_enabled
            || actl_graphics_alpha != gfx_graphics_alpha
            || !reset2
            || !reset1
            || screen_off
    }

    /// Drain the DAC entries whose colour changed, as `(index, r, g, b)` with
    /// each component shifted to host width by `dac_shift`, as Bochs's
    /// `palette_change_common` calls are.
    pub(crate) fn take_dac_palette_changes(&mut self) -> impl Iterator<Item = (u8, u8, u8, u8)> + '_ {
        let any = core::mem::take(&mut self.dac_any_dirty);
        (0..PEL_COLOR_COUNT).filter_map(move |i| {
            if !any || !core::mem::take(&mut self.dac_dirty[i]) {
                return None;
            }
            let entry = self.pel_data[i];
            Some((
                i as u8,
                entry[0] << self.dac_shift,
                entry[1] << self.dac_shift,
                entry[2] << self.dac_shift,
            ))
        })
    }

    /// Take a pending `clear_screen()` owed to the GUI (Bochs calls
    /// `bx_gui->clear_screen()` directly from `skip_update`).
    pub(crate) fn take_pending_clear_screen(&mut self) -> bool {
        core::mem::take(&mut self.pending_clear_screen)
    }

    /// Re-extract both character generators from plane 2 of planar memory.
    ///
    /// Bochs `bx_vgacore_c::update_charmap()` (vgacore.cc): glyph bytes live in
    /// plane 2, so byte `i` of a map is `memory[(address << 2) + i * 4 + 2]`.
    /// When both maps select the same address Bochs publishes the SAME buffer as
    /// map 1 — which is what makes the attribute bit-3 font select harmless in
    /// the usual single-font case.
    fn update_charmap(&mut self) {
        let mut addr = (self.charmap_address1 as usize) << 2;
        for i in 0..CHARMAP_SIZE {
            self.charmap[0][i] = vga_storage_get(self, (addr + 2) & (VGA_MEM_SIZE - 1));
            addr += 4;
        }
        if self.charmap_address2 != self.charmap_address1 {
            let mut addr = ((self.charmap_address2 as usize) << 2) + 2;
            for i in 0..CHARMAP_SIZE {
                self.charmap[1][i] = vga_storage_get(self, addr & (VGA_MEM_SIZE - 1));
                addr += 4;
            }
        } else {
            self.charmap[1] = self.charmap[0];
        }
    }

    /// One of the two extracted character generators (0 or 1), as raw VGA glyph
    /// bitmaps: 32 bytes per glyph, each byte MSB-first (bit 7 = leftmost pixel).
    pub(crate) fn charmap(&self, map: usize) -> &[u8; CHARMAP_SIZE] {
        &self.charmap[map & 1]
    }

    /// Draw one frame into `sink` — Bochs `bx_vgacore_c::update()`.
    ///
    /// Upstream calls the front end from inside this function, and so does
    /// this: the screen-clear request, the completed DAC writes, the character
    /// generators and the pixels all reach the sink here, in Bochs's order,
    /// instead of being stashed for a caller to drain and forward.
    ///
    /// One signature in every build (R0). Without `alloc` there is no VBE
    /// backing store and no tile buffer, so only the text path can produce a
    /// frame — which is what the no-alloc build did before, through a
    /// separately-typed second entry point.
    pub(crate) fn refresh<S: DisplaySink>(&mut self, sink: &mut S) -> Refreshed {
        self.refresh_frame(sink)
    }

    /// The frame preamble every card owes its front end, whichever half draws
    /// the pixels: the pending screen clear and the DAC writes completed since
    /// the last frame.
    ///
    /// Bochs `skip_update()` calls `bx_gui->clear_screen()` even on frames it
    /// declines to draw, and publishes each completed DAC entry through
    /// `palette_change_common()` before the frame that uses it — so both happen
    /// ahead of any decision about how, or whether, to draw.
    pub(crate) fn drain_frame_preamble<S: DisplaySink>(&mut self, sink: &mut S) {
        if core::mem::take(&mut self.pending_clear_screen) {
            sink.clear_screen();
        }
        if core::mem::take(&mut self.dac_any_dirty) {
            for index in 0..PEL_COLOR_COUNT {
                if !core::mem::take(&mut self.dac_dirty[index]) {
                    continue;
                }
                let entry = self.pel_data[index];
                let _redraw = sink.palette_change(
                    index as u8,
                    Rgb {
                        red: entry[0] << self.dac_shift,
                        green: entry[1] << self.dac_shift,
                        blue: entry[2] << self.dac_shift,
                    },
                );
            }
        }
    }

    /// The mode dispatch, split out so [`Self::refresh`] reads as the sequence
    /// Bochs performs rather than as one long function.
    /// No `Dirt` reaches here: every window the core renders from — the legacy
    /// aperture and the text plane — is latched, plane-masked and chain-4
    /// addressed, so it must trap and can never carry page-tracked dirt. Only
    /// a card's linear framebuffer can, and that is the card's to draw.
    fn refresh_frame<S: DisplaySink>(&mut self, sink: &mut S) -> Refreshed {
        if self.in_text_mode() {
            return self.refresh_text_mode(sink);
        }

        #[cfg(feature = "alloc")]
        {
            self.refresh_legacy_graphics(sink)
        }
        // A planar graphics frame needs a tile buffer to convert into, which
        // this build has no allocator for. The mode still runs — the guest's
        // writes land in VRAM and the dirty bitmap tracks them — there is just
        // nothing that can present it.
        #[cfg(not(feature = "alloc"))]
        {
            Refreshed::Unchanged
        }
    }

    /// Draw a text frame — Bochs `bx_vgacore_c::update()`'s alphanumeric path.
    ///
    /// The one mode every build can present: text needs no conversion buffer,
    /// so this is what a no-alloc machine draws.
    fn refresh_text_mode<S: DisplaySink>(&mut self, sink: &mut S) -> Refreshed {
        // Check if we're in text mode (match Bochs `vgacore.cc` semantics).
        //
        // In Bochs, `s.graphics_ctrl.graphics_alpha` and `s.graphics_ctrl.memory_mapping`
        // are derived from the Graphics Controller register index 0x06:
        //   graphics_alpha = value & 0x01
        //   memory_mapping = (value >> 2) & 0x03
        //
        // Text mode when `graphics_alpha == 0`. Memory mapping selects which aperture
        // is active (B0000 vs B8000 for mono/color text).
        if !self.in_text_mode() {
            return Refreshed::Unchanged;
        }

        // Bochs vgacore.cc update(): `if ((s.vga_mem_updated & 4) > 0) update_charmap();`
        // — re-extract the character generators before drawing the frame.
        let charmap_updated = (self.vga_mem_updated & VGA_MEM_UPDATED_CHARMAP) != 0;
        if charmap_updated {
            self.vga_mem_updated &= !VGA_MEM_UPDATED_CHARMAP;
            self.update_charmap();
        }

        // Bochs vgacore.cc update(): `if (skip_update()) return;` — no frame is
        // drawn while the display is disabled or a mode set is in progress.
        if self.skip_update() {
            return Refreshed::Unchanged;
        }

        // The grid comes from the shared reader, so the frame drawn here and
        // any scrape of the same registers describe the same screen. The start
        // address is the per-frame latch, as in Bochs's renderers
        // (`tm_info.start_address = (s.CRTC.start_addr << 1)`).
        let Some(geometry) = self.text_geometry() else {
            return Refreshed::Unchanged;
        };
        let start_address = geometry.start_address;
        let line_offset = geometry.line_offset;
        let cursor_address = geometry.cursor_address;

        let cs_start = self.crtc_regs[CRTC_CURSOR_START] & CRTC_CURSOR_START_MASK;
        let cs_end = self.crtc_regs[CRTC_CURSOR_END] & CRTC_CURSOR_END_MASK;

        let line_compare = {
            let lc_low = self.crtc_regs[CRTC_LINE_COMPARE] as u16;
            let lc_bit8 = if self.crtc_regs[CRTC_OVERFLOW] & 0x10 != 0 {
                0x100u16
            } else {
                0
            };
            let lc_bit9 = if self.crtc_regs[CRTC_MAX_SCAN_LINE] & 0x40 != 0 {
                0x200u16
            } else {
                0
            };
            lc_low | lc_bit8 | lc_bit9
        };
        let h_panning = self.attr_regs[ATTR_REG_HORIZ_PIXEL_PAN] & ATTR_HPANNING_MASK;
        let v_panning = self.crtc_regs[CRTC_PRESET_ROW_SCAN] & CRTC_PRESET_ROW_MASK;
        let line_graphics = (self.attr_regs[ATTR_REG_MODE_CONTROL] & ATTR_MODE_LINE_GRAPHICS) != 0;
        let split_hpanning =
            (self.attr_regs[ATTR_REG_MODE_CONTROL] & ATTR_MODE_SPLIT_HPANNING) != 0;
        let blink_flags = {
            let mut flags = 0u8;
            // Bit 3 of attr mode control register = blink/intensity select
            if self.attr_regs[ATTR_REG_MODE_CONTROL] & 0x08 != 0 {
                flags |= 1; // BX_TEXT_BLINK_MODE
            }
            flags
        };

        // Build palette (matching vgacore.cc)
        let mut actl_palette = [0u8; 16];
        for (i, palette) in actl_palette.iter_mut().enumerate() {
            // Bochs vgacore.cc update(): actl_palette[i] = palette_reg[i] & pel.mask
            *palette = self.attr_regs[i] & self.pel_mask;
        }

        // Copy from VGA memory to text_buffer if needed.
        // We update the visible page whenever memory changed since the last update,
        // or when parameters request a full refresh.
        let need_refresh = self.text_buffer_update || (self.vga_mem_updated > 0);
        let visible_size = 0x8000.min(self.text_buffer.len());

        // Bochs maps the selected window to the same underlying memory backing store.
        let visible_size = visible_size.min(self.text_memory.len());
        if need_refresh {
            self.text_buffer[..visible_size].copy_from_slice(&self.text_memory[..visible_size]);
            self.text_buffer_update = false;
        }

        // Create text mode info
        let tm_info = VgaTextModeInfo {
            start_address,
            cs_start,
            cs_end,
            line_offset,
            line_compare,
            h_panning,
            v_panning,
            line_graphics,
            split_hpanning,
            blink_flags,
            actl_palette,
        };

        // Dimension_update parameters (matching vgacore.cc)
        let c_width = geometry.char_width;
        let i_width = geometry.pixel_width;
        let i_height = geometry.pixel_height;
        let fh = geometry.char_height;

        // Only signal dimension change when something actually changed (vgacore.cc)
        let dimension_changed = i_width != self.last_xres
            || i_height != self.last_yres
            || c_width != self.last_fw
            || fh != self.last_fh
            || self.last_bpp > 8;
        if dimension_changed {
            self.last_xres = i_width;
            self.last_yres = i_height;
            self.last_fw = c_width;
            self.last_fh = fh;
            self.last_bpp = 8;
            sink.dimension_update(Dimensions {
                width: i_width,
                height: i_height,
                font_width: c_width,
                font_height: fh,
                bits_per_pixel: 8,
            });
        }

        // Bochs vgacore.cc update_charmap() pushes both guest character
        // generators before the text is drawn with them.
        if charmap_updated {
            sink.set_text_charmap(0, &self.charmap[0]);
            sink.set_text_charmap(1, &self.charmap[1]);
        }

        // The snapshot is the previous frame's cells and the buffer is this
        // frame's; the front end diffs them. Pushing before the snapshot
        // advances is what removes the two 32 KiB copies this used to return.
        sink.text_update(
            &self.text_snapshot,
            &self.text_buffer,
            cursor_cell(cursor_address, &tm_info),
            &tm_info,
        );

        // Bochs redraws unconditionally in text mode and lets the front end
        // decide from the diff, so the frame above is always sent; only the
        // internal snapshot advance is conditional.
        if self.vga_mem_updated > 0 {
            self.text_snapshot[..visible_size].copy_from_slice(&self.text_buffer[..visible_size]);
            self.vga_mem_updated = 0;
            self.text_dirty = false;
        }

        Refreshed::Frame
    }
}

/// Where the text cursor sits, in character cells, or `None` when it is
/// disabled or parked outside the visible page.
///
/// Bochs signals the absent cursor with an out-of-range address and leaves
/// every front end to recognise it. The magic number stops here instead.
///
/// `line_offset` divides safely: `text_geometry` substitutes the default row
/// stride when the CRTC offset register reads zero, so a mode this is reached
/// in never carries a zero.
fn cursor_cell(cursor_address: u16, info: &VgaTextModeInfo) -> Option<CursorPos> {
    if cursor_address >= 0x7fff {
        return None;
    }
    let offset_from_start = cursor_address.saturating_sub(info.start_address);
    Some(CursorPos {
        col: u32::from((offset_from_start % info.line_offset) / 2),
        row: u32::from(offset_from_start / info.line_offset),
    })
}

/// VGA memory read handler (called from memory system)
/// Based on bx_vgacore_c::mem_read / mem_read_handler in vgacore.cc
/// Implements read mode 0 (return selected plane) and read mode 1 (color compare).
/// Loads latch register on every read.
impl VgaCore {


    /// Read from the legacy `A0000-BFFFF` aperture.
    ///
    /// Bochs `bx_vgacore_c::mem_read`. The physical address is reconstructed
    /// from the window base because the VBE aperture path addresses by it; the
    /// *routing* decision it used to serve is gone.
    pub(crate) fn legacy_read(
        &mut self,
        at: crate::api::WindowOffset,
        len: u32,
        data: &mut [u8],
    ) {
        for (i, byte) in data.iter_mut().enumerate().take(len as usize) {
            *byte = self.legacy_read_byte(at.get() + i as u64);
        }
    }

    /// One byte of the legacy aperture — Bochs `bx_vgacore_c::mem_read`, whose
    /// virtual is per byte because a card claims individual addresses. The
    /// width-aware handlers are the register blocks, not video memory.
    pub(crate) fn legacy_read_byte(&mut self, offset: u64) -> u8 {
        vga_mem_read_byte(self, VGA_WINDOW_GRAPHICS_BASE + offset)
    }

    /// Read from the linear framebuffer behind BAR0.
    ///
    /// Bochs vga.cc `mem_read`'s LFB arm: chain-4 accesses reach
    /// `bx_vgacore_c::mem_read(offset)` with the raw offset, so the full 256 KB
    /// is addressable — wrapping to 128 KB and re-entering at the legacy base
    /// would re-apply window gating.
    ///
    /// The window exists in every build so the dispatch has one shape; only the
    /// backing store is `alloc`-only. Without it `init` never registers the
    /// window, so a machine cannot route here at all — reaching it means one was
    /// mapped that nothing can serve.
    pub(crate) fn lfb_read(
        &mut self,
        at: crate::api::WindowOffset,
        len: u32,
        data: &mut [u8],
    ) {
        #[cfg(not(feature = "alloc"))]
        {
            let _ = (at, len);
            tracing::error!("VGA: framebuffer read with no backing store");
            data.fill(0xff);
        }
        #[cfg(feature = "alloc")]
        for i in 0..len as u64 {
            let Some(byte) = data.get_mut(i as usize) else {
                break;
            };
            let offset = at.get() + i;
            if self.seq_chain_four && offset < 0x40000 {
                *byte = vga_mem_read_byte(self, offset);
            } else {
                *byte = 0xff;
            }
        }
    }

    /// Write to the legacy `A0000-BFFFF` aperture — Bochs
    /// `bx_vgacore_c::mem_write`, all four write modes with full planar
    /// memory support.
    pub(crate) fn legacy_write(
        &mut self,
        at: crate::api::WindowOffset,
        len: u32,
        data: &[u8],
    ) {
        for (i, &value) in data.iter().enumerate().take(len as usize) {
            self.legacy_write_byte(at.get() + i as u64, value);
        }
    }

    /// One byte of a video-memory window.
    ///
    /// The unit an extension is offered an access in, because that is the unit
    /// Bochs offers one in: `bx_vgacore_c::mem_read` takes a single address and
    /// the handler above it loops. Only the framebuffer windows are addressed
    /// this way — a register block is width-aware and is never split.
    pub(crate) fn window_read_byte(&mut self, window: VgaWindow, offset: u64) -> u8 {
        match window {
            VgaWindow::Legacy => self.legacy_read_byte(offset),
            VgaWindow::Lfb => {
                let mut byte = [0u8; 1];
                self.lfb_read(crate::api::WindowOffset(offset), 1, &mut byte);
                byte[0]
            }
            // The card owns its register block; a core with no extension has
            // no registers there to answer with.
            VgaWindow::Registers => 0xff,
        }
    }

    /// The write half of [`Self::window_read_byte`].
    pub(crate) fn window_write_byte(&mut self, window: VgaWindow, offset: u64, value: u8) {
        match window {
            VgaWindow::Legacy => self.legacy_write_byte(offset, value),
            VgaWindow::Lfb => {
                self.lfb_write(crate::api::WindowOffset(offset), 1, &[value])
            }
            VgaWindow::Registers => {}
        }
    }

    /// One byte into the legacy aperture — the write half of
    /// [`Self::legacy_read_byte`], and per byte for the same reason.
    pub(crate) fn legacy_write_byte(&mut self, offset: u64, value: u8) {
        vga_mem_write_byte(self, VGA_WINDOW_GRAPHICS_BASE + offset, value);
    }

    /// Write to the linear framebuffer behind BAR0. Bochs vga.cc `mem_write`'s
    /// LFB arm: chain-4 writes take the raw offset over the full 256 KB, and a
    /// non-chain-4 write past the aperture is dropped.
    ///
    /// Present in every build for the reason [`Self::lfb_read`] gives.
    pub(crate) fn lfb_write(
        &mut self,
        at: crate::api::WindowOffset,
        len: u32,
        data: &[u8],
    ) {
        #[cfg(not(feature = "alloc"))]
        {
            let _ = (at, len, data);
            tracing::error!("VGA: framebuffer write with no backing store");
        }
        #[cfg(feature = "alloc")]
        for i in 0..len as u64 {
            let Some(&value) = data.get(i as usize) else {
                break;
            };
            let offset = at.get() + i;
            if self.seq_chain_four && offset < 0x40000 {
                vga_mem_write_byte(self, offset, value);
            }
        }
    }
}

/// The adapter's three windows, each answering only what the machine routed to
/// it. Replaces `is_mmio_addr`, which existed solely to re-derive this split
/// from the physical address.
impl crate::api::MmioDevice for VgaCore {
    fn mmio_read(
        &mut self,
        window: crate::api::WindowId,
        at: crate::api::WindowOffset,
        len: u32,
        data: &mut [u8],
        _ctx: &mut crate::api::DeviceCtx<'_>,
    ) {
        match VgaWindow::from_id(window) {
            Some(VgaWindow::Legacy) => self.legacy_read(at, len, data),
            Some(VgaWindow::Lfb) => self.lfb_read(at, len, data),
            Some(VgaWindow::Registers) => data.fill(0xff),
            None => {
                tracing::error!("VGA: read from window {window:?}, which it never declared");
                data.fill(0xff);
            }
        }
    }

    fn mmio_write(
        &mut self,
        window: crate::api::WindowId,
        at: crate::api::WindowOffset,
        len: u32,
        data: &[u8],
        _ctx: &mut crate::api::DeviceCtx<'_>,
    ) {
        match VgaWindow::from_id(window) {
            Some(VgaWindow::Legacy) => self.legacy_write(at, len, data),
            Some(VgaWindow::Lfb) => self.lfb_write(at, len, data),
            Some(VgaWindow::Registers) => {}
            None => {
                tracing::error!("VGA: write to window {window:?}, which it never declared")
            }
        }
    }
}

/// Planar video memory, which is the storage in every mode the core renders.
///
/// A DISPI mode at 4bpp renders through `get_vga_pixel` from this same planar
/// memory, exactly as Bochs's 4bpp path calls `bx_vgacore_c`'s — so the linear
/// framebuffer is not a second copy of it, and the core needs no view of one.
fn vga_storage_get(vga: &VgaCore, index: usize) -> u8 {
    #[cfg(feature = "alloc")]
    if vga.vbe_planar_alias {
        return vga.vbe_memory.get(index).copied().unwrap_or(0);
    }
    vga.vga_memory.get(index).copied().unwrap_or(0)
}

/// The write half of [`vga_storage_get`]. Under the alias both stores are
/// written, so a mode change in either direction finds the bytes where it
/// expects them.
fn vga_storage_set(vga: &mut VgaCore, index: usize, value: u8) {
    #[cfg(feature = "alloc")]
    if vga.vbe_planar_alias {
        if let Some(slot) = vga.vbe_memory.get_mut(index) {
            *slot = value;
        }
    }
    if let Some(slot) = vga.vga_memory.get_mut(index) {
        *slot = value;
    }
}

/// Read a single byte from VGA memory. Matches Bochs vgacore.cc `mem_read`.
fn vga_mem_read_byte(vga: &mut VgaCore, addr: BxPhyAddress) -> u8 {
    let mut read_map_select = vga.graphics_regs[GFX_REG_READ_MAP_SELECT] & 0x03;

    // Window gating: compute offset from address (Bochs vgacore.cc)
    let memory_mapping =
        (vga.graphics_regs[GFX_REG_MISC] >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK;
    let mut offset = if addr >= 0xA0000 {
        match memory_mapping {
            1 => {
                // 0xA0000..0xAFFFF
                if addr > 0xAFFFF {
                    return 0xFF;
                }
                (addr & 0xFFFF) as u32
            }
            2 => {
                // 0xB0000..0xB7FFF
                if !(0xB0000..=0xB7FFF).contains(&addr) {
                    return 0xFF;
                }
                (addr & 0x7FFF) as u32
            }
            3 => {
                // 0xB8000..0xBFFFF
                if addr < 0xB8000 {
                    return 0xFF;
                }
                (addr & 0x7FFF) as u32
            }
            _ => {
                // 0xA0000..0xBFFFF
                (addr & 0x1FFFF) as u32
            }
        }
    } else {
        addr as u32
    };
    offset = offset.wrapping_add(vga.ext_read_offset);

    // Chain-four mode (Mode 13h: 320x200x256)
    if vga.seq_chain_four {
        return vga_storage_get(vga, offset as usize);
    }

    // Read mode (graphics_regs[5] bit 3)
    let read_mode = (vga.graphics_regs[GFX_REG_GRAPHICS_MODE] >> 3) & 0x01;

    match read_mode {
        0 => {
            // Read mode 0: load all 4 planes into latch, return selected plane
            // Bochs vgacore.cc
            if !vga.seq_odd_even_dis {
                // Odd/even mode: adjacent byte addresses alternate between plane pairs
                let base = ((offset & !1) << 2) as usize;
                vga.latch[0] = vga_storage_get(vga, base);
                vga.latch[1] = vga_storage_get(vga, base + 1);
                vga.latch[2] = vga_storage_get(vga, base + 2);
                vga.latch[3] = vga_storage_get(vga, base + 3);
                read_map_select = (read_map_select & 2) | (offset as u8 & 1);
            } else {
                // Normal planar mode
                let base = (offset << 2) as usize;
                vga.latch[0] = vga_storage_get(vga, base);
                vga.latch[1] = vga_storage_get(vga, base + 1);
                vga.latch[2] = vga_storage_get(vga, base + 2);
                vga.latch[3] = vga_storage_get(vga, base + 3);
            }
            vga.latch[read_map_select as usize & 3]
        }
        _ => {
            // Read mode 1: color compare
            // Bochs vgacore.cc
            let color_compare = (vga.graphics_regs[GFX_REG_COLOR_COMPARE] & 0x0F) as usize;
            let color_dont_care = (vga.graphics_regs[GFX_REG_COLOR_DONT_CARE] & 0x0F) as usize;

            let base = (offset << 2) as usize;
            let mut latch0 = vga_storage_get(vga, base);
            let mut latch1 = vga_storage_get(vga, base + 1);
            let mut latch2 = vga_storage_get(vga, base + 2);
            let mut latch3 = vga_storage_get(vga, base + 3);

            vga.latch[0] = latch0;
            vga.latch[1] = latch1;
            vga.latch[2] = latch2;
            vga.latch[3] = latch3;

            latch0 ^= CCDAT[color_compare][0];
            latch1 ^= CCDAT[color_compare][1];
            latch2 ^= CCDAT[color_compare][2];
            latch3 ^= CCDAT[color_compare][3];

            latch0 &= CCDAT[color_dont_care][0];
            latch1 &= CCDAT[color_dont_care][1];
            latch2 &= CCDAT[color_dont_care][2];
            latch3 &= CCDAT[color_dont_care][3];

            !(latch0 | latch1 | latch2 | latch3)
        }
    }
}

/// Write a single byte to VGA memory. Matches Bochs vgacore.cc `mem_write`.
fn vga_mem_write_byte(vga: &mut VgaCore, addr: BxPhyAddress, value: u8) {
    let sequ_map_mask = vga.seq_regs[SEQ_REG_MAP_MASK] & 0x0F;
    let graphics_alpha = (vga.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) != 0;

    // Window gating: compute offset (Bochs vgacore.cc)
    let memory_mapping =
        (vga.graphics_regs[GFX_REG_MISC] >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK;
    let mut offset = if addr >= 0xA0000 {
        match memory_mapping {
            1 => {
                // 0xA0000..0xAFFFF
                if !(0xA0000..=0xAFFFF).contains(&addr) {
                    return;
                }
                (addr & 0xFFFF) as u32
            }
            2 => {
                // 0xB0000..0xB7FFF
                if !(0xB0000..=0xB7FFF).contains(&addr) {
                    return;
                }
                (addr & 0x7FFF) as u32
            }
            3 => {
                // 0xB8000..0xBFFFF
                if !(0xB8000..=0xBFFFF).contains(&addr) {
                    return;
                }
                (addr & 0x7FFF) as u32
            }
            _ => {
                // 0xA0000..0xBFFFF
                if !(0xA0000..=0xBFFFF).contains(&addr) {
                    return;
                }
                (addr & 0x1FFFF) as u32
            }
        }
    } else {
        addr as u32
    };
    offset = offset.wrapping_add(vga.ext_offset);


    // Chain-four mode (Mode 13h: 320x200x256) — Bochs vgacore.cc
    if vga.seq_chain_four {
        vga_storage_set(vga, offset as usize, value);
        vga.vga_mem_updated |= 1 << (offset % 4) as u8;
        #[cfg(feature = "alloc")]
        if graphics_alpha {
            vga.mark_legacy_dirty_offset(offset);
        }
        return;
    }

    // Compute new_val[4] based on write mode — Bochs vgacore.cc
    let mut new_val = [0u8; 4];
    let write_mode = vga.graphics_regs[GFX_REG_GRAPHICS_MODE] & 0x03;
    let mut value = value;

    match write_mode {
        0 => {
            // Write mode 0 — Bochs vgacore.cc
            let bitmask = vga.graphics_regs[GFX_REG_BIT_MASK];
            let set_reset = vga.graphics_regs[GFX_REG_SET_RESET];
            let enable_set_reset = vga.graphics_regs[GFX_REG_ENABLE_SET_RESET];
            let data_rotate = vga.graphics_regs[GFX_REG_DATA_ROTATE] & 0x07;
            let raster_op = (vga.graphics_regs[GFX_REG_DATA_ROTATE] >> 3) & 0x03;

            // Rotate CPU data
            if data_rotate > 0 {
                value = value.rotate_right(data_rotate.into());
            }

            // Start from latch values masked by ~bitmask
            new_val[0] = vga.latch[0] & !bitmask;
            new_val[1] = vga.latch[1] & !bitmask;
            new_val[2] = vga.latch[2] & !bitmask;
            new_val[3] = vga.latch[3] & !bitmask;

            match raster_op {
                0 => {
                    // Replace
                    new_val[0] |= if (enable_set_reset & 1) != 0 {
                        if (set_reset & 1) != 0 {
                            bitmask
                        } else {
                            0
                        }
                    } else {
                        value & bitmask
                    };
                    new_val[1] |= if (enable_set_reset & 2) != 0 {
                        if (set_reset & 2) != 0 {
                            bitmask
                        } else {
                            0
                        }
                    } else {
                        value & bitmask
                    };
                    new_val[2] |= if (enable_set_reset & 4) != 0 {
                        if (set_reset & 4) != 0 {
                            bitmask
                        } else {
                            0
                        }
                    } else {
                        value & bitmask
                    };
                    new_val[3] |= if (enable_set_reset & 8) != 0 {
                        if (set_reset & 8) != 0 {
                            bitmask
                        } else {
                            0
                        }
                    } else {
                        value & bitmask
                    };
                }
                1 => {
                    // AND
                    new_val[0] |= if (enable_set_reset & 1) != 0 {
                        if (set_reset & 1) != 0 {
                            vga.latch[0] & bitmask
                        } else {
                            0
                        }
                    } else {
                        (value & vga.latch[0]) & bitmask
                    };
                    new_val[1] |= if (enable_set_reset & 2) != 0 {
                        if (set_reset & 2) != 0 {
                            vga.latch[1] & bitmask
                        } else {
                            0
                        }
                    } else {
                        (value & vga.latch[1]) & bitmask
                    };
                    new_val[2] |= if (enable_set_reset & 4) != 0 {
                        if (set_reset & 4) != 0 {
                            vga.latch[2] & bitmask
                        } else {
                            0
                        }
                    } else {
                        (value & vga.latch[2]) & bitmask
                    };
                    new_val[3] |= if (enable_set_reset & 8) != 0 {
                        if (set_reset & 8) != 0 {
                            vga.latch[3] & bitmask
                        } else {
                            0
                        }
                    } else {
                        (value & vga.latch[3]) & bitmask
                    };
                }
                2 => {
                    // OR
                    new_val[0] |= if (enable_set_reset & 1) != 0 {
                        if (set_reset & 1) != 0 {
                            bitmask
                        } else {
                            vga.latch[0] & bitmask
                        }
                    } else {
                        (value | vga.latch[0]) & bitmask
                    };
                    new_val[1] |= if (enable_set_reset & 2) != 0 {
                        if (set_reset & 2) != 0 {
                            bitmask
                        } else {
                            vga.latch[1] & bitmask
                        }
                    } else {
                        (value | vga.latch[1]) & bitmask
                    };
                    new_val[2] |= if (enable_set_reset & 4) != 0 {
                        if (set_reset & 4) != 0 {
                            bitmask
                        } else {
                            vga.latch[2] & bitmask
                        }
                    } else {
                        (value | vga.latch[2]) & bitmask
                    };
                    new_val[3] |= if (enable_set_reset & 8) != 0 {
                        if (set_reset & 8) != 0 {
                            bitmask
                        } else {
                            vga.latch[3] & bitmask
                        }
                    } else {
                        (value | vga.latch[3]) & bitmask
                    };
                }
                _ => {
                    // XOR
                    new_val[0] |= if (enable_set_reset & 1) != 0 {
                        if (set_reset & 1) != 0 {
                            !vga.latch[0] & bitmask
                        } else {
                            vga.latch[0] & bitmask
                        }
                    } else {
                        (value ^ vga.latch[0]) & bitmask
                    };
                    new_val[1] |= if (enable_set_reset & 2) != 0 {
                        if (set_reset & 2) != 0 {
                            !vga.latch[1] & bitmask
                        } else {
                            vga.latch[1] & bitmask
                        }
                    } else {
                        (value ^ vga.latch[1]) & bitmask
                    };
                    new_val[2] |= if (enable_set_reset & 4) != 0 {
                        if (set_reset & 4) != 0 {
                            !vga.latch[2] & bitmask
                        } else {
                            vga.latch[2] & bitmask
                        }
                    } else {
                        (value ^ vga.latch[2]) & bitmask
                    };
                    new_val[3] |= if (enable_set_reset & 8) != 0 {
                        if (set_reset & 8) != 0 {
                            !vga.latch[3] & bitmask
                        } else {
                            vga.latch[3] & bitmask
                        }
                    } else {
                        (value ^ vga.latch[3]) & bitmask
                    };
                }
            }
        }
        1 => {
            // Write mode 1: latch copy — Bochs vgacore.cc
            new_val[0] = vga.latch[0];
            new_val[1] = vga.latch[1];
            new_val[2] = vga.latch[2];
            new_val[3] = vga.latch[3];
        }
        2 => {
            // Write mode 2 — Bochs vgacore.cc
            let bitmask = vga.graphics_regs[GFX_REG_BIT_MASK];
            let raster_op = (vga.graphics_regs[GFX_REG_DATA_ROTATE] >> 3) & 0x03;

            new_val[0] = vga.latch[0] & !bitmask;
            new_val[1] = vga.latch[1] & !bitmask;
            new_val[2] = vga.latch[2] & !bitmask;
            new_val[3] = vga.latch[3] & !bitmask;

            match raster_op {
                0 => {
                    // Write
                    new_val[0] |= if (value & 1) != 0 { bitmask } else { 0 };
                    new_val[1] |= if (value & 2) != 0 { bitmask } else { 0 };
                    new_val[2] |= if (value & 4) != 0 { bitmask } else { 0 };
                    new_val[3] |= if (value & 8) != 0 { bitmask } else { 0 };
                }
                1 => {
                    // AND
                    new_val[0] |= if (value & 1) != 0 {
                        vga.latch[0] & bitmask
                    } else {
                        0
                    };
                    new_val[1] |= if (value & 2) != 0 {
                        vga.latch[1] & bitmask
                    } else {
                        0
                    };
                    new_val[2] |= if (value & 4) != 0 {
                        vga.latch[2] & bitmask
                    } else {
                        0
                    };
                    new_val[3] |= if (value & 8) != 0 {
                        vga.latch[3] & bitmask
                    } else {
                        0
                    };
                }
                2 => {
                    // OR
                    new_val[0] |= if (value & 1) != 0 {
                        bitmask
                    } else {
                        vga.latch[0] & bitmask
                    };
                    new_val[1] |= if (value & 2) != 0 {
                        bitmask
                    } else {
                        vga.latch[1] & bitmask
                    };
                    new_val[2] |= if (value & 4) != 0 {
                        bitmask
                    } else {
                        vga.latch[2] & bitmask
                    };
                    new_val[3] |= if (value & 8) != 0 {
                        bitmask
                    } else {
                        vga.latch[3] & bitmask
                    };
                }
                _ => {
                    // XOR
                    new_val[0] |= if (value & 1) != 0 {
                        !vga.latch[0] & bitmask
                    } else {
                        vga.latch[0] & bitmask
                    };
                    new_val[1] |= if (value & 2) != 0 {
                        !vga.latch[1] & bitmask
                    } else {
                        vga.latch[1] & bitmask
                    };
                    new_val[2] |= if (value & 4) != 0 {
                        !vga.latch[2] & bitmask
                    } else {
                        vga.latch[2] & bitmask
                    };
                    new_val[3] |= if (value & 8) != 0 {
                        !vga.latch[3] & bitmask
                    } else {
                        vga.latch[3] & bitmask
                    };
                }
            }
        }
        _ => {
            // Write mode 3 — Bochs vgacore.cc
            let data_rotate = vga.graphics_regs[GFX_REG_DATA_ROTATE] & 0x07;
            let raster_op = (vga.graphics_regs[GFX_REG_DATA_ROTATE] >> 3) & 0x03;
            let set_reset = vga.graphics_regs[GFX_REG_SET_RESET];

            // Rotate CPU data
            if data_rotate > 0 {
                value = value.rotate_right(data_rotate.into());
            }

            let bitmask = vga.graphics_regs[GFX_REG_BIT_MASK] & value;

            new_val[0] = vga.latch[0] & !bitmask;
            new_val[1] = vga.latch[1] & !bitmask;
            new_val[2] = vga.latch[2] & !bitmask;
            new_val[3] = vga.latch[3] & !bitmask;

            // value &= bitmask (Bochs line 2082) — but value is only used in
            // set_reset expansion below, not directly
            let masked_value = value & bitmask;

            match raster_op {
                0 => {
                    // Write
                    new_val[0] |= if (set_reset & 1) != 0 {
                        masked_value
                    } else {
                        0
                    };
                    new_val[1] |= if (set_reset & 2) != 0 {
                        masked_value
                    } else {
                        0
                    };
                    new_val[2] |= if (set_reset & 4) != 0 {
                        masked_value
                    } else {
                        0
                    };
                    new_val[3] |= if (set_reset & 8) != 0 {
                        masked_value
                    } else {
                        0
                    };
                }
                1 => {
                    // AND
                    new_val[0] |= (if (set_reset & 1) != 0 {
                        masked_value
                    } else {
                        0
                    }) & vga.latch[0];
                    new_val[1] |= (if (set_reset & 2) != 0 {
                        masked_value
                    } else {
                        0
                    }) & vga.latch[1];
                    new_val[2] |= (if (set_reset & 4) != 0 {
                        masked_value
                    } else {
                        0
                    }) & vga.latch[2];
                    new_val[3] |= (if (set_reset & 8) != 0 {
                        masked_value
                    } else {
                        0
                    }) & vga.latch[3];
                }
                2 => {
                    // OR
                    new_val[0] |= (if (set_reset & 1) != 0 {
                        masked_value
                    } else {
                        0
                    }) | vga.latch[0];
                    new_val[1] |= (if (set_reset & 2) != 0 {
                        masked_value
                    } else {
                        0
                    }) | vga.latch[1];
                    new_val[2] |= (if (set_reset & 4) != 0 {
                        masked_value
                    } else {
                        0
                    }) | vga.latch[2];
                    new_val[3] |= (if (set_reset & 8) != 0 {
                        masked_value
                    } else {
                        0
                    }) | vga.latch[3];
                }
                _ => {
                    // XOR
                    new_val[0] |= (if (set_reset & 1) != 0 {
                        masked_value
                    } else {
                        0
                    }) ^ vga.latch[0];
                    new_val[1] |= (if (set_reset & 2) != 0 {
                        masked_value
                    } else {
                        0
                    }) ^ vga.latch[1];
                    new_val[2] |= (if (set_reset & 4) != 0 {
                        masked_value
                    } else {
                        0
                    }) ^ vga.latch[2];
                    new_val[3] |= (if (set_reset & 8) != 0 {
                        masked_value
                    } else {
                        0
                    }) ^ vga.latch[3];
                }
            }
        }
    }

    // Commit new_val to planar memory — Bochs vgacore.cc
    if !vga.seq_odd_even_dis {
        // Odd/even mode — Bochs vgacore.cc
        let plane = (offset & 1) as u8;
        let mask = sequ_map_mask & (0x05 << plane);
        if mask > 0 {
            if (mask & 0x03) != 0 {
                let final_val = new_val[plane as usize];
                let mem_idx = (((offset & !1) << 2) | plane as u32) as usize;
                vga_storage_set(vga, mem_idx, final_val);
                vga.vga_mem_updated |= 1 << plane;
            } else {
                let final_val = new_val[(plane + 2) as usize];
                let mem_idx = (((offset & !1) << 2) | (plane as u32 + 2)) as usize;
                vga_storage_set(vga, mem_idx, final_val);
                vga.vga_mem_updated |= 4 << plane;
            }
            if !graphics_alpha {
                // Text mode: update text_buffer (Bochs vgacore.cc)
                let mem_mask = TEXT_SNAP_SIZE[memory_mapping as usize & 3] - 1;
                let text_offset = (offset as usize) & mem_mask;
                // In odd/even text mode, plane 0 = chars, plane 1 = attrs.
                // The final value written was for the selected plane.
                let write_val = if (mask & 0x03) != 0 {
                    new_val[plane as usize]
                } else {
                    new_val[(plane + 2) as usize]
                };
                if let Some(slot) = vga.text_memory.get_mut(text_offset) {
                    if *slot != write_val {
                        *slot = write_val;
                        vga.text_dirty = true;
                    }
                }
            }
            #[cfg(feature = "alloc")]
            if graphics_alpha {
                vga.mark_legacy_dirty_offset(offset);
            }
        }
    } else {
        // Normal planar mode (odd_even_dis=true) — Bochs vgacore.cc
        if (sequ_map_mask & 0x0F) != 0 {
            vga.vga_mem_updated |= sequ_map_mask;
            let base = (offset << 2) as usize;
            if (sequ_map_mask & 0x01) != 0 {
                vga_storage_set(vga, base, new_val[0]);
            }
            if (sequ_map_mask & 0x02) != 0 {
                vga_storage_set(vga, base + 1, new_val[1]);
            }
            if (sequ_map_mask & 0x04) != 0 {
                vga_storage_set(vga, base + 2, new_val[2]);
            }
            if (sequ_map_mask & 0x08) != 0 {
                vga_storage_set(vga, base + 3, new_val[3]);
            }

            if !graphics_alpha {
                // Text mode: update text_buffer (Bochs vgacore.cc)
                // In planar text mode, write value to text_memory for rendering
                let mem_mask = TEXT_SNAP_SIZE[memory_mapping as usize & 3] - 1;
                let text_offset = (offset as usize) & mem_mask;
                // Write plane 0 value as the character / attribute byte
                // (plane selection already handled by map_mask)
                if (sequ_map_mask & 0x03) != 0 {
                    // Planes 0 or 1 are text-relevant
                    let write_val = if (sequ_map_mask & 0x01) != 0 {
                        new_val[0]
                    } else {
                        new_val[1]
                    };
                    if let Some(slot) = vga.text_memory.get_mut(text_offset) {
                        if *slot != write_val {
                            *slot = write_val;
                            vga.text_dirty = true;
                        }
                    }
                }
            }
            #[cfg(feature = "alloc")]
            if graphics_alpha {
                vga.mark_legacy_dirty_offset(offset);
            }
        }
    }
}

// =============================================================================
// VBE MMIO handlers for BAR2 (QEMU-compatible, used by OVMF QemuVideoDxe)
// =============================================================================
// BAR2 MMIO layout:
//   0x000-0x3FF: VBE index registers (EDID/DDC, currently unimplemented)
//   0x400-0x4FF: VBE EDID data (currently unimplemented)
//   0x500-0x515: Bochs VBE extension registers (PCI_VGA_BOCHS_OFFSET)
//
// When PCI is enabled the BAR2 window is registered as its own memory handler
// on BAR2 commit, so an access arrives already named as `VgaWindow::Registers`
// with an offset from the window base, and routes to vbe_mmio_read /
// vbe_mmio_write. BAR0 is the linear framebuffer.

/// Result of a PCI config write: which BAR (if any) queued a new base for
/// transactional memory-handler relocation.
#[derive(Debug, Default, Clone, Copy)]
pub struct VgaBarChange {
    pub lfb: bool,
    pub mmio: bool,
}

impl VgaCore {




}


impl VgaCore {







}

#[cfg(feature = "std")]
fn invalid_vga_snapshot(message: &'static str) -> SnapError {
    SnapError::Invalid(message)
}

#[cfg(feature = "std")]
fn snapshot_v3_usize_len(len: usize) -> SnapResult<u64> {
    let len = u64::try_from(len)
        .map_err(|_| invalid_vga_snapshot("VGA buffer length does not fit u64"))?;
    if len > MAX_SECTION_LEN {
        return Err(invalid_vga_snapshot(
            "VGA buffer length exceeds snapshot section bound",
        ));
    }
    Ok(len)
}

#[cfg(feature = "std")]
fn write_snapshot_u32_len<W: SnapWrite>(writer: &mut W, len: usize) -> SnapResult<()> {
    let len = snapshot_v3_usize_len(len)?;
    writer.write_u32(
        u32::try_from(len)
            .map_err(|_| invalid_vga_snapshot("VGA fixed-buffer length does not fit u32"))?,
    )
}

#[cfg(feature = "std")]
fn read_snapshot_u32_len<R: SnapRead>(
    reader: &mut R,
    maximum: usize,
    _description: &'static str,
) -> SnapResult<usize> {
    reader.read_count(maximum)
}

#[cfg(feature = "std")]
fn read_snapshot_fixed_array<R: SnapRead, const N: usize>(
    reader: &mut R,
    bytes: &mut [u8; N],
    description: &'static str,
) -> SnapResult<()> {
    let len = read_snapshot_u32_len(reader, N, description)?;
    if len != N {
        return Err(invalid_vga_snapshot("VGA fixed-buffer length mismatch"));
    }
    reader.read_bytes(bytes)
}

#[cfg(feature = "std")]
fn vga_snapshot_pci_byte_is_mutable(index: usize) -> bool {
    matches!(index, 0x0c | 0x0d | 0x3c) || (0x10..=0x13).contains(&index) || (0x18..=0x1b).contains(&index)
}

#[cfg(feature = "std")]
fn vga_snapshot_bpp_is_valid(bpp: u16) -> bool {
    matches!(
        bpp,
        VBE_DISPI_BPP_4
            | VBE_DISPI_BPP_8
            | VBE_DISPI_BPP_15
            | VBE_DISPI_BPP_16
            | VBE_DISPI_BPP_24
            | VBE_DISPI_BPP_32
    )
}

#[cfg(feature = "std")]
fn vga_snapshot_vbe_layout(bpp: u16, virtual_xres: u16) -> SnapResult<(u8, u16)> {
    let bpp_multiplier = match bpp {
        VBE_DISPI_BPP_4 | VBE_DISPI_BPP_8 => 1,
        VBE_DISPI_BPP_15 | VBE_DISPI_BPP_16 => 2,
        VBE_DISPI_BPP_24 => 3,
        VBE_DISPI_BPP_32 => 4,
        _ => return Err(invalid_vga_snapshot("VBE bpp is invalid")),
    };
    let line_offset = if bpp == VBE_DISPI_BPP_4 {
        virtual_xres / 8
    } else {
        virtual_xres
            .checked_mul(u16::from(bpp_multiplier))
            .ok_or_else(|| invalid_vga_snapshot("VBE line offset overflows"))?
    };
    if line_offset == 0 {
        return Err(invalid_vga_snapshot("VBE line offset is zero"));
    }
    Ok((bpp_multiplier, line_offset))
}

#[cfg(feature = "std")]
fn vga_snapshot_bank_offset(bank: u16, bank_granularity_kb: u16) -> SnapResult<u32> {
    u32::from(bank)
        .checked_mul(
            u32::from(bank_granularity_kb)
                .checked_mul(1024)
                .ok_or_else(|| invalid_vga_snapshot("VBE bank granularity overflows"))?,
        )
        .ok_or_else(|| invalid_vga_snapshot("VBE bank offset overflows"))
}

#[cfg(feature = "std")]
fn validate_vga_snapshot_bar_base(base: u32, span: u32) -> SnapResult<()> {
    if span == 0 || !span.is_power_of_two() {
        return Err(invalid_vga_snapshot("VGA BAR span is invalid"));
    }
    if base & (span - 1) != 0 {
        return Err(invalid_vga_snapshot("VGA BAR base is misaligned"));
    }
    base.checked_add(span - 1)
        .ok_or_else(|| invalid_vga_snapshot("VGA BAR range overflows"))?;
    Ok(())
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use rusty_box_core::snap::SnapError;
    use crate::api::WindowOffset;
    use crate::display::sink::Redraw;

    /// A tick reading under a stated rate — what the card is handed now
    /// instead of a bare instruction count it would divide for itself.
    fn clock_at(ticks: u64) -> VmClock {
        VmClock::new(
            rusty_box_core::time::VmInstant::from_ticks(ticks),
            rusty_box_core::time::ClockHz::BOCHS_DEFAULT,
        )
    }

    /// Records what a front end would receive, so a test asserts the frame that
    /// reaches a display rather than an intermediate value on the way there.
    #[derive(Default)]
    struct RecordingSink {
        dimensions: Option<Dimensions>,
        tiles: alloc::vec::Vec<(TilePos, alloc::vec::Vec<u8>)>,
        text_frames: usize,
        cursor: Option<CursorPos>,
        charmaps: alloc::vec::Vec<usize>,
        palette: alloc::vec::Vec<(u8, Rgb)>,
        clears: usize,
        flushes: usize,
    }

    impl RecordingSink {
        /// The tile whose top-left corner is at `(x, y)`.
        fn tile_at(&self, x: u32, y: u32) -> &[u8] {
            self.tiles
                .iter()
                .find(|(at, _)| at.x == x && at.y == y)
                .map(|(_, rgba)| rgba.as_slice())
                .expect("no tile at that position")
        }
    }

    impl DisplaySink for RecordingSink {
        fn dimension_update(&mut self, dims: Dimensions) {
            self.dimensions = Some(dims);
        }
        fn text_update(
            &mut self,
            _previous: &[u8],
            _current: &[u8],
            cursor: Option<CursorPos>,
            _info: &VgaTextModeInfo,
        ) {
            self.text_frames += 1;
            self.cursor = cursor;
        }
        fn graphics_tile_update(&mut self, rgba: &[u8], at: TilePos) {
            self.tiles.push((at, rgba.to_vec()));
        }
        fn palette_change(&mut self, index: u8, colour: Rgb) -> Redraw {
            self.palette.push((index, colour));
            Redraw::NotNeeded
        }
        fn set_text_charmap(&mut self, map: usize, _glyphs: &[u8]) {
            self.charmaps.push(map);
        }
        fn clear_screen(&mut self) {
            self.clears += 1;
        }
        fn flush(&mut self) {
            self.flushes += 1;
        }
    }

    /// Draw one frame and report everything the front end saw.
    fn draw(vga: &mut VgaCard<StdVga>) -> RecordingSink {
        draw_with(vga, crate::display::card::Dirt::SelfTracked)
    }

    /// Draw one frame with a stated source of dirtiness — the hypervisor case,
    /// where the device observed none of the guest's writes.
    fn draw_with(
        vga: &mut VgaCard<StdVga>,
        dirt: crate::display::card::Dirt<'_>,
    ) -> RecordingSink {
        let mut sink = RecordingSink::default();
        vga.refresh(&mut sink, dirt);
        sink
    }

    /// A card, as the machine holds one.
    fn card() -> VgaCard<StdVga> {
        VgaCard::with_extension(StdVga::new())
    }

    /// A card answers where it says it answers.
    ///
    /// The declaration is the only thing the machine maps, so a window the card
    /// forgets to declare is an aperture the guest writes into with nothing
    /// behind it — and a mode the card does not support looks exactly the same
    /// from inside the guest. Both ranges are checked against Bochs: the legacy
    /// aperture `bx_vgacore_c::init` registers, and the framebuffer
    /// `bx_vga_c::init_vga_extension` registers at the card's own base.
    #[test]
    fn the_windows_a_card_declares_are_the_ones_it_answers_on() {
        let vga = card();
        let declared: Vec<_> = vga.windows().unwrap().as_slice().into_iter().collect();

        assert_eq!(
            declared[0].id,
            VgaWindow::Legacy.id(),
            "the core's aperture is declared first, as Bochs registers it first"
        );
        assert_eq!((declared[0].base, declared[0].end), (0xA_0000, 0xB_FFFF));

        assert_eq!(declared.len(), 2, "core aperture plus the card's framebuffer");
        assert_eq!(declared[1].id, VgaWindow::Lfb.id());
        assert_eq!(
            declared[1].base,
            u64::from(vga.ext.vbe.base_address),
            "the framebuffer window follows the card's base, not a constant"
        );
        assert_eq!(
            declared[1].end - declared[1].base + 1,
            u64::from(vga.ext.vbe_memsize),
            "and spans exactly the memory the card reports"
        );

        // Every declared window is one the card will actually be handed back.
        for decl in vga.windows().unwrap().as_slice() {
            assert!(
                VgaWindow::from_id(decl.id).is_some(),
                "declared window {:?} is not one this card routes on",
                decl.id
            );
        }
    }

    /// A card may not declare more windows than the bound allows, and if it
    /// tries, it is told — never silently truncated.
    #[test]
    fn a_card_that_declares_too_many_windows_is_refused() {
        use crate::api::{Declared, WindowDecl, WindowDecls, MAX_DEVICE_WINDOWS};

        let mut decls = WindowDecls::new();
        for index in 0..MAX_DEVICE_WINDOWS {
            assert_eq!(
                decls.push(WindowDecl {
                    id: crate::api::WindowId(index as u8),
                    base: 0,
                    end: 0,
                }),
                Declared::Accepted
            );
        }
        assert_eq!(
            decls.push(WindowDecl {
                id: crate::api::WindowId(9),
                base: 0,
                end: 0,
            }),
            Declared::NoRoom
        );
        assert_eq!(decls.as_slice().len(), MAX_DEVICE_WINDOWS);
    }

    /// Write video memory the way a guest access does — offered to the card a
    /// byte at a time, falling through to the core when it declines. Poking
    /// `core.legacy_write` instead would skip whichever half owns the mode.
    fn write_vram(vga: &mut VgaCard<StdVga>, window: VgaWindow, offset: u64, bytes: &[u8]) {
        use crate::display::card::{MemCtx, VgaExtension, Written};
        for (i, &byte) in bytes.iter().enumerate() {
            let at = offset + i as u64;
            let mut cx = MemCtx::new(&mut vga.core, window, WindowOffset(at));
            if vga.ext.vga_mem_write(&mut cx, byte) == Written::FallThrough {
                vga.core.window_write_byte(window, at, byte);
            }
        }
    }

    /// The read half of [`write_vram`].
    fn read_vram(vga: &mut VgaCard<StdVga>, window: VgaWindow, offset: u64, out: &mut [u8]) {
        use crate::display::card::{MemCtx, VgaExtension};
        for (i, slot) in out.iter_mut().enumerate() {
            let at = offset + i as u64;
            let mut cx = MemCtx::new(&mut vga.core, window, WindowOffset(at));
            *slot = match vga.ext.vga_mem_read(&mut cx) {
                Some(value) => value,
                None => vga.core.window_read_byte(window, at),
            };
        }
    }
    use crate::pci::PciDevice;
    #[cfg(feature = "std")]
    use rusty_box_core::snap::SnapshotSection;
    #[cfg(feature = "std")]


    /// Program one DISPI register the way a guest does — through the port hook
    /// the card claims, not by poking its state.
    fn write_vbe(vga: &mut VgaCard<StdVga>, index: u16, value: u16) {
        use crate::display::card::{PortCtx, VgaExtension};
        for (port, value) in [
            (VBE_DISPI_IOPORT_INDEX, u32::from(index)),
            (VBE_DISPI_IOPORT_DATA, u32::from(value)),
        ] {
            let mut cx = PortCtx::new(&mut vga.core, port, 2);
            vga.ext.vga_pio_write(&mut cx, value);
        }
    }

    fn pci_vga() -> VgaCard<StdVga> {
        let mut vga = card();
        vga.enable_pci();
        vga
    }

    /// A colour-text adapter whose CRTC describes `cols` columns of
    /// `char_height`-scanline cells over a `scan_lines`-line display.
    fn text_mode_vga(cols: u8, char_height: u8, scan_lines: u16) -> VgaCore {
        let mut vga = VgaCore::new();
        vga.graphics_regs[GFX_REG_MISC] =
            (VgaMemoryMapping::ColorText32k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        vga.crtc_regs[CRTC_HORIZ_DISPLAY_END] = cols.saturating_sub(1);
        vga.crtc_regs[CRTC_MAX_SCAN_LINE] = char_height.saturating_sub(1) & CRTC_MSL_MASK;
        let vde = scan_lines - 1;
        vga.crtc_regs[CRTC_VERT_DISPLAY_END] = (vde & 0xFF) as u8;
        let mut overflow = 0u8;
        if vde & 0x100 != 0 {
            overflow |= CRTC_OVERFLOW_VDE_BIT8;
        }
        if vde & 0x200 != 0 {
            overflow |= CRTC_OVERFLOW_VDE_BIT9;
        }
        vga.crtc_regs[CRTC_OVERFLOW] = overflow;
        vga.crtc_regs[CRTC_OFFSET] = (u16::from(cols) * BYTES_PER_CHAR as u16 / 4) as u8;
        vga
    }

    /// Bochs vgacore.cc update() derives the row count from the display end and
    /// the cell height, and caps nothing: an 80x50 guest has fifty rows.
    #[test]
    fn a_fifty_row_text_mode_reports_all_fifty_rows() {
        let vga = text_mode_vga(80, 8, 400);
        let geometry = vga.text_geometry().expect("text mode");
        assert_eq!(geometry.cols, 80);
        assert_eq!(geometry.rows, 50);
        assert_eq!(geometry.char_height, 8);
        assert_eq!(geometry.pixel_height, 400);
    }

    /// Linux's vgacon loads a console font by moving the memory map to A0000
    /// while the adapter stays alphanumeric (GR06 = 0x00). Bochs vgacore.cc
    /// `update()` decides text or graphics on `graphics_alpha` alone, so that
    /// is still a character grid.
    #[test]
    fn an_alphanumeric_adapter_mapped_at_a0000_is_still_text() {
        let mut vga = text_mode_vga(80, 16, 400);
        vga.graphics_regs[GFX_REG_MISC] = 0x00;
        assert!(vga.in_text_mode());
        let geometry = vga.text_geometry().expect("still a character grid");
        assert_eq!((geometry.cols, geometry.rows), (80, 25));
    }

    /// The same register state reaches the front end as a text frame. Drawn
    /// through the planar graphics path instead, the character and attribute
    /// bytes decode as pixels: regular stripes wherever the screen holds spaces.
    #[test]
    fn a_font_load_draws_a_text_frame_not_planar_pixels() {
        let mut vga = card();
        vga.core.vga_enabled = true;
        vga.core.video_enabled = true;
        vga.core.seq_regs[SEQ_REG_RESET] = 0x03;
        vga.core.crtc_regs[CRTC_HORIZ_DISPLAY_END] = 79;
        vga.core.crtc_regs[CRTC_MAX_SCAN_LINE] = 15 & CRTC_MSL_MASK;
        vga.core.crtc_regs[CRTC_VERT_DISPLAY_END] = (399u16 & 0xFF) as u8;
        vga.core.crtc_regs[CRTC_OVERFLOW] = CRTC_OVERFLOW_VDE_BIT8;
        vga.core.crtc_regs[CRTC_OFFSET] = 40;
        vga.core.graphics_regs[GFX_REG_MISC] = 0x00;

        let sink = draw(&mut vga);
        assert!(sink.tiles.is_empty(), "no planar tiles for an alphanumeric adapter");
        assert!(sink.text_frames > 0, "the frame is drawn as text");
        assert_eq!(sink.dimensions.map(|dims| dims.font_height), Some(16));
    }

    /// The cursor is valid anywhere on the displayed page. Capping the page at
    /// 25 rows made a cursor below row 25 read as absent, so a guest in 80x50
    /// drew no cursor at all past the halfway line.
    #[test]
    fn a_cursor_below_the_first_twenty_five_rows_is_still_on_screen() {
        let mut vga = text_mode_vga(80, 8, 400);
        // Row 30, column 0.
        let cell = 30u16 * 80;
        vga.crtc_regs[CRTC_CURSOR_LOC_HIGH] = (cell >> 8) as u8;
        vga.crtc_regs[CRTC_CURSOR_LOC_LOW] = (cell & 0xFF) as u8;

        let geometry = vga.text_geometry().expect("text mode");
        assert_ne!(geometry.cursor_address, VgaTextGeometry::CURSOR_OFF);
        assert_eq!(geometry.cursor_address, cell * 2);
    }

    /// A cursor parked past the last displayed row reads as absent, which is
    /// the `0x7fff` Bochs substitutes.
    #[test]
    fn a_cursor_past_the_last_row_reads_as_absent() {
        let mut vga = text_mode_vga(80, 16, 400);
        let cell = 0x3F00u16;
        vga.crtc_regs[CRTC_CURSOR_LOC_HIGH] = (cell >> 8) as u8;
        vga.crtc_regs[CRTC_CURSOR_LOC_LOW] = (cell & 0xFF) as u8;

        let geometry = vga.text_geometry().expect("text mode");
        assert_eq!(geometry.cursor_address, VgaTextGeometry::CURSOR_OFF);
    }

    /// Bochs replaces the column count and the cell height together when the
    /// CRTC has not been programmed yet, so a half-written register pair still
    /// describes the standard 80x25 grid rather than a grid built from one
    /// real value and one default.
    #[test]
    fn an_unprogrammed_crtc_falls_back_to_the_whole_standard_grid() {
        let mut vga = text_mode_vga(80, 8, 400);
        vga.crtc_regs[CRTC_HORIZ_DISPLAY_END] = 0; // cols == 1

        let geometry = vga.text_geometry().expect("text mode");
        assert_eq!(geometry.cols, 80);
        assert_eq!(geometry.char_height, 16);
        assert_eq!(geometry.rows, 25);
    }

    /// The emulated CGA 160x100x16 mode programs two-scanline cells over a
    /// 400-line display; Bochs widens the cell to four so the grid comes out
    /// at 100 rows instead of 200.
    #[test]
    fn the_emulated_cga_160x100_mode_gets_a_hundred_rows() {
        let vga = text_mode_vga(80, 2, 400);
        let geometry = vga.text_geometry().expect("text mode");
        assert_eq!(geometry.rows, 100);
        assert_eq!(geometry.char_height, 4);
    }

    /// A grid that would run past the text aperture draws no frame at all —
    /// Bochs reports "text mode: out of memory" and returns.
    #[test]
    fn a_grid_too_large_for_the_text_aperture_draws_nothing() {
        // 512 rows of two scan lines each, at the widest row stride the CRTC
        // offset register can ask for.
        let mut vga = text_mode_vga(80, 2, 1024);
        vga.crtc_regs[CRTC_OFFSET] = 0xFF;
        assert_eq!(vga.text_geometry(), None);
    }

    /// Reading a real adapter through the display role sees the characters the
    /// guest wrote, at the grid the CRTC describes — not a fixed 80x25 one.
    #[test]
    fn the_display_role_reads_the_guest_text_plane() {
        use crate::display::{DisplaySource, TextPos};

        let mut vga = text_mode_vga(80, 8, 400);
        // "OK" at row 2, column 4 of the displayed page, char/attribute pairs.
        let geometry = vga.text_geometry().expect("text mode");
        let cell =
            usize::from(geometry.start_address) + 2 * usize::from(geometry.line_offset) + 4 * 2;
        vga.text_memory[cell] = b'O';
        vga.text_memory[cell + 2] = b'K';

        let grid = vga.text_grid().expect("text mode");
        assert_eq!(grid.rows, 50);
        assert_eq!(grid.cols, 80);
        assert_eq!(vga.text_char(TextPos::new(2, 4)), 'O');
        assert_eq!(vga.text_char(TextPos::new(2, 5)), 'K');
        assert_eq!(vga.text_char(TextPos::new(2, 6)), ' ');
        assert_eq!(vga.resolution(), crate::display::Resolution::new(720, 400));
    }

    /// The cursor's cell is reported in grid coordinates, which is what a
    /// caller reading the screen can act on.
    #[test]
    fn the_display_role_reports_the_cursor_cell() {
        use crate::display::{DisplaySource, TextPos};

        let mut vga = text_mode_vga(80, 8, 400);
        let cell = 30u16 * 80 + 7;
        vga.crtc_regs[CRTC_CURSOR_LOC_HIGH] = (cell >> 8) as u8;
        vga.crtc_regs[CRTC_CURSOR_LOC_LOW] = (cell & 0xFF) as u8;

        let grid = vga.text_grid().expect("text mode");
        assert_eq!(grid.cursor, Some(TextPos::new(30, 7)));
    }

    /// A graphics mode has no character grid to report.
    #[test]
    fn a_graphics_mode_has_no_text_geometry() {
        let mut vga = text_mode_vga(80, 16, 400);
        vga.graphics_regs[GFX_REG_MISC] |= GFX_MISC_GRAPHICS_ALPHA;
        assert_eq!(vga.text_geometry(), None);
    }

    #[test]
    fn pci_disabled_is_invisible_to_enumeration() {
        let vga = card();
        assert!(!vga.ext.pci_enabled());
        assert_eq!(vga.pci_read(0x00, 4), 0xFFFF_FFFF);
        // Writes are ignored, no BAR change signalled.
        let mut vga = vga;
        let change = vga.pci_write(0x10, 0xE800_0000, 4);
        assert!(!change.lfb && !change.mmio);
    }

    #[test]
    fn pci_identity_class_and_bar0_type() {
        let vga = pci_vga();
        assert_eq!(vga.pci_read(0x00, 4), 0x1111_1234); // device<<16 | vendor
        assert_eq!(vga.pci_read(0x08, 4), 0x0300_0000); // rev/prog-if/subclass/class
        assert_eq!(vga.pci_read(0x10, 1) & 0x0F, 0x08); // BAR0 prefetchable memory
    }

    #[test]
    fn bar0_size_probe_returns_16mb_mask() {
        let mut vga = pci_vga();
        vga.pci_write(0x10, 0xFFFF_FFFF, 4);
        assert_eq!(vga.pci_read(0x10, 4), 0xFF00_0008); // ~(16MiB-1) | prefetchable
        assert!(
            vga.peek_pending_lfb_relocate().is_none(),
            "probe must not queue a relocation"
        );
    }

    #[test]
    fn bar2_size_probe_returns_4kb_mask() {
        let mut vga = pci_vga();
        vga.pci_write(0x18, 0xFFFF_FFFF, 4);
        assert_eq!(vga.pci_read(0x18, 4), 0xFFFF_F000); // ~(4KiB-1)
        assert!(vga.peek_pending_mmio_relocate().is_none());
    }

    #[test]
    fn bar0_write_queues_lfb_relocation_and_preserves_type_bits() {
        let mut vga = pci_vga();
        let change = vga.pci_write(0x10, 0xE800_0000, 4);
        assert!(change.lfb && !change.mmio);
        assert_eq!(
            vga.peek_pending_lfb_relocate(),
            Some((0xE000_0000, 0xE800_0000))
        );
        assert_eq!(vga.pci_read(0x10, 4), 0xE800_0008); // base + preserved type nibble
    }

    #[test]
    fn bar0_commit_to_same_base_is_a_noop() {
        let mut vga = pci_vga();
        let change = vga.pci_write(0x10, 0xE000_0000, 4); // equals the seeded init base
        assert!(!change.lfb);
        assert!(vga.peek_pending_lfb_relocate().is_none());
    }

    #[test]
    fn bar2_write_queues_mmio_registration() {
        let mut vga = pci_vga();
        let change = vga.pci_write(0x18, 0xF000_0000, 4);
        assert!(change.mmio && !change.lfb);
        assert_eq!(vga.peek_pending_mmio_relocate(), Some((0, 0xF000_0000)));
        assert_eq!(vga.pci_read(0x18, 4), 0xF000_0000);
    }
    #[test]
    fn lfb_relocation_stays_pending_until_commit() {
        let mut vga = pci_vga();
        vga.pci_write(0x10, 0xE800_0000, 4);

        assert_eq!(
            vga.peek_pending_lfb_relocate(),
            Some((0xE000_0000, 0xE800_0000))
        );
        assert_eq!(vga.ext.vbe.base_address, 0xE000_0000);

        assert_eq!(
            vga.commit_pending_lfb_relocate(),
            Some((0xE000_0000, 0xE800_0000))
        );
        assert_eq!(vga.ext.vbe.base_address, 0xE800_0000);
        assert!(vga.peek_pending_lfb_relocate().is_none());
    }

    #[test]
    fn pending_bar2_move_keeps_old_mapping_until_commit() {
        let mut vga = pci_vga();
        vga.pci_write(0x18, 0xF000_0000, 4);
        assert_eq!(
            vga.commit_pending_mmio_relocate(),
            Some((0, 0xF000_0000))
        );

        vga.pci_write(0x18, 0xF010_0000, 4);
        assert_eq!(
            vga.peek_pending_mmio_relocate(),
            Some((0xF000_0000, 0xF010_0000))
        );
        assert_eq!(
            vga.ext.mmio_base, 0xF000_0000,
            "the live window stays where the machine registered it"
        );

        assert_eq!(
            vga.commit_pending_mmio_relocate(),
            Some((0xF000_0000, 0xF010_0000))
        );
        assert_eq!(vga.ext.mmio_base, 0xF010_0000);
    }


    #[test]
    fn pci_command_register_is_read_only() {
        let mut vga = pci_vga();
        vga.pci_write(0x04, 0x00, 1);
        assert_eq!(vga.pci_read(0x04, 1), 0x03);
        vga.pci_write(0x04, 0xFF, 1);
        assert_eq!(vga.pci_read(0x04, 1), 0x03);
    }

    #[test]
    fn pci_non_command_writable_config_bytes_remain_writable() {
        let mut vga = pci_vga();
        for (address, value) in [(0x0C, 0xA5), (0x0D, 0x5A), (0x3C, 0x0B)] {
            vga.pci_write(address, value, 1);
            assert_eq!(vga.pci_read(address, 1), value);
        }
    }

    #[test]
    fn unimplemented_bars_read_back_zero() {
        let mut vga = pci_vga();
        for bar in [0x14u8, 0x1C, 0x20, 0x24, 0x30] {
            vga.pci_write(bar, 0xFFFF_FFFF, 4);
            assert_eq!(vga.pci_read(bar, 4), 0, "BAR/ROM at {bar:#x} must be 0");
        }
    }

    #[test]
    fn ids_and_class_are_read_only() {
        let mut vga = pci_vga();
        vga.pci_write(0x00, 0xDEAD_BEEF, 4);
        vga.pci_write(0x08, 0xFFFF_FFFF, 4);
        assert_eq!(vga.pci_read(0x00, 4), 0x1111_1234);
        assert_eq!(vga.pci_read(0x08, 4), 0x0300_0000);
    }

    #[test]
    fn pci_state_survives_reset() {
        let mut vga = pci_vga();
        vga.pci_write(0x10, 0xE800_0000, 4);
        let _ = vga.commit_pending_lfb_relocate(); // as the deferred handler would
        vga.reset();
        assert!(vga.ext.pci_enabled());
        assert_eq!(vga.pci_read(0x00, 4), 0x1111_1234);
        assert_eq!(vga.pci_read(0x10, 4), 0xE800_0008); // BAR persists
        assert_eq!(vga.pci_read(0x04, 1) & 0x03, 0x03); // command re-applied
    }

    #[test]
    fn preferred_mode_raises_caps_reallocs_tiles_and_survives_reset() {
        let mut vga = card();
        let default_x_tiles = vga.core.num_x_tiles;

        // 1920 exceeds the built-in 1600 cap, forcing a cap raise + tile regrow.
        vga.set_preferred_mode(1920, 1080, 32);

        assert!(vga.ext.vbe.max_xres >= 1920);
        assert!(vga.ext.vbe.max_yres >= 1080);
        assert_eq!(vga.ext.vbe.xres, 1920);
        assert_eq!(vga.ext.vbe.yres, 1080);
        assert_eq!(vga.ext.vbe.bpp, 32);
        assert!(vga.core.num_x_tiles > default_x_tiles);
        assert_eq!(
            vga.core.num_x_tiles,
            ((vga.ext.vbe.max_xres as u32).div_ceil(VGA_X_TILESIZE)) as u16
        );
        assert_eq!(
            vga.core.vga_tile_updated.len(),
            vga.core.num_x_tiles as usize * vga.core.num_y_tiles as usize
        );

        // Reset re-defaults the device but must re-apply the preferred mode.
        vga.reset();
        assert!(vga.ext.vbe.max_xres >= 1920);
        assert_eq!(vga.ext.vbe.xres, 1920);
        assert_eq!(vga.ext.vbe.yres, 1080);
    }

    #[test]
    fn preferred_mode_never_lowers_default_caps() {
        let mut vga = card();
        let (dx, dy) = (vga.ext.vbe.max_xres, vga.ext.vbe.max_yres);

        // A small mode must not shrink the built-in capability ceiling.
        vga.set_preferred_mode(800, 600, 16);

        assert_eq!(vga.ext.vbe.max_xres, dx);
        assert_eq!(vga.ext.vbe.max_yres, dy);
        assert_eq!(vga.ext.vbe.xres, 800);
        assert_eq!(vga.ext.vbe.yres, 600);
    }

    #[test]
    fn vbe_io_ports_program_mode_and_lfb_update_returns_rgba_tile() {
        let mut vga = card();

        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 2);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 2);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_32);
        write_vbe(
            &mut vga,
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED,
        );

        write_vram(&mut vga, VgaWindow::Lfb, 0, &[0x11, 0x22, 0x33, 0x44]);

        let sink = draw(&mut vga);
        let dims = sink.dimensions.expect("a mode set announces its geometry");
        assert_eq!((dims.width, dims.height, dims.bits_per_pixel), (2, 2, 32));
        assert_eq!(&sink.tile_at(0, 0)[0..4], &[0x33, 0x22, 0x11, 0xff]);
    }

    #[test]
    fn vbe_4bpp_bank_write_offsets_legacy_vga_memory() {
        let mut vga = card();
        vga.core.seq_chain_four = true;
        vga.core.seq_odd_even_dis = true;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);

        write_vram(&mut vga, VgaWindow::Legacy, 0, &[0x5a]);

        assert_eq!(vga.core.vga_memory[0], 0);
        assert_eq!(vga.core.vga_memory[0x10000], 0x5a);
    }

    #[test]
    fn vbe_4bpp_chain_four_bank_write_updates_vbe_backing() {
        let mut vga = card();
        vga.core.seq_chain_four = true;
        vga.core.seq_odd_even_dis = true;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);

        write_vram(&mut vga, VgaWindow::Legacy, 0, &[0x5a]);

        assert_eq!(vga.core.vbe_memory[0x10000], 0x5a);
    }

    #[test]
    fn vbe_4bpp_planar_bank_write_is_addressable() {
        let mut vga = card();
        vga.core.seq_odd_even_dis = true;
        vga.core.seq_regs[SEQ_REG_MAP_MASK] = 0x01;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;
        vga.core.graphics_regs[GFX_REG_BIT_MASK] = 0xff;
        vga.core.graphics_regs[GFX_REG_READ_MAP_SELECT] = 0;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);

        write_vram(&mut vga, VgaWindow::Legacy, 0, &[0x3c]);
        let mut byte = [0];
        read_vram(&mut vga, VgaWindow::Legacy, 0, &mut byte);

        assert_eq!(byte[0], 0x3c);
    }

    #[test]
    fn vbe_4bpp_read_bank_is_independent_from_write_bank() {
        let mut vga = card();
        vga.core.seq_chain_four = true;
        vga.core.seq_odd_even_dis = true;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, VBE_DISPI_BANK_WR | 1);
        write_vram(&mut vga, VgaWindow::Legacy, 0, &[0x7b]);

        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, VBE_DISPI_BANK_WR);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, VBE_DISPI_BANK_RD | 1);

        let mut byte = [0];
        read_vram(&mut vga, VgaWindow::Legacy, 0, &mut byte);

        assert_eq!(byte[0], 0x7b);
    }

    #[test]
    fn disabling_vbe_clears_legacy_bank_offset() {
        let mut vga = card();
        vga.core.seq_chain_four = true;
        vga.core.seq_odd_even_dis = true;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);

        write_vram(&mut vga, VgaWindow::Legacy, 0, &[0xa5]);

        assert_eq!(vga.core.vga_memory[0], 0xa5);
    }

    #[test]
    fn vbe_8bpp_dac_change_redraws_existing_tile() {
        let mut vga = card();

        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_8);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        vga.core.pel_data[5] = [0x3f, 0, 0];
        write_vram(&mut vga, VgaWindow::Lfb, 0, &[5]);

        let first = draw(&mut vga);
        assert_eq!(&first.tile_at(0, 0)[0..4], &[0xfc, 0x00, 0x00, 0xff]);

        vga.core.write_port(VGA_PEL_ADDR_WRITE, 5, 1);
        vga.core.write_port(VGA_PEL_DATA, 0, 1);
        vga.core.write_port(VGA_PEL_DATA, 0x3f, 1);
        vga.core.write_port(VGA_PEL_DATA, 0, 1);

        let second = draw(&mut vga);
        assert_eq!(&second.tile_at(0, 0)[0..4], &[0x00, 0xfc, 0x00, 0xff]);
        assert!(
            second.palette.iter().any(|&(index, _)| index == 5),
            "a completed DAC write reaches the front end as well as the pixels"
        );
    }

    #[test]
    fn an_eight_bit_dac_reaches_the_screen_and_the_front_end_unshifted() {
        let mut vga = card();

        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_8);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED | VBE_DISPI_8BIT_DAC);
        vga.core.write_port(VGA_PEL_ADDR_WRITE, 5, 1);
        vga.core.write_port(VGA_PEL_DATA, 0xf0, 1);
        vga.core.write_port(VGA_PEL_DATA, 0x00, 1);
        vga.core.write_port(VGA_PEL_DATA, 0x0c, 1);
        write_vram(&mut vga, VgaWindow::Lfb, 0, &[5]);

        let wide = draw(&mut vga);
        assert_eq!(&wide.tile_at(0, 0)[0..4], &[0xf0, 0x00, 0x0c, 0xff]);
        assert_eq!(
            wide.palette,
            [(5, Rgb { red: 0xf0, green: 0x00, blue: 0x0c })],
            "an 8-bit DAC entry is already host width"
        );

        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);

        let narrow = draw(&mut vga);
        assert_eq!(
            &narrow.tile_at(0, 0)[0..4],
            &[0xf0, 0x00, 0x0c, 0xff],
            "narrowing the DAC halves the entries and restores the shift, so the colour holds"
        );
    }

    #[test]
    fn legacy_chain_four_graphics_update_returns_palette_rgba_tile() {
        let mut vga = card();
        vga.core.vga_enabled = true;
        vga.core.video_enabled = true;
        vga.core.seq_regs[SEQ_REG_RESET] = 0x03;
        vga.core.seq_regs[SEQ_REG_MEMORY_MODE] = 0x0e;
        vga.core.seq_chain_four = true;
        vga.core.seq_odd_even_dis = true;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;
        vga.core.graphics_regs[GFX_REG_GRAPHICS_MODE] = 2 << 5;
        vga.core.crtc_regs[CRTC_HORIZ_DISPLAY_END] = 0;
        vga.core.crtc_regs[CRTC_VERT_DISPLAY_END] = 0;
        vga.core.crtc_regs[CRTC_VERT_BLANK_START] = 0;
        vga.core.crtc_regs[CRTC_MODE_CONTROL] = 0x40;
        vga.core.pel_data[5] = [0x3f, 0, 0];

        write_vram(&mut vga, VgaWindow::Legacy, 0, &[5]);

        let sink = draw(&mut vga);
        assert_eq!(&sink.tile_at(0, 0)[0..4], &[0xfc, 0x00, 0x00, 0xff]);
    }
    #[test]
    fn legacy_graphics_register_change_redraws_without_memory_write() {
        let mut vga = card();
        vga.core.vga_enabled = true;
        vga.core.video_enabled = true;
        vga.core.seq_regs[SEQ_REG_RESET] = 0x03;
        vga.core.seq_regs[SEQ_REG_MEMORY_MODE] = 0x0e;
        vga.core.seq_chain_four = true;
        vga.core.seq_odd_even_dis = true;
        vga.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.core.attr_regs[0x10] |= 0x01;
        vga.core.graphics_regs[GFX_REG_GRAPHICS_MODE] = 2 << 5;
        vga.core.crtc_regs[CRTC_HORIZ_DISPLAY_END] = 0;
        vga.core.crtc_regs[CRTC_VERT_DISPLAY_END] = 0;
        vga.core.crtc_regs[CRTC_VERT_BLANK_START] = 0;
        vga.core.crtc_regs[CRTC_MODE_CONTROL] = 0x40;

        write_vram(&mut vga, VgaWindow::Legacy, 0, &[5]);
        assert!(!draw(&mut vga).tiles.is_empty(), "the write produces a frame");
        assert!(
            draw(&mut vga).tiles.is_empty(),
            "and nothing further until something changes again"
        );

        vga.core.write_port(VGA_CRTC_INDEX, CRTC_OFFSET as u32, 1);
        vga.core.write_port(VGA_CRTC_DATA, 1, 1);

        assert!(
            !draw(&mut vga).tiles.is_empty(),
            "a register change redraws with no memory write of its own"
        );
    }

    /// A framebuffer window mapped as host RAM produces no exits, so the card
    /// sees none of the guest's writes and its own tile bitmap stays clean. The
    /// hypervisor's page bitmap is then the only record of what changed, and a
    /// frame drawn from it must reach the front end anyway.
    ///
    /// This is the path that cannot be retrofitted: a refresh with nowhere to
    /// receive a bitmap would present a stale screen forever under WHP or KVM,
    /// with nothing in the device to indicate why.
    #[test]
    fn a_page_bitmap_selects_tiles_the_device_never_saw_written() {
        let mut vga = card();
        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 2);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 2);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_32);
        write_vbe(
            &mut vga,
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED,
        );
        // The mode set marks everything dirty; take that frame first so what
        // follows is only what the bitmap accounts for.
        let _mode_set = draw(&mut vga);

        // Write straight into video memory, exactly as a guest does through a
        // direct-mapped window: no device path runs, so no tile is marked.
        vga.core.vbe_memory[0..4].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        assert!(
            draw(&mut vga).tiles.is_empty(),
            "self-tracking cannot see a write the device never handled"
        );

        // The hypervisor reports page 0 dirty, which is where those bytes are.
        let bitmap = [1u64];
        let sink = draw_with(
            &mut vga,
            crate::display::card::Dirt::Pages {
                bitmap: &bitmap,
                page_size: 4096,
            },
        );
        assert_eq!(
            &sink.tile_at(0, 0)[0..4],
            &[0x33, 0x22, 0x11, 0xff],
            "the page bitmap must produce the tile self-tracking missed"
        );

        // A bitmap reporting nothing dirty produces nothing, so the union does
        // not simply redraw everything whenever a bitmap is supplied.
        let clean = [0u64];
        assert!(
            draw_with(
                &mut vga,
                crate::display::card::Dirt::Pages {
                    bitmap: &clean,
                    page_size: 4096,
                },
            )
            .tiles
            .is_empty(),
            "a clean bitmap draws nothing"
        );
    }

    #[test]
    fn vbe_virtual_offset_wraps_and_redraws() {
        let mut vga = card();

        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_32);
        write_vbe(
            &mut vga,
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED,
        );
        write_vbe(&mut vga, VBE_DISPI_INDEX_VIRT_WIDTH, 1600);
        write_vbe(&mut vga, VBE_DISPI_INDEX_Y_OFFSET, 2622);

        let wrapped_start = vga.ext.vbe.virtual_start as usize;
        assert!(wrapped_start < vga.core.vbe_memory.len());
        vga.core.vbe_memory[wrapped_start..wrapped_start + 4].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        let sink = draw(&mut vga);
        assert_eq!(&sink.tile_at(0, 0)[0..4], &[0x33, 0x22, 0x11, 0xff]);
    }

    // ---- Finding #5: write to 0x3CC (Misc Output *read* port) is ignored ----
    // Bochs vgacore.cc write: `case 0x03cc: /* Graphics 1 Position (EGA) */ // ignore`.
    // The real Misc Output write port is 0x3C2.
    #[test]
    fn misc_output_write_to_0x3cc_is_ignored() {
        let mut vga = VgaCore::new();

        // Program a known, distinctive Misc Output value via the real write
        // port (0x3C2). Keep color_emulation=1 so the color-mode ports stay
        // routable for the rest of the test.
        vga.write_port(VGA_MISC_OUTPUT_WRITE, 0xAB, 1);
        assert_eq!(vga.misc_output, 0xAB);
        assert!(vga.misc_color_emulation);

        // A write to 0x3CC must be a no-op (Bochs: "Graphics 1 Position (EGA)").
        vga.write_port(VGA_MISC_OUTPUT, 0x00, 1);

        assert_eq!(
            vga.misc_output, 0xAB,
            "0x3CC write must not alter Misc Output"
        );
        assert!(
            vga.misc_color_emulation,
            "0x3CC write must not flip color/mono emulation"
        );
        // Read path at 0x3CC is unaffected and still reflects the programmed value.
        assert_eq!(vga.read_port(VGA_MISC_OUTPUT, 1, clock_at(0)), 0xAB);
    }

    // ---- Finding #6a: Sequencer index is stored unmasked; out-of-range DATA
    // writes are no-ops (Bochs vgacore.cc write: `default:` case does nothing) ----
    #[test]
    fn sequencer_out_of_range_index_data_write_is_noop() {
        let mut vga = VgaCore::new();
        vga.seq_regs = [0x11, 0x22, 0x33, 0x44, 0x55];

        vga.write_port(VGA_SEQ_INDEX, 8, 1);
        assert_eq!(vga.seq_index, 8, "sequencer index must be stored unmasked");
        vga.write_port(VGA_SEQ_DATA, 0x00, 1);

        // Index 8 is out of range (valid: 0..=4); the write must be dropped,
        // not aliased onto index 0 (sequencer reset), which would have reset
        // the sequencer and cleared char map state.
        assert_eq!(vga.seq_regs, [0x11, 0x22, 0x33, 0x44, 0x55]);
    }

    // ---- Finding #6b: CRTC index 0x22 read-back returns the graphics latch,
    // not an aliased register (Bochs vgacore.cc read: `case 0x22`) ----
    #[test]
    fn crtc_index_0x22_reads_back_graphics_latch() {
        let mut vga = VgaCore::new();
        // Give CR2 (start horizontal blank) a sentinel value distinct from the
        // latch. With the old `& 0x1F` masking, index 0x22 aliased onto CR2.
        vga.crtc_regs[CRTC_START_HORIZ_BLANK] = 0xAB;
        vga.latch = [0x11, 0x22, 0x33, 0x44];
        vga.graphics_regs[GFX_REG_READ_MAP_SELECT] = 2;

        vga.write_port(VGA_CRTC_INDEX, 0x22, 1);
        assert_eq!(
            vga.crtc_index, 0x22,
            "CRTC index must be masked with 0x3F, not 0x1F"
        );

        let data = vga.read_port(VGA_CRTC_DATA, 1, clock_at(0));
        assert_eq!(
            data, 0x33,
            "0x3D5 must read back latch[read_map_select], not CR2"
        );
    }

    // ---- Finding #6c: Graphics Controller index is stored unmasked; out-of-range
    // DATA writes are no-ops (Bochs vgacore.cc write: `default:` case does nothing) ----
    #[test]
    fn graphics_out_of_range_index_data_write_is_noop() {
        let mut vga = VgaCore::new();
        vga.graphics_regs = [1, 2, 3, 4, 5, 6, 7, 8, 9];

        vga.write_port(VGA_GRAPHICS_INDEX, 0x20, 1);
        assert_eq!(
            vga.graphics_index, 0x20,
            "graphics index must be stored unmasked"
        );
        vga.write_port(VGA_GRAPHICS_DATA, 0xFF, 1);

        assert_eq!(vga.graphics_regs, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn word_read_of_crtc_returns_index_and_data() {
        // Bochs vgacore.cc read: a 16-bit access combines two byte reads,
        // low | high<<8 — inw(0x3D4) → index | data<<8.
        let mut vga = VgaCore::new();
        vga.write_port(VGA_CRTC_INDEX, CRTC_OVERFLOW as u32, 1);
        vga.crtc_regs[CRTC_OVERFLOW] = 0x5A;
        let word = vga.read_port(VGA_CRTC_INDEX, 2, clock_at(0));
        assert_eq!(word & 0xFF, CRTC_OVERFLOW as u32, "low byte = index reg");
        assert_eq!((word >> 8) & 0xFF, 0x5A, "high byte = data reg");
    }

    // Bochs vgacore.cc write cases 0x03ba/0x03da: `feature_control = value & 0x08`;
    // read case 0x03ca returns it; read case 0x03db returns 0.
    #[test]
    fn feature_control_round_trips_via_3da_and_3ca() {
        let mut vga = VgaCore::new();
        assert_eq!(vga.read_port(0x3CA, 1, clock_at(0)), 0x00, "reset value is 0");

        vga.write_port(VGA_STATUS, 0xFF, 1);
        assert_eq!(
            vga.read_port(0x3CA, 1, clock_at(0)),
            0x08,
            "only bit 3 of a 0x3DA write is retained"
        );

        // The 0x3BA alias is gated off while in color emulation — Bochs
        // vgacore.cc write_handler returns early for 0x3b0-0x3bf when
        // misc_output.color_emulation is set — so it must NOT clear the value.
        vga.write_port(VGA_STATUS_MONO, 0x00, 1);
        assert_eq!(
            vga.read_port(0x3CA, 1, clock_at(0)),
            0x08,
            "mono-port write must be ignored in color emulation mode"
        );

        // Writing 0 through the active (color) port does clear it.
        vga.write_port(VGA_STATUS, 0x00, 1);
        assert_eq!(vga.read_port(0x3CA, 1, clock_at(0)), 0x00);

        // 0x3DB is the high byte of a 16-bit read from 0x3DA: Bochs returns 0.
        assert_eq!(vga.read_port(0x3DB, 1, clock_at(0)), 0x00);
    }

    // Bochs vgacore.cc keeps sequencer registers as decomposed fields, so a
    // read-back only exposes the retained bits (write case 0x03c5 / read case
    // 0x03c5). Reset register 0's falling edge also clears char-map select.
    #[test]
    fn sequencer_registers_mask_on_store_like_bochs() {
        let mut vga = VgaCore::new();
        let write_seq = |vga: &mut VgaCore, index: u32, value: u32| {
            vga.write_port(VGA_SEQ_INDEX, index, 1);
            vga.write_port(VGA_SEQ_DATA, value, 1);
        };

        // Reg 0 (reset): only reset1|reset2 survive.
        write_seq(&mut vga, 0, 0xFF);
        assert_eq!(vga.seq_regs[SEQ_REG_RESET], 0x03);
        // Reg 1 (clocking mode): value & 0x3D.
        write_seq(&mut vga, 1, 0xFF);
        assert_eq!(vga.seq_regs[SEQ_REG_CLOCKING_MODE], 0x3D);
        // Reg 2 (map mask): 4 plane bits.
        write_seq(&mut vga, 2, 0xFF);
        assert_eq!(vga.seq_regs[SEQ_REG_MAP_MASK], 0x0F);
        // Reg 3 (char map select): 6 bits.
        write_seq(&mut vga, 3, 0xFF);
        assert_eq!(vga.seq_regs[SEQ_REG_CHAR_MAP_SELECT], 0x3F);
        // Reg 4 (memory mode): only extended_mem/odd_even_dis/chain_four.
        write_seq(&mut vga, 4, 0xFF);
        assert_eq!(vga.seq_regs[SEQ_REG_MEMORY_MODE], 0x0E);
        assert!(vga.seq_chain_four);

        // Reset1 falling edge (bit 0: 1 -> 0) clears char-map select.
        assert_ne!(vga.seq_regs[SEQ_REG_CHAR_MAP_SELECT], 0);
        write_seq(&mut vga, 0, 0x00);
        assert_eq!(
            vga.seq_regs[SEQ_REG_CHAR_MAP_SELECT], 0,
            "reset1 falling edge resets the character map selection"
        );
    }

    // Bochs vgacore.cc update_charmap(): glyph bytes live in plane 2, so byte i
    // of a map is memory[(address << 2) + i*4 + 2]. Sequencer register 3 picks
    // the two map offsets through charmap_offset[], gated on CRTC 9 being > 0.
    #[test]
    fn guest_charmap_extracts_plane_two_like_bochs() {
        let mut vga = card();

        // Two distinct glyph patterns at the plane-2 bytes for offsets
        // 0x0000 (map index 0) and 0x4000 (map index 1).
        vga.core.vga_memory[2] = 0xA5; // byte 0 of the map at address 0x0000
        vga.core.vga_memory[6] = 0x3C; // byte 1 of the same map
        vga.core.vga_memory[(0x4000usize << 2) + 2] = 0x5A; // byte 0 of the map at 0x4000

        // A non-zero maximum-scan-line is required before Bochs applies the
        // selection, so program CRTC 9 first.
        vga.core.write_port(VGA_CRTC_INDEX, CRTC_MAX_SCAN_LINE as u32, 1);
        vga.core.write_port(VGA_CRTC_DATA, 0x0F, 1);

        // Sequencer register 3 = 0 selects charmap A = B = offset 0x0000.
        vga.core.write_port(VGA_SEQ_INDEX, 3, 1);
        vga.core.write_port(VGA_SEQ_DATA, 0x00, 1);
        assert_eq!(vga.core.charmap_address1, 0x0000);
        assert_eq!(vga.core.charmap_address2, 0x0000);
        assert_ne!(vga.core.vga_mem_updated & VGA_MEM_UPDATED_CHARMAP, 0);

        // The frame carries both generators to the front end. Every path that
        // presents a frame goes through this one refresh, so a guest that
        // reprograms its font cannot render with stale glyphs on one front end
        // and current ones on another — which is exactly what happened while
        // the shared-framebuffer path kept its own copy of the forwarding.
        let sink = draw(&mut vga);
        assert_eq!(
            sink.charmaps,
            alloc::vec![0, 1],
            "a dirty character generator reaches the display, both maps"
        );

        assert_eq!(vga.core.charmap(0)[0], 0xA5, "plane-2 byte 0");
        assert_eq!(vga.core.charmap(0)[1], 0x3C, "plane-2 byte 1 (stride 4)");
        assert_eq!(
            vga.core.charmap(1)[0],
            0xA5,
            "equal addresses must publish the same glyphs to both maps"
        );

        // Register 3 = 0x04 selects map B = index 1 -> offset 0x4000.
        vga.core.write_port(VGA_SEQ_INDEX, 3, 1);
        vga.core.write_port(VGA_SEQ_DATA, 0x04, 1);
        assert_eq!(vga.core.charmap_address1, 0x0000);
        assert_eq!(vga.core.charmap_address2, 0x4000);
        vga.core.update_charmap();
        assert_eq!(vga.core.charmap(0)[0], 0xA5, "map 0 unchanged");
        assert_eq!(vga.core.charmap(1)[0], 0x5A, "map 1 now reads the 0x4000 glyphs");

        // A sequencer reset (reset1 falling edge) clears the selection.
        vga.core.write_port(VGA_SEQ_INDEX, 0, 1);
        vga.core.write_port(VGA_SEQ_DATA, 0x03, 1);
        vga.core.write_port(VGA_SEQ_DATA, 0x00, 1);
        assert_eq!(vga.core.charmap_address1, 0);
        assert_eq!(vga.core.charmap_address2, 0);
    }

    // Bochs vgacore.cc write case 0x03c0 data-write mode: per-register bit masks.
    #[test]
    fn attribute_registers_mask_on_store_like_bochs() {
        let mut vga = VgaCore::new();
        let write_attr = |vga: &mut VgaCore, index: u32, value: u32| {
            // Address phase (flip-flop clear), then data phase.
            vga.attr_flip_flop = false;
            vga.write_port(VGA_ATTRIB_ADDR, index, 1);
            vga.write_port(VGA_ATTRIB_ADDR, value, 1);
        };

        // 0x11 overscan color: 6 bits.
        write_attr(&mut vga, 0x11, 0xFF);
        assert_eq!(vga.attr_regs[0x11], 0x3F);
        // 0x12 color plane enable / 0x13 pel panning / 0x14 color select: 4 bits.
        write_attr(&mut vga, 0x12, 0xFF);
        assert_eq!(vga.attr_regs[0x12], 0x0F);
        write_attr(&mut vga, 0x13, 0xFF);
        assert_eq!(vga.attr_regs[0x13], 0x0F);
        write_attr(&mut vga, 0x14, 0xFF);
        assert_eq!(vga.attr_regs[0x14], 0x0F);
        // Palette registers keep all 8 bits (Bochs stores value unmasked).
        write_attr(&mut vga, 0x05, 0xFF);
        assert_eq!(vga.attr_regs[0x05], 0xFF);
    }

    // Bochs vgacore.cc CRTC write case 0x09: y_doublescan = ((value & 0x9f) > 0).
    #[test]
    fn crtc_max_scan_line_derives_y_doublescan() {
        let mut vga = VgaCore::new();
        vga.write_port(VGA_CRTC_INDEX, CRTC_MAX_SCAN_LINE as u32, 1);

        // 0x00 -> no doubling.
        vga.write_port(VGA_CRTC_DATA, 0x00, 1);
        assert!(!vga.y_doublescan);

        // Mode 13h programs 0x41 (max scan line 1 + line-compare bit 9): doubled.
        vga.write_port(VGA_CRTC_DATA, 0x41, 1);
        assert!(vga.y_doublescan);

        // Only bits in 0x9F count — 0x40 alone (line compare bit 9) does not.
        vga.write_port(VGA_CRTC_DATA, 0x40, 1);
        assert!(!vga.y_doublescan);

        // Bit 7 (0x80) is inside the mask.
        vga.write_port(VGA_CRTC_DATA, 0x80, 1);
        assert!(vga.y_doublescan);
    }

    #[test]
    fn write_to_0x3c1_ignored_and_0x3c2_reads_zero() {
        // Bochs vgacore.cc: 0x3C1 (Attribute Data READ port) ignores writes;
        // 0x3C2 read (Input Status 0) returns 0, not 0xFF.
        let mut vga = VgaCore::new();
        vga.attr_index = 5;
        vga.attr_regs[5] = 0x11;
        vga.write_port(VGA_ATTRIB_DATA, 0xFF, 1);
        assert_eq!(vga.attr_regs[5], 0x11, "0x3C1 write must not modify attr regs");
        assert_eq!(vga.read_port(VGA_MISC_OUTPUT_WRITE, 1, clock_at(0)), 0x00, "0x3C2 read = 0");
    }

    // ---- Finding #7: CR11 bit 7 write-protects CRTC registers 0-7 ----
    // Bochs vgacore.cc write: when `CRTC.reg[0x11] & 0x80` is set, writes to
    // CRTC indices 0x00-0x06 are dropped and a write to 0x07 updates only bit 4.
    #[test]
    fn crtc_write_protect_locks_registers_0_to_7() {
        let mut vga = VgaCore::new();

        // Seed CR0..CR7 with distinct sentinel values while unprotected.
        for index in 0u32..=7 {
            vga.write_port(VGA_CRTC_INDEX, index, 1);
            let sentinel = 0x05 + index as u8; // CR7 sentinel (0x0C) has bit4 clear
            vga.write_port(VGA_CRTC_DATA, sentinel as u32, 1);
        }
        let seeded = vga.crtc_regs;
        assert_eq!(seeded[0x07], 0x0C);

        // Engage write protection via CR11 bit 7.
        vga.write_port(VGA_CRTC_INDEX, CRTC_VERT_RETRACE_END as u32, 1);
        vga.write_port(VGA_CRTC_DATA, 0x80, 1);
        assert_eq!(vga.crtc_regs[CRTC_VERT_RETRACE_END], 0x80);

        // Attempt to overwrite CR0..CR6: must be dropped entirely.
        for index in 0u32..=6 {
            vga.write_port(VGA_CRTC_INDEX, index, 1);
            vga.write_port(VGA_CRTC_DATA, 0xFF, 1);
            assert_eq!(
                vga.crtc_regs[index as usize], seeded[index as usize],
                "CR{index} must be unchanged while write-protected"
            );
        }

        // CR7 write while protected: only bit 4 (line-compare bit 8) may change.
        vga.write_port(VGA_CRTC_INDEX, CRTC_OVERFLOW as u32, 1);
        vga.write_port(VGA_CRTC_DATA, 0xFF, 1);
        assert_eq!(
            vga.crtc_regs[CRTC_OVERFLOW],
            seeded[CRTC_OVERFLOW] | 0x10,
            "CR7 write while protected must set only bit 4"
        );

        vga.write_port(VGA_CRTC_INDEX, CRTC_OVERFLOW as u32, 1);
        vga.write_port(VGA_CRTC_DATA, 0x00, 1);
        assert_eq!(
            vga.crtc_regs[CRTC_OVERFLOW],
            seeded[CRTC_OVERFLOW] & !0x10,
            "CR7 write while protected must clear only bit 4"
        );

        // Disengage write protection (CR11 is never itself protected).
        vga.write_port(VGA_CRTC_INDEX, CRTC_VERT_RETRACE_END as u32, 1);
        vga.write_port(VGA_CRTC_DATA, 0x00, 1);
        assert_eq!(vga.crtc_regs[CRTC_VERT_RETRACE_END], 0x00);

        // Writes to CR0..CR6 now go through normally again.
        vga.write_port(VGA_CRTC_INDEX, 0, 1);
        vga.write_port(VGA_CRTC_DATA, 0x99, 1);
        assert_eq!(vga.crtc_regs[0], 0x99);
    }
    #[cfg(feature = "std")]
    #[test]
    fn vga_snapshot_restores_planar_vbe_palette_and_forces_redraw() {
        let mut source = pci_vga();
        source.pci_write(0x10, 0xE800_0000, 4);
        source.pci_write(0x18, 0xF010_0000, 4);
        write_vbe(&mut source, VBE_DISPI_INDEX_XRES, 320);
        write_vbe(&mut source, VBE_DISPI_INDEX_YRES, 200);
        write_vbe(&mut source, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_8);
        write_vbe(
            &mut source,
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED | VBE_DISPI_NOCLEARMEM,
        );
        write_vbe(&mut source, VBE_DISPI_INDEX_VIRT_WIDTH, 640);
        write_vbe(&mut source, VBE_DISPI_INDEX_X_OFFSET, 7);
        write_vbe(&mut source, VBE_DISPI_INDEX_Y_OFFSET, 9);

        source.core.seq_regs[SEQ_REG_MEMORY_MODE] = 0x04;
        source.core.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        source.core.graphics_regs[GFX_REG_READ_MAP_SELECT] = 2;
        source.core.vga_memory[0x24 * 4 + 2] = 0xA1;
        source.core.text_memory[0x42] = b'V';
        source.core.vbe_memory[0x1234] = 0xB2;
        source.core.latch = [0x11, 0x22, 0x33, 0x44];
        source.core.status_reg = 0xA0;
        source.core.write_port(VGA_PEL_ADDR_WRITE, 0x4D, 1);
        source.core.write_port(VGA_PEL_DATA, 0x12, 1);
        source.core.write_port(VGA_PEL_DATA, 0x23, 1);
        source.core.write_port(VGA_PEL_DATA, 0x34, 1);

        let mut saved = Vec::new();
        source.save(&mut saved).unwrap();
        assert_eq!(source.snapshot_len().unwrap(), saved.len() as u64);

        let mut restored = pci_vga();
        restored.pci_write(0x10, 0xD000_0000, 4);
        restored.commit_pending_lfb_relocate();
        restored.pci_write(0x18, 0xF100_0000, 4);
        restored.commit_pending_mmio_relocate();
        let live_mapping = restored.snapshot_v3_committed_mapping_target();
        restored.core.vga_memory[0x24 * 4 + 2] = 0;
        restored.core.text_memory[0x42] = 0;
        restored.core.vbe_memory[0x1234] = 0;
        restored.core.latch = [0; 4];
        restored.core.status_reg = 0;
        restored.core.text_dirty = false;
        restored.core.text_buffer_update = false;
        restored.core.vga_mem_updated = 0;
        restored.core.text_buffer.fill(0xFF);
        restored.core.text_snapshot.fill(0xFF);
        restored.core.last_xres = 1;
        restored.core.last_yres = 1;
        restored.core.last_fw = 1;
        restored.core.last_fh = 1;
        restored.core.last_bpp = 1;
        restored.core.vga_tile_updated.fill(false);

        let mut reader: &[u8] = saved.as_slice();
        let target = restored.restore(&mut reader).unwrap();
        assert!(
            reader.is_empty(),
            "the card consumed its whole section, exactly — the container asserts \
             this too, but only a device knows what its own body should span"
        );

        assert_eq!(
            restored.snapshot_v3_committed_mapping_target(),
            live_mapping,
            "decode must leave the live handler mapping untouched"
        );
        assert_eq!(
            target,
            VgaSnapshotRestoreTarget {
                lfb_base: 0xE800_0000,
                mmio_base: 0xF010_0000,
            }
        );
        restored.commit_snapshot_v3_mapping_target(target);
        restored.rebuild_snapshot_v3_derived_state().unwrap();

        assert_eq!(restored.snapshot_v3_committed_mapping_target(), target);
        assert!(restored.core.text_dirty);
        assert!(restored.core.text_buffer_update);
        assert_eq!(restored.core.vga_mem_updated, 1);
        assert!(restored.core.vga_tile_updated.iter().all(|dirty| *dirty));
        assert!(restored.core.text_buffer.iter().all(|byte| *byte == 0));
        assert!(restored.core.text_snapshot.iter().all(|byte| *byte == 0));
        assert_eq!(
            (
                restored.core.last_xres,
                restored.core.last_yres,
                restored.core.last_fw,
                restored.core.last_fh,
                restored.core.last_bpp,
            ),
            (0, 0, 0, 0, 0)
        );

        let mut value = [0];
        read_vram(&mut restored, VgaWindow::Lfb, 0x1234, &mut value);
        assert_eq!(value, [0xB2], "VBE backing memory must survive restore");

        restored.core.write_port(VGA_DAC_STATE, 0x4D, 1);
        assert_eq!(restored.core.read_port(VGA_PEL_DATA, 1, clock_at(0)), 0x12);
        assert_eq!(restored.core.read_port(VGA_PEL_DATA, 1, clock_at(0)), 0x23);
        assert_eq!(restored.core.read_port(VGA_PEL_DATA, 1, clock_at(0)), 0x34);
        restored.core.write_port(VGA_CRTC_INDEX, 0x22, 1);
        assert_eq!(restored.core.read_port(VGA_CRTC_DATA, 1, clock_at(0)), 0x33);
        assert_eq!(restored.core.read_port(VGA_STATUS, 1, clock_at(0)), 0xA9);

        write_vbe(&mut restored, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
        read_vram(&mut restored, VgaWindow::Legacy, 0x24, &mut value);
        assert_eq!(value, [0xA1], "planar memory must survive restore");
        assert_eq!(restored.core.get_text_memory()[0x42], b'V');
    }

    #[cfg(feature = "std")]
    #[test]
    fn a_restored_eight_bit_dac_keeps_its_width_and_republishes_every_entry() {
        let mut source = pci_vga();
        source.pci_write(0x10, 0xE800_0000, 4);
        source.pci_write(0x18, 0xF010_0000, 4);
        write_vbe(&mut source, VBE_DISPI_INDEX_XRES, 320);
        write_vbe(&mut source, VBE_DISPI_INDEX_YRES, 200);
        write_vbe(&mut source, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_8);
        write_vbe(
            &mut source,
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED | VBE_DISPI_8BIT_DAC,
        );
        source.core.write_port(VGA_PEL_ADDR_WRITE, 5, 1);
        source.core.write_port(VGA_PEL_DATA, 0xf0, 1);
        source.core.write_port(VGA_PEL_DATA, 0x00, 1);
        source.core.write_port(VGA_PEL_DATA, 0x0c, 1);
        write_vram(&mut source, VgaWindow::Lfb, 0, &[5]);

        let mut saved = Vec::new();
        source.save(&mut saved).unwrap();

        let mut restored = pci_vga();
        let mut reader: &[u8] = saved.as_slice();
        let target = restored.restore(&mut reader).unwrap();
        restored.commit_snapshot_v3_mapping_target(target);
        restored.rebuild_snapshot_v3_derived_state().unwrap();

        let sink = draw(&mut restored);
        assert_eq!(
            &sink.tile_at(0, 0)[0..4],
            &[0xf0, 0x00, 0x0c, 0xff],
            "the restored DAC keeps the width the guest chose"
        );
        assert_eq!(
            sink.palette.len(),
            PEL_COLOR_COUNT,
            "a restore republishes the whole DAC table, as Bochs after_restore_state does"
        );
        assert!(
            sink.palette.contains(&(5, Rgb { red: 0xf0, green: 0x00, blue: 0x0c })),
            "republished entries carry the restored width"
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn vga_snapshot_rejects_oversized_pci_config_length() {
        let source = card();
        let mut saved = Vec::new();
        source.save(&mut saved).unwrap();
        // The PCI config length immediately follows the fixed 84-byte scalar prefix.
        saved[88..92].copy_from_slice(&257u32.to_le_bytes());

        let mut restored = card();
        let mut reader: &[u8] = saved.as_slice();
        let error = restored.restore(&mut reader).unwrap_err();
        assert!(
            matches!(error, SnapError::Invalid(_)),
            "a rejected device state names what was wrong: {error:?}"
        );
    }
}

/// The VGA apertures as a memory-mapped device.
///
/// Bochs vgacore.cc registers `mem_read`/`mem_write` for the legacy window and
/// the LFB; the return value reports whether the access was claimed, which the
/// dispatcher has never consulted — an address only reaches here because the
/// map already decided it belongs to this device.
/// The VGA's port block — Bochs vgacore.cc `read_handler`/`write_handler`.
///
/// A port access neither raises an interrupt nor arms a timer here: the
/// vertical-retrace timer is machine-owned and re-armed at the scheduler
/// boundary from the CRTC timing, so the only capability this device takes
/// from its context is the clock the retrace phase is measured against.
impl crate::api::PioDevice for VgaCore {
    fn pio_read(
        &mut self,
        port: u16,
        len: crate::api::IoLen,
        ctx: &mut crate::api::DeviceCtx<'_>,
    ) -> u32 {
        self.read_port(port, len.bytes(), ctx.clock)
    }

    fn pio_write(
        &mut self,
        port: u16,
        value: u32,
        len: crate::api::IoLen,
        _ctx: &mut crate::api::DeviceCtx<'_>,
    ) {
        self.write_port(port, value, len.bytes());
    }
}

/// Reading the VGA's screen.
///
/// Bochs's `get_text_snapshot` hands the GUI the live text plane and the same
/// grid the renderer used; this reports the same thing one cell at a time so a
/// no-alloc caller can read a screen without a buffer.
impl crate::display::DisplaySource for VgaCore {
    fn resolution(&self) -> crate::display::Resolution {
        if let Some(geometry) = self.text_geometry() {
            return crate::display::Resolution::new(
                geometry.pixel_width,
                geometry.pixel_height,
            );
        }
        let (width, height) = self.determine_screen_dimensions();
        crate::display::Resolution::new(width, height)
    }

    fn text_grid(&self) -> Option<crate::display::TextGrid> {
        let geometry = self.text_geometry()?;
        let cursor = if geometry.cursor_address == VgaTextGeometry::CURSOR_OFF
            || geometry.line_offset == 0
        {
            None
        } else {
            let from_start = geometry.cursor_address.saturating_sub(geometry.start_address);
            Some(crate::display::TextPos::new(
                usize::from(from_start / geometry.line_offset),
                usize::from((from_start % geometry.line_offset) / BYTES_PER_CHAR as u16),
            ))
        };
        Some(crate::display::TextGrid {
            rows: geometry.rows,
            cols: geometry.cols,
            cursor,
        })
    }

    fn text_char(&self, pos: crate::display::TextPos) -> char {
        let Some(geometry) = self.text_geometry() else {
            return ' ';
        };
        if pos.row >= geometry.rows || pos.col >= geometry.cols {
            return ' ';
        }
        self.text_char_at(&geometry, pos.row, pos.col)
    }
}

/// The v3 snapshot section describes the CARD, so it binds to this one.
/// Bochs registers state per class — `bx_geforce_c::register_state` writes
/// an entirely different set — and the stream interleaves core and VBE
/// fields, so no generic per-half hook could reproduce the layout.
impl VgaCard<StdVga> {
    /// Rebuilds parsed VGA state and invalidates all GUI-facing caches after a
    /// successful whole-machine snapshot restore.
    #[cfg(feature = "std")]
    pub fn rebuild_snapshot_v3_derived_state(&mut self) -> SnapResult<()> {
        self.ext.validate_vbe_snapshot_state(&VgaSnapshotVbeState::from(&self.ext.vbe))?;
        self.validate_snapshot_v3_cache_topology()?;

        self.core.misc_color_emulation = (self.core.misc_output & MISC_OUT_COLOR_EMULATION) != 0;
        self.core.misc_enable_ram = (self.core.misc_output & MISC_OUT_ENABLE_RAM) != 0;
        self.core.misc_clock_select =
            (self.core.misc_output >> MISC_OUT_CLOCK_SEL_SHIFT) & MISC_OUT_CLOCK_SEL_MASK;
        self.core.misc_select_high_bank = (self.core.misc_output & MISC_OUT_HIGH_BANK) != 0;
        self.core.misc_horiz_sync_pol = (self.core.misc_output & MISC_OUT_HORIZ_POL) != 0;
        self.core.misc_vert_sync_pol = (self.core.misc_output & MISC_OUT_VERT_POL) != 0;
        self.core.seq_chain_four = (self.core.seq_regs[SEQ_REG_MEMORY_MODE] & 0x08) != 0;
        self.core.seq_odd_even_dis = (self.core.seq_regs[SEQ_REG_MEMORY_MODE] & 0x04) != 0;

        let (bpp_multiplier, line_offset) = vga_snapshot_vbe_layout(
            self.ext.vbe.bpp,
            self.ext.vbe.virtual_xres,
        )?;
        self.ext.vbe.bpp_multiplier = bpp_multiplier;
        self.ext.vbe.line_offset = line_offset;
        self.ext.vbe.visible_screen_size = u32::from(line_offset)
            .checked_mul(u32::from(self.ext.vbe.yres))
            .ok_or_else(|| invalid_vga_snapshot("VBE visible-screen size overflows"))?;
        self.core.vga_mem_mask = if self.ext.vbe.enabled == VBE_DISPI_ENABLED {
            self.ext.vbe_memsize
                .checked_sub(1)
                .ok_or_else(|| invalid_vga_snapshot("VBE memory size is zero"))?
        } else {
            u32::try_from(VGA_MEM_SIZE - 1)
                .map_err(|_| invalid_vga_snapshot("legacy VGA memory mask does not fit u32"))?
        };
        self.core.ext_offset = vga_snapshot_bank_offset(
            self.ext.vbe.bank[0],
            self.ext.vbe.bank_granularity_kb,
        )?;
        self.core.ext_read_offset = vga_snapshot_bank_offset(
            self.ext.vbe.bank[1],
            self.ext.vbe.bank_granularity_kb,
        )?;
        self.ext.recompute_vbe_virtual_start(&mut self.core);
        self.core.calculate_retrace_timing();

        // Bochs vgacore.cc after_restore_state republishes every DAC entry
        // `<< dac_shift`. The snapshot carries the DAC width the shift derives
        // from, and a marked entry reaches the front end in the next frame's
        // preamble.
        self.core.dac_shift = dac_shift_for(self.ext.vbe.dac_8bit);
        self.core.dac_dirty.fill(true);
        self.core.dac_any_dirty = true;

        let cursor_addr = (usize::from(self.core.crtc_regs[CRTC_CURSOR_LOC_HIGH]) << 8)
            | usize::from(self.core.crtc_regs[CRTC_CURSOR_LOC_LOW]);
        self.core.cursor_pos = (
            cursor_addr / BYTES_PER_ROW,
            (cursor_addr % BYTES_PER_ROW) / BYTES_PER_CHAR,
        );

        self.core.text_buffer.fill(0);
        self.core.text_snapshot.fill(0);
        self.core.text_dirty = true;
        self.core.text_buffer_update = true;
        self.core.vga_mem_updated = 1;
        self.core.last_xres = 0;
        self.core.last_yres = 0;
        self.core.last_fw = 0;
        self.core.last_fh = 0;
        self.core.last_bpp = 0;
        self.core.vga_tile_updated.fill(true);
        Ok(())
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_source(&self) -> SnapResult<()> {
        self.ext.validate_vbe_snapshot_state(&VgaSnapshotVbeState::from(&self.ext.vbe))?;
        self.validate_snapshot_v3_cache_topology()?;
        if snapshot_v3_usize_len(self.core.vbe_memory.len())?
            != u64::from(self.ext.vbe_memsize)
        {
            return Err(invalid_vga_snapshot(
                "live VBE backing storage does not match configured size",
            ));
        }
        if self.core.pel_data.len() > MAX_COUNT {
            return Err(invalid_vga_snapshot("VGA palette exceeds snapshot count bound"));
        }
        Ok(())
    }

    #[cfg(feature = "std")]
    #[allow(clippy::too_many_arguments)]
    fn validate_snapshot_v3_scalars(
        &self,
        crtc_index: u8,
        attr_index: u8,
        dac_state: u8,
        pel_write_cycle: u8,
        pel_read_cycle: u8,
        has_icount_sync: bool,
        ips: u64,
        vbe_memsize: u32,
        preferred_mode: Option<(u16, u16, u16)>,
        vbe: &VgaSnapshotVbeState,
        pci_enabled: bool,
        target: VgaSnapshotRestoreTarget,
    ) -> SnapResult<()> {
        if crtc_index > CRTC_INDEX_MASK {
            return Err(invalid_vga_snapshot("VGA CRTC index is out of range"));
        }
        if attr_index > ATTR_INDEX_MASK {
            return Err(invalid_vga_snapshot("VGA attribute index is out of range"));
        }
        if !matches!(
            dac_state,
            DAC_STATE_WRITE_MODE | DAC_STATE_READ_MODE | 0x01
        ) {
            return Err(invalid_vga_snapshot("VGA DAC state is invalid"));
        }
        if pel_write_cycle >= PEL_CYCLES_PER_COLOR || pel_read_cycle >= PEL_CYCLES_PER_COLOR {
            return Err(invalid_vga_snapshot("VGA DAC cycle is out of range"));
        }
        if has_icount_sync != self.core.has_icount_sync || ips != self.core.ips {
            return Err(invalid_vga_snapshot("VGA retrace clock configuration mismatch"));
        }
        if vbe_memsize != self.ext.vbe_memsize {
            return Err(invalid_vga_snapshot("VBE memory configuration mismatch"));
        }
        if preferred_mode != self.core.preferred_mode {
            return Err(invalid_vga_snapshot("VBE preferred-mode configuration mismatch"));
        }
        if pci_enabled != self.ext.pci_enabled {
            return Err(invalid_vga_snapshot("VGA PCI enable configuration mismatch"));
        }
        self.ext.validate_vbe_snapshot_state(vbe)?;
        validate_vga_snapshot_bar_base(target.lfb_base, self.ext.vbe_memsize)?;
        validate_vga_snapshot_bar_base(target.mmio_base, PCI_VGA_MMIO_SIZE)
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_cache_topology(&self) -> SnapResult<()> {
        let expected_x_tiles = (u32::from(self.ext.vbe.max_xres)
            .checked_add(VGA_X_TILESIZE - 1)
            .ok_or_else(|| invalid_vga_snapshot("VGA horizontal tile count overflows"))?)
            / VGA_X_TILESIZE;
        let expected_y_tiles = (u32::from(self.ext.vbe.max_yres)
            .checked_add(VGA_Y_TILESIZE - 1)
            .ok_or_else(|| invalid_vga_snapshot("VGA vertical tile count overflows"))?)
            / VGA_Y_TILESIZE;
        if u32::from(self.core.num_x_tiles) != expected_x_tiles
            || u32::from(self.core.num_y_tiles) != expected_y_tiles
        {
            return Err(invalid_vga_snapshot("VGA tile topology does not match configuration"));
        }
        let tile_count = expected_x_tiles
            .checked_mul(expected_y_tiles)
            .ok_or_else(|| invalid_vga_snapshot("VGA tile count overflows"))?;
        let tile_count = usize::try_from(tile_count)
            .map_err(|_| invalid_vga_snapshot("VGA tile count does not fit usize"))?;
        if self.core.vga_tile_updated.len() != tile_count {
            return Err(invalid_vga_snapshot("VGA tile cache capacity mismatch"));
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl rusty_box_core::snap::SnapshotSection for VgaCard<StdVga> {
    const TAG: u32 = SEC_VGA;
    type Restored = VgaSnapshotRestoreTarget;

    /// Returns the exact byte length of the standalone VGA v3 section payload.
    fn snapshot_len(&self) -> SnapResult<u64> {
        self.validate_snapshot_v3_source()?;

        let text_len = snapshot_v3_usize_len(VGA_TEXT_MEM_SIZE)?;
        let planar_len = snapshot_v3_usize_len(VGA_MEM_SIZE)?;
        let pci_len = snapshot_v3_usize_len(self.ext.pci_conf.len())?;
        let palette_len = checked_section_len_mul(
            snapshot_v3_usize_len(self.core.pel_data.len())?,
            snapshot_v3_usize_len(usize::from(PEL_CYCLES_PER_COLOR))?,
        )?;
        let vbe_len = snapshot_v3_usize_len(self.core.vbe_memory.len())?;

        // Section version plus the scalar state written before every array.
        // 90 = 84 + feature_control (u8) + y_doublescan (bool)
        //         + charmap_address1/2 (2 x u16). The extracted charmap buffers
        //         are derived from planar memory and re-extracted on restore.
        let mut len = checked_section_len_add(4, 90)?;
        for array_len in [
            pci_len,
            snapshot_v3_usize_len(self.core.crtc_regs.len())?,
            snapshot_v3_usize_len(self.core.attr_regs.len())?,
            snapshot_v3_usize_len(self.core.seq_regs.len())?,
            snapshot_v3_usize_len(self.core.graphics_regs.len())?,
            text_len,
            palette_len,
            snapshot_v3_usize_len(self.core.latch.len())?,
            planar_len,
        ] {
            len = checked_section_len_add(len, 4)?;
            len = checked_section_len_add(len, array_len)?;
        }

        // VBE backing storage is configured, not guest-sized, but its length is
        // encoded as u64 to prevent a format-dependent host-size conversion.
        len = checked_section_len_add(len, 8)?;
        checked_section_len_add(len, vbe_len)
    }

    /// Streams the complete standalone VGA v3 section, including its version.
    ///
    /// Fixed buffers are written straight to the destination; this method never
    /// creates a payload vector or a copy of the framebuffer.
    fn save<W: SnapWrite>(&self, writer: &mut W) -> SnapResult<()> {
        self.validate_snapshot_v3_source()?;

        writer.write_u32(SECTION_VERSION)?;
        writer.write_u8(self.core.crtc_index)?;
        writer.write_u8(self.core.attr_index)?;
        writer.write_bool(self.core.attr_flip_flop)?;
        writer.write_bool(self.core.video_enabled)?;
        writer.write_u8(self.core.seq_index)?;
        writer.write_u8(self.core.graphics_index)?;
        writer.write_u8(self.core.status_reg)?;
        writer.write_u8(self.core.misc_output)?;
        writer.write_bool(self.core.vga_enabled)?;

        writer.write_u8(self.core.pel_mask)?;
        writer.write_u8(self.core.dac_state)?;
        writer.write_u8(self.core.pel_write_addr)?;
        writer.write_u8(self.core.pel_read_addr)?;
        writer.write_u8(self.core.pel_write_cycle)?;
        writer.write_u8(self.core.pel_read_cycle)?;

        writer.write_bool(self.core.has_icount_sync)?;
        writer.write_u64(self.core.ips)?;
        writer.write_u32(self.ext.vbe_memsize)?;
        writer.write_bool(self.core.preferred_mode.is_some())?;
        let preferred_mode = self.core.preferred_mode.unwrap_or((0, 0, 0));
        writer.write_u16(preferred_mode.0)?;
        writer.write_u16(preferred_mode.1)?;
        writer.write_u16(preferred_mode.2)?;

        let vbe = VgaSnapshotVbeState::from(&self.ext.vbe);
        writer.write_u16(vbe.cur_dispi)?;
        writer.write_u16(vbe.max_xres)?;
        writer.write_u16(vbe.max_yres)?;
        writer.write_u16(vbe.max_bpp)?;
        writer.write_u16(vbe.xres)?;
        writer.write_u16(vbe.yres)?;
        writer.write_u16(vbe.bpp)?;
        writer.write_u16(vbe.bank[0])?;
        writer.write_u16(vbe.bank[1])?;
        writer.write_u16(vbe.bank_granularity_kb)?;
        writer.write_u16(vbe.enabled)?;
        writer.write_u16(vbe.curindex)?;
        writer.write_u16(vbe.offset_x)?;
        writer.write_u16(vbe.offset_y)?;
        writer.write_u16(vbe.virtual_xres)?;
        writer.write_u16(vbe.virtual_yres)?;
        writer.write_bool(vbe.get_capabilities)?;
        writer.write_bool(vbe.dac_8bit)?;
        writer.write_bool(vbe.ddc_enabled)?;

        writer.write_u32(self.core.ext_start_addr)?;
        let mapping_target = self.ext.snapshot_v3_mapping_target();
        // Bochs registers both in its VGA state list (vgacore.cc register_state:
        // "feature_control" and BXRS_PARAM_BOOL y_doublescan).
        writer.write_u8(self.core.feature_control as u8)?;
        writer.write_bool(self.core.y_doublescan)?;
        writer.write_u16(self.core.charmap_address1)?;
        writer.write_u16(self.core.charmap_address2)?;
        writer.write_bool(self.core.ext_y_dblsize)?;
        writer.write_bool(self.ext.pci_enabled)?;
        writer.write_u32(mapping_target.lfb_base)?;
        writer.write_u32(mapping_target.mmio_base)?;

        write_snapshot_u32_len(writer, self.ext.pci_conf.len())?;
        writer.write_bytes(&self.ext.pci_conf)?;
        write_snapshot_u32_len(writer, self.core.crtc_regs.len())?;
        writer.write_bytes(&self.core.crtc_regs)?;
        write_snapshot_u32_len(writer, self.core.attr_regs.len())?;
        writer.write_bytes(&self.core.attr_regs)?;
        write_snapshot_u32_len(writer, self.core.seq_regs.len())?;
        writer.write_bytes(&self.core.seq_regs)?;
        write_snapshot_u32_len(writer, self.core.graphics_regs.len())?;
        writer.write_bytes(&self.core.graphics_regs)?;
        write_snapshot_u32_len(writer, self.core.text_memory.len())?;
        writer.write_bytes(&self.core.text_memory)?;

        let palette_len = checked_section_len_mul(
            snapshot_v3_usize_len(self.core.pel_data.len())?,
            snapshot_v3_usize_len(usize::from(PEL_CYCLES_PER_COLOR))?,
        )?;
        writer.write_u32(
            u32::try_from(palette_len)
                .map_err(|_| invalid_vga_snapshot("VGA palette length does not fit u32"))?,
        )?;
        for color in &self.core.pel_data {
            writer.write_bytes(color)?;
        }

        write_snapshot_u32_len(writer, self.core.latch.len())?;
        writer.write_bytes(&self.core.latch)?;
        write_snapshot_u32_len(writer, self.core.vga_memory.len())?;
        writer.write_bytes(&self.core.vga_memory)?;
        writer.write_u64(snapshot_v3_usize_len(self.core.vbe_memory.len())?)?;
        writer.write_bytes(&self.core.vbe_memory)
    }

    /// Restores one bounded VGA v3 section and returns its desired BAR bases.
    ///
    /// Decoding never changes `vbe.base_address` or `mmio_base`, nor does it
    /// register a memory handler.  The caller must use the returned target only
    /// after the machine-level atomic relocation succeeds.
    fn restore<R: SnapRead>(
        &mut self,
        reader: &mut R,
    ) -> SnapResult<VgaSnapshotRestoreTarget> {
        if reader.read_u32()? != SECTION_VERSION {
            return Err(invalid_vga_snapshot("unsupported VGA snapshot section version"));
        }

        let crtc_index = reader.read_u8()?;
        let attr_index = reader.read_u8()?;
        let attr_flip_flop = reader.read_bool()?;
        let video_enabled = reader.read_bool()?;
        let seq_index = reader.read_u8()?;
        let graphics_index = reader.read_u8()?;
        let status_reg = reader.read_u8()?;
        let misc_output = reader.read_u8()?;
        let vga_enabled = reader.read_bool()?;

        let pel_mask = reader.read_u8()?;
        let dac_state = reader.read_u8()?;
        let pel_write_addr = reader.read_u8()?;
        let pel_read_addr = reader.read_u8()?;
        let pel_write_cycle = reader.read_u8()?;
        let pel_read_cycle = reader.read_u8()?;

        let has_icount_sync = reader.read_bool()?;
        let ips = reader.read_u64()?;
        let vbe_memsize = reader.read_u32()?;
        let preferred_mode = if reader.read_bool()? {
            Some((reader.read_u16()?, reader.read_u16()?, reader.read_u16()?))
        } else {
            let ignored = (reader.read_u16()?, reader.read_u16()?, reader.read_u16()?);
            if ignored != (0, 0, 0) {
                return Err(invalid_vga_snapshot("absent preferred VBE mode is nonzero"));
            }
            None
        };
        let saved_vbe = VgaSnapshotVbeState {
            cur_dispi: reader.read_u16()?,
            max_xres: reader.read_u16()?,
            max_yres: reader.read_u16()?,
            max_bpp: reader.read_u16()?,
            xres: reader.read_u16()?,
            yres: reader.read_u16()?,
            bpp: reader.read_u16()?,
            bank: [reader.read_u16()?, reader.read_u16()?],
            bank_granularity_kb: reader.read_u16()?,
            enabled: reader.read_u16()?,
            curindex: reader.read_u16()?,
            offset_x: reader.read_u16()?,
            offset_y: reader.read_u16()?,
            virtual_xres: reader.read_u16()?,
            virtual_yres: reader.read_u16()?,
            get_capabilities: reader.read_bool()?,
            dac_8bit: reader.read_bool()?,
            ddc_enabled: reader.read_bool()?,
        };
        let ext_start_addr = reader.read_u32()?;
        let feature_control = reader.read_u8()?;
        let y_doublescan = reader.read_bool()?;
        let charmap_address1 = reader.read_u16()?;
        let charmap_address2 = reader.read_u16()?;
        let ext_y_dblsize = reader.read_bool()?;
        let pci_enabled = reader.read_bool()?;
        let target = VgaSnapshotRestoreTarget {
            lfb_base: reader.read_u32()?,
            mmio_base: reader.read_u32()?,
        };

        self.validate_snapshot_v3_scalars(
            crtc_index,
            attr_index,
            dac_state,
            pel_write_cycle,
            pel_read_cycle,
            has_icount_sync,
            ips,
            vbe_memsize,
            preferred_mode,
            &saved_vbe,
            pci_enabled,
            target,
        )?;

        let pci_len = read_snapshot_u32_len(reader, self.ext.pci_conf.len(), "PCI config")?;
        if pci_len != self.ext.pci_conf.len() {
            return Err(invalid_vga_snapshot("VGA PCI config length mismatch"));
        }
        let mut saved_bar0 = 0u32;
        let mut saved_bar2 = 0u32;
        let expected_bar0_low = self.ext.pci_conf[0x10] & 0x0f;
        let expected_bar2_low = self.ext.pci_conf[0x18] & 0x0f;
        for index in 0..self.ext.pci_conf.len() {
            let saved = reader.read_u8()?;
            let live = self.ext.pci_conf[index];
            if !vga_snapshot_pci_byte_is_mutable(index) && saved != live {
                return Err(invalid_vga_snapshot("VGA immutable PCI config mismatch"));
            }
            if index == 0x10 && saved & 0x0f != expected_bar0_low {
                return Err(invalid_vga_snapshot("VGA BAR0 type bits changed"));
            }
            if index == 0x18 && saved & 0x0f != expected_bar2_low {
                return Err(invalid_vga_snapshot("VGA BAR2 type bits changed"));
            }
            match index {
                0x10 => saved_bar0 |= u32::from(saved),
                0x11 => saved_bar0 |= u32::from(saved) << 8,
                0x12 => saved_bar0 |= u32::from(saved) << 16,
                0x13 => saved_bar0 |= u32::from(saved) << 24,
                0x18 => saved_bar2 |= u32::from(saved),
                0x19 => saved_bar2 |= u32::from(saved) << 8,
                0x1a => saved_bar2 |= u32::from(saved) << 16,
                0x1b => saved_bar2 |= u32::from(saved) << 24,
                _ => {}
            }
            self.ext.pci_conf[index] = saved;
        }
        if pci_enabled
            && (saved_bar0 & !(self.ext.vbe_memsize - 1) != target.lfb_base
                || saved_bar2 & !(PCI_VGA_MMIO_SIZE - 1) != target.mmio_base)
        {
            return Err(invalid_vga_snapshot(
                "VGA desired BAR target disagrees with PCI configuration",
            ));
        }

        self.core.crtc_index = crtc_index;
        self.core.attr_index = attr_index;
        self.core.attr_flip_flop = attr_flip_flop;
        self.core.video_enabled = video_enabled;
        self.core.seq_index = seq_index;
        self.core.graphics_index = graphics_index;
        self.core.status_reg = status_reg;
        self.core.misc_output = misc_output;
        self.core.vga_enabled = vga_enabled;
        self.core.pel_mask = pel_mask;
        self.core.dac_state = dac_state;
        self.core.pel_write_addr = pel_write_addr;
        self.core.pel_read_addr = pel_read_addr;
        self.core.pel_write_cycle = pel_write_cycle;
        self.core.pel_read_cycle = pel_read_cycle;
        self.ext.vbe.cur_dispi = saved_vbe.cur_dispi;
        self.ext.vbe.max_xres = saved_vbe.max_xres;
        self.ext.vbe.max_yres = saved_vbe.max_yres;
        self.ext.vbe.max_bpp = saved_vbe.max_bpp;
        self.ext.vbe.xres = saved_vbe.xres;
        self.ext.vbe.yres = saved_vbe.yres;
        self.ext.vbe.bpp = saved_vbe.bpp;
        self.ext.vbe.bank = saved_vbe.bank;
        self.ext.vbe.bank_granularity_kb = saved_vbe.bank_granularity_kb;
        self.ext.vbe.enabled = saved_vbe.enabled;
        self.ext.vbe.curindex = saved_vbe.curindex;
        self.ext.vbe.offset_x = saved_vbe.offset_x;
        self.ext.vbe.offset_y = saved_vbe.offset_y;
        self.ext.vbe.virtual_xres = saved_vbe.virtual_xres;
        self.ext.vbe.virtual_yres = saved_vbe.virtual_yres;
        self.ext.vbe.get_capabilities = saved_vbe.get_capabilities;
        self.ext.vbe.dac_8bit = saved_vbe.dac_8bit;
        self.ext.vbe.ddc_enabled = saved_vbe.ddc_enabled;
        self.core.ext_start_addr = ext_start_addr;
        self.core.feature_control = feature_control;
        self.core.y_doublescan = y_doublescan;
        self.core.charmap_address1 = charmap_address1;
        self.core.charmap_address2 = charmap_address2;
        // Re-derive the character generators from the restored planar memory
        // (Bochs likewise rebuilds them from state rather than storing glyphs).
        self.core.update_charmap();
        self.core.ext_y_dblsize = ext_y_dblsize;
        self.ext.pending_lfb_relocate = None;
        self.ext.pending_mmio_base = None;

        read_snapshot_fixed_array(reader, &mut self.core.crtc_regs, "CRTC registers")?;
        read_snapshot_fixed_array(reader, &mut self.core.attr_regs, "attribute registers")?;
        read_snapshot_fixed_array(reader, &mut self.core.seq_regs, "sequencer registers")?;
        read_snapshot_fixed_array(reader, &mut self.core.graphics_regs, "graphics registers")?;
        read_snapshot_fixed_array(reader, &mut self.core.text_memory, "text memory")?;

        let palette_len = read_snapshot_u32_len(
            reader,
            self.core.pel_data
                .len()
                .checked_mul(usize::from(PEL_CYCLES_PER_COLOR))
                .ok_or_else(|| invalid_vga_snapshot("VGA palette length overflows"))?,
            "DAC palette",
        )?;
        if palette_len
            != self
                .core
                .pel_data
                .len()
                .checked_mul(usize::from(PEL_CYCLES_PER_COLOR))
                .ok_or_else(|| invalid_vga_snapshot("VGA palette length overflows"))?
        {
            return Err(invalid_vga_snapshot("VGA DAC palette length mismatch"));
        }
        for color in &mut self.core.pel_data {
            reader.read_bytes(color)?;
        }

        read_snapshot_fixed_array(reader, &mut self.core.latch, "graphics latch")?;
        read_snapshot_fixed_array(reader, &mut self.core.vga_memory, "planar VGA memory")?;

        let vbe_len = reader.read_len(self.core.vbe_memory.len())?;
        if vbe_len != self.core.vbe_memory.len() {
            return Err(invalid_vga_snapshot("VBE memory length does not match configuration"));
        }
        reader.read_bytes(&mut self.core.vbe_memory)?;

        Ok(target)
    }

}

/// The DISPI register file and the PCI device — Bochs vga.cc, which is
/// `bx_vga_c`'s and not the core's. Entry points that reprogram the standard
/// VGA underneath take the core they sit on.
impl StdVga {
    fn vbe_read_index(&self, index: u16) -> u16 {
        match index {
            VBE_DISPI_INDEX_ID => self.vbe.cur_dispi,
            VBE_DISPI_INDEX_XRES => {
                if self.vbe.get_capabilities {
                    self.vbe.max_xres
                } else {
                    self.vbe.xres
                }
            }
            VBE_DISPI_INDEX_YRES => {
                if self.vbe.get_capabilities {
                    self.vbe.max_yres
                } else {
                    self.vbe.yres
                }
            }
            VBE_DISPI_INDEX_BPP => {
                if self.vbe.get_capabilities {
                    self.vbe.max_bpp
                } else {
                    self.vbe.bpp
                }
            }
            VBE_DISPI_INDEX_ENABLE => {
                let mut value = self.vbe.enabled;
                if self.vbe.get_capabilities {
                    value |= VBE_DISPI_GETCAPS;
                }
                if self.vbe.dac_8bit {
                    value |= VBE_DISPI_8BIT_DAC;
                }
                value
            }
            VBE_DISPI_INDEX_BANK => self.vbe.bank[0],
            VBE_DISPI_INDEX_X_OFFSET => self.vbe.offset_x,
            VBE_DISPI_INDEX_Y_OFFSET => self.vbe.offset_y,
            VBE_DISPI_INDEX_VIRT_WIDTH => self.vbe.virtual_xres,
            VBE_DISPI_INDEX_VIRT_HEIGHT => self.vbe.virtual_yres,
            VBE_DISPI_INDEX_VIDEO_MEMORY_64K => (self.vbe_memsize >> 16) as u16,
            VBE_DISPI_INDEX_DDC => {
                // Bochs vga.cc vbe_read VBE_DISPI_INDEX_DDC: bit 7 reports
                // the interface enabled, low bits are the DDC line states;
                // disabled reads as 0x000F.
                if self.vbe.ddc_enabled {
                    (1 << 7) | self.ddc.read() as u16
                } else {
                    0x000F
                }
            }
            _ => {
                tracing::error!("VBE read: unknown index 0x{:x}", index);
                0
            }
        }
    }

    /// MMIO read handler for BAR2.
    ///
    /// Translates MMIO offset reads into VBE register reads.
    /// Matches Bochs `bx_vga_c::vbe_mmio_read_handler`.
    pub(crate) fn vbe_mmio_read(
        &mut self,
        at: crate::api::WindowOffset,
        len: u32,
        data: &mut [u8],
    ) {
        let offset = (at.get() & 0xFFF) as u32;
        let mut value: u32 = 0xFFFF_FFFF;

        if offset >= PCI_VGA_BOCHS_OFFSET && offset < PCI_VGA_BOCHS_OFFSET + PCI_VGA_BOCHS_SIZE {
            let reg_offset = offset - PCI_VGA_BOCHS_OFFSET;
            let index = (reg_offset >> 1) as u16;
            self.vbe.curindex = index;
            value = self.vbe_read_index(index) as u32;
        }

        match len {
            1 => {
                if let Some(d) = data.first_mut() {
                    *d = value as u8;
                }
            }
            2 => {
                let bytes = (value as u16).to_le_bytes();
                data[..2].copy_from_slice(&bytes);
            }
            4 => {
                let bytes = value.to_le_bytes();
                data[..4].copy_from_slice(&bytes);
            }
            _ => {
                tracing::error!("vbe_mmio_read: unsupported len={}", len);
            }
        }
    }

    /// MMIO write handler for BAR2.
    ///
    /// Translates MMIO offset writes into VBE register writes.
    /// Matches Bochs `bx_vga_c::vbe_mmio_write_handler`.
    pub(crate) fn vbe_mmio_write(
        &mut self, core: &mut VgaCore,
        at: crate::api::WindowOffset,
        len: u32,
        data: &[u8],
    ) {
        let offset = (at.get() & 0xFFF) as u32;

        let value: u32 = match len {
            1 => data.first().copied().unwrap_or(0) as u32,
            2 => {
                let mut buf = [0u8; 2];
                buf[..data.len().min(2)].copy_from_slice(&data[..data.len().min(2)]);
                u16::from_le_bytes(buf) as u32
            }
            4 => {
                let mut buf = [0u8; 4];
                buf[..data.len().min(4)].copy_from_slice(&data[..data.len().min(4)]);
                u32::from_le_bytes(buf)
            }
            _ => {
                tracing::error!("vbe_mmio_write: unsupported len={}", len);
                return;
            }
        };

        if offset >= PCI_VGA_BOCHS_OFFSET && offset < PCI_VGA_BOCHS_OFFSET + PCI_VGA_BOCHS_SIZE {
            let reg_offset = offset - PCI_VGA_BOCHS_OFFSET;
            let index = (reg_offset >> 1) as u16;
            self.vbe.curindex = index;
            self.vbe_write_index(core, index, value as u16);
        }
    }

    /// Handle a VBE data-port write (port 0x01CF or MMIO-dispatched).
    ///
    /// Matches the Bochs `vbe_write` / `vbe_write_handler` logic.
    fn vbe_write_index(&mut self, core: &mut VgaCore, index: u16, value16: u16) {
        let mut needs_update = false;

        match index {
            VBE_DISPI_INDEX_ID => {
                // Accept any known DISPI ID
                if value16 >= VBE_DISPI_ID0 && value16 <= VBE_DISPI_ID5 {
                    self.vbe.cur_dispi = value16;
                }
            }
            VBE_DISPI_INDEX_XRES => {
                if self.vbe.enabled == 0 {
                    if value16 <= self.vbe.max_xres {
                        self.vbe.xres = value16;
                    }
                }
            }
            VBE_DISPI_INDEX_YRES => {
                if self.vbe.enabled == 0 {
                    if value16 <= self.vbe.max_yres {
                        self.vbe.yres = value16;
                    }
                }
            }
            VBE_DISPI_INDEX_BPP => {
                if self.vbe.enabled == 0 {
                    let bpp = if value16 == 0 {
                        VBE_DISPI_BPP_8
                    } else {
                        value16
                    };
                    if bpp == VBE_DISPI_BPP_4
                        || bpp == VBE_DISPI_BPP_8
                        || bpp == VBE_DISPI_BPP_15
                        || bpp == VBE_DISPI_BPP_16
                        || bpp == VBE_DISPI_BPP_24
                        || bpp == VBE_DISPI_BPP_32
                    {
                        self.vbe.bpp = bpp;
                    }
                }
            }
            VBE_DISPI_INDEX_BANK => {
                let num_banks = {
                    let mut nb = (core.vbe_memsize >> 10) / self.vbe.bank_granularity_kb as u32;
                    if self.vbe.bpp == VBE_DISPI_BPP_4 {
                        nb >>= 2;
                    }
                    nb as u16
                };
                let rw_mode = if (value16 & VBE_DISPI_BANK_RW) != 0 {
                    value16 & VBE_DISPI_BANK_RW
                } else {
                    VBE_DISPI_BANK_RW // compatibility mode
                };
                let bank_val = value16 & 0x1ff;
                if bank_val < num_banks {
                    if (rw_mode & VBE_DISPI_BANK_WR) != 0 {
                        self.vbe.bank[0] = bank_val;
                    }
                    if (rw_mode & VBE_DISPI_BANK_RD) != 0 {
                        self.vbe.bank[1] = bank_val;
                    }
                    core.ext_offset =
                        self.vbe.bank[0] as u32 * ((self.vbe.bank_granularity_kb as u32) << 10);
                    core.ext_read_offset =
                        self.vbe.bank[1] as u32 * ((self.vbe.bank_granularity_kb as u32) << 10);
                }
            }
            VBE_DISPI_INDEX_ENABLE => {
                if (value16 & VBE_DISPI_ENABLED) != 0 && self.vbe.enabled == 0 {
                    // Enabling VBE mode
                    self.vbe.virtual_yres = self.vbe.yres;
                    self.vbe.virtual_xres = self.vbe.xres;

                    self.vbe.offset_x = 0;
                    self.vbe.offset_y = 0;
                    self.vbe.virtual_start = 0;
                    core.ext_offset = 0;
                    core.ext_read_offset = 0;
                    core.vga_mem_mask = core.vbe_memsize.saturating_sub(1);
                    self.vbe.bank = [0; 2];

                    match self.vbe.bpp {
                        VBE_DISPI_BPP_4 => {
                            self.vbe.bpp_multiplier = 1;
                            self.vbe.line_offset = self.vbe.virtual_xres >> 3;
                        }
                        VBE_DISPI_BPP_8 => {
                            self.vbe.bpp_multiplier = 1;
                            self.vbe.line_offset = self.vbe.virtual_xres;
                        }
                        VBE_DISPI_BPP_15 => {
                            self.vbe.bpp_multiplier = 2;
                            self.vbe.line_offset = self.vbe.virtual_xres * 2;
                        }
                        VBE_DISPI_BPP_16 => {
                            self.vbe.bpp_multiplier = 2;
                            self.vbe.line_offset = self.vbe.virtual_xres * 2;
                        }
                        VBE_DISPI_BPP_24 => {
                            self.vbe.bpp_multiplier = 3;
                            self.vbe.line_offset = self.vbe.virtual_xres * 3;
                        }
                        VBE_DISPI_BPP_32 => {
                            self.vbe.bpp_multiplier = 4;
                            self.vbe.line_offset = self.vbe.virtual_xres << 2;
                        }
                        _ => {}
                    }
                    self.vbe.visible_screen_size =
                        self.vbe.line_offset as u32 * self.vbe.yres as u32;

                    #[cfg(feature = "alloc")]
                    {
                        if core.vbe_memory.len() != core.vbe_memsize as usize {
                            core.vbe_memory.resize(core.vbe_memsize as usize, 0);
                        }
                        if (value16 & VBE_DISPI_NOCLEARMEM) == 0 {
                            core.vbe_memory.fill(0);
                        }
                        core.redraw_area(0, 0, self.vbe.xres as u32, self.vbe.yres as u32);
                    }
                    #[cfg(not(feature = "alloc"))]
                    if (value16 & VBE_DISPI_NOCLEARMEM) == 0 {
                        core.vga_memory.fill(0);
                    }

                    if self.vbe.bpp != VBE_DISPI_BPP_4 {
                        core.last_bpp = self.vbe.bpp as u32;
                        core.last_fh = 0;
                    }
                } else if (value16 & VBE_DISPI_ENABLED) == 0 && self.vbe.enabled != 0 {
                    // Disabling VBE mode — return to legacy VGA
                    core.text_buffer_update = true;
                    core.last_yres = 0;
                    core.ext_offset = 0;
                    core.ext_read_offset = 0;
                    core.vga_mem_mask = (VGA_MEM_SIZE - 1) as u32;
                    self.vbe.bank = [0; 2];
                }

                self.vbe.enabled = value16 & VBE_DISPI_ENABLED;
                self.vbe.get_capabilities = (value16 & VBE_DISPI_GETCAPS) != 0;

                // Handle bank granularity change
                let new_bank_gran: u16 = if (value16 & VBE_DISPI_BANK_GRANULARITY_32K) != 0 {
                    32
                } else {
                    64
                };
                if new_bank_gran != self.vbe.bank_granularity_kb {
                    self.vbe.bank_granularity_kb = new_bank_gran;
                    self.vbe.bank[0] = 0;
                    self.vbe.bank[1] = 0;
                    core.ext_offset = 0;
                    core.ext_read_offset = 0;
                }

                // Handle 8-bit DAC mode change
                let new_dac_8bit = (value16 & VBE_DISPI_8BIT_DAC) != 0;
                if new_dac_8bit != self.vbe.dac_8bit {
                    if new_dac_8bit {
                        for i in 0..256 {
                            core.pel_data[i][0] <<= 2;
                            core.pel_data[i][1] <<= 2;
                            core.pel_data[i][2] <<= 2;
                        }
                    } else {
                        for i in 0..256 {
                            core.pel_data[i][0] >>= 2;
                            core.pel_data[i][1] >>= 2;
                            core.pel_data[i][2] >>= 2;
                        }
                    }
                    self.vbe.dac_8bit = new_dac_8bit;
                    core.dac_shift = dac_shift_for(new_dac_8bit);
                    needs_update = true;
                }
            }
            VBE_DISPI_INDEX_X_OFFSET => {
                self.vbe.offset_x = value16;
                self.recompute_vbe_virtual_start(core);
                needs_update = true;
            }
            VBE_DISPI_INDEX_Y_OFFSET => {
                self.vbe.offset_y = value16;
                self.recompute_vbe_virtual_start(core);
                needs_update = true;
            }
            VBE_DISPI_INDEX_VIRT_WIDTH => {
                let new_width = value16;
                let new_height = if self.vbe.bpp != VBE_DISPI_BPP_4 {
                    (core.vbe_memsize / self.vbe.bpp_multiplier as u32) / new_width as u32
                } else {
                    (core.vbe_memsize << 1) / new_width as u32
                };
                let (final_width, final_height) = if new_height as u16 >= self.vbe.yres {
                    (new_width, new_height as u16)
                } else {
                    // Cannot fit: recalculate width for yres
                    let h = self.vbe.yres;
                    let w = if self.vbe.bpp != VBE_DISPI_BPP_4 {
                        (core.vbe_memsize / self.vbe.bpp_multiplier as u32) / h as u32
                    } else {
                        (core.vbe_memsize << 1) / h as u32
                    };
                    (w as u16, h)
                };
                self.vbe.virtual_xres = final_width;
                self.vbe.virtual_yres = final_height;
                if self.vbe.bpp != VBE_DISPI_BPP_4 {
                    self.vbe.line_offset = self.vbe.virtual_xres * self.vbe.bpp_multiplier as u16;
                } else {
                    self.vbe.line_offset = self.vbe.virtual_xres >> 3;
                }
                self.vbe.visible_screen_size = self.vbe.line_offset as u32 * self.vbe.yres as u32;
                self.recompute_vbe_virtual_start(core);
                needs_update = true;
            }
            VBE_DISPI_INDEX_VIRT_HEIGHT => {
                // Read-only in Bochs; ignore writes
            }
            VBE_DISPI_INDEX_DDC => {
                // Bochs vga.cc vbe_write VBE_DISPI_INDEX_DDC: bit 7 enables
                // the DDC interface; bits 0/1 drive the I2C clock (DCK) and
                // data (DDA) lines of the monitor's EDID channel.
                if (value16 >> 7) & 1 != 0 {
                    self.vbe.ddc_enabled = true;
                    self.ddc.write(value16 & 1 != 0, (value16 >> 1) & 1 != 0);
                } else {
                    self.vbe.ddc_enabled = false;
                }
            }
            _ => {
                tracing::error!(
                    "VBE write: unknown index 0x{:x}, value 0x{:x}",
                    index,
                    value16
                );
            }
        }

        if needs_update {
            core.vga_mem_updated = 1;
            #[cfg(feature = "alloc")]
            core.redraw_area(0, 0, self.vbe.xres as u32, self.vbe.yres as u32);
        }
        // Enable and depth are the two registers that decide it, and both
        // arrive here.
        core.vbe_planar_alias = self.vbe.enabled != 0 && self.vbe.bpp == VBE_DISPI_BPP_4;

    }

    fn apply_preferred_mode(&mut self, core: &mut VgaCore) {
        let Some((xres, yres, bpp)) = core.preferred_mode else {
            return;
        };
        // Never lower the built-in defaults; only raise the ceiling so the
        // requested mode is not rejected by the DISPI xres/yres range checks.
        self.vbe.max_xres = self.vbe.max_xres.max(xres);
        self.vbe.max_yres = self.vbe.max_yres.max(yres);
        self.vbe.max_bpp = self.vbe.max_bpp.max(bpp);
        // Power-on VBE dimensions (the guest may still program its own mode).
        self.vbe.xres = xres;
        self.vbe.yres = yres;
        self.vbe.bpp = bpp;
        self.vbe.virtual_xres = xres;
        self.vbe.virtual_yres = yres;

        // Grow the dirty-tile grid to cover the (possibly larger) capability.
        let num_x_tiles = ((self.vbe.max_xres as u32 + VGA_X_TILESIZE - 1) / VGA_X_TILESIZE) as u16;
        let num_y_tiles = ((self.vbe.max_yres as u32 + VGA_Y_TILESIZE - 1) / VGA_Y_TILESIZE) as u16;
        if num_x_tiles != core.num_x_tiles || num_y_tiles != core.num_y_tiles {
            core.num_x_tiles = num_x_tiles;
            core.num_y_tiles = num_y_tiles;
            #[cfg(feature = "alloc")]
            {
                core.vga_tile_updated = vec![true; num_x_tiles as usize * num_y_tiles as usize];
            }
        }
    }

    #[cfg(feature = "std")]
    fn validate_vbe_snapshot_state(&self, vbe: &VgaSnapshotVbeState) -> SnapResult<()> {
        if self.vbe_memsize == 0 || !self.vbe_memsize.is_power_of_two() {
            return Err(invalid_vga_snapshot("VBE memory configuration is not a power of two"));
        }
        if vbe.max_xres != self.vbe.max_xres
            || vbe.max_yres != self.vbe.max_yres
            || vbe.max_bpp != self.vbe.max_bpp
        {
            return Err(invalid_vga_snapshot("VBE capability configuration mismatch"));
        }
        if vbe.cur_dispi < VBE_DISPI_ID0 || vbe.cur_dispi > VBE_DISPI_ID5 {
            return Err(invalid_vga_snapshot("VBE DISPI identifier is invalid"));
        }
        if vbe.xres == 0
            || vbe.yres == 0
            || vbe.xres > vbe.max_xres
            || vbe.yres > vbe.max_yres
            || !vga_snapshot_bpp_is_valid(vbe.bpp)
            || vbe.bpp > vbe.max_bpp
        {
            return Err(invalid_vga_snapshot("VBE resolution or bpp is invalid"));
        }
        if vbe.enabled != VBE_DISPI_DISABLED && vbe.enabled != VBE_DISPI_ENABLED {
            return Err(invalid_vga_snapshot("VBE enable value is invalid"));
        }
        if vbe.bank_granularity_kb != 32 && vbe.bank_granularity_kb != 64 {
            return Err(invalid_vga_snapshot("VBE bank granularity is invalid"));
        }
        if vbe.virtual_xres == 0
            || vbe.virtual_yres == 0
            || vbe.virtual_xres < vbe.xres
            || vbe.virtual_yres < vbe.yres
        {
            return Err(invalid_vga_snapshot("VBE virtual resolution is invalid"));
        }

        let (_, line_offset) = vga_snapshot_vbe_layout(vbe.bpp, vbe.virtual_xres)?;
        let virtual_size = u32::from(line_offset)
            .checked_mul(u32::from(vbe.virtual_yres))
            .ok_or_else(|| invalid_vga_snapshot("VBE virtual-screen size overflows"))?;
        let visible_size = u32::from(line_offset)
            .checked_mul(u32::from(vbe.yres))
            .ok_or_else(|| invalid_vga_snapshot("VBE visible-screen size overflows"))?;
        if virtual_size > self.vbe_memsize || visible_size > self.vbe_memsize {
            return Err(invalid_vga_snapshot("VBE screen geometry exceeds configured memory"));
        }
        if u32::from(vbe.offset_x)
            .checked_add(u32::from(vbe.xres))
            .ok_or_else(|| invalid_vga_snapshot("VBE horizontal offset overflows"))?
            > u32::from(vbe.virtual_xres)
            || u32::from(vbe.offset_y)
                .checked_add(u32::from(vbe.yres))
                .ok_or_else(|| invalid_vga_snapshot("VBE vertical offset overflows"))?
                > u32::from(vbe.virtual_yres)
        {
            return Err(invalid_vga_snapshot("VBE display offset is outside the virtual screen"));
        }

        let bank_granularity = u32::from(vbe.bank_granularity_kb)
            .checked_mul(1024)
            .ok_or_else(|| invalid_vga_snapshot("VBE bank granularity overflows"))?;
        let mut bank_count = self.vbe_memsize / bank_granularity;
        if vbe.bpp == VBE_DISPI_BPP_4 {
            bank_count /= 4;
        }
        if bank_count == 0
            || u32::from(vbe.bank[0]) >= bank_count
            || u32::from(vbe.bank[1]) >= bank_count
        {
            return Err(invalid_vga_snapshot("VBE bank is outside configured memory"));
        }
        for bank in vbe.bank {
            if vga_snapshot_bank_offset(bank, vbe.bank_granularity_kb)? >= self.vbe_memsize {
                return Err(invalid_vga_snapshot("VBE bank offset is outside configured memory"));
            }
        }
        Ok(())
    }

    fn recompute_vbe_virtual_start(&mut self, core: &mut VgaCore) {
        let mut virtual_start = self.vbe.offset_y as u32 * self.vbe.line_offset as u32;
        if self.vbe.bpp != VBE_DISPI_BPP_4 {
            virtual_start = virtual_start
                .wrapping_add(self.vbe.offset_x as u32 * self.vbe.bpp_multiplier as u32);
        } else {
            virtual_start = virtual_start.wrapping_add((self.vbe.offset_x as u32) >> 3);
        }
        self.vbe.virtual_start = virtual_start & core.vga_mem_mask;
    }

    #[cfg(feature = "alloc")]
    fn vbe_mem_read_byte(&mut self, core: &mut VgaCore, addr: BxPhyAddress) -> u8 {
        let offset = if addr >= self.vbe.base_address as BxPhyAddress {
            addr - self.vbe.base_address as BxPhyAddress
        } else if addr < 0xB0000 {
            self.vbe.bank[1] as BxPhyAddress
                * ((self.vbe.bank_granularity_kb as BxPhyAddress) << 10)
                + (addr & 0xffff)
        } else {
            return 0;
        };

        core.vbe_memory.get(offset as usize).copied().unwrap_or(0)
    }

    #[cfg(feature = "alloc")]
    fn vbe_mem_write_byte(&mut self, core: &mut VgaCore, addr: BxPhyAddress, value: u8) {
        let offset = if addr >= self.vbe.base_address as BxPhyAddress {
            addr - self.vbe.base_address as BxPhyAddress
        } else if addr < 0xB0000 {
            self.vbe.bank[0] as BxPhyAddress
                * ((self.vbe.bank_granularity_kb as BxPhyAddress) << 10)
                + (addr & 0xffff)
        } else {
            return;
        };

        let Some(slot) = core.vbe_memory.get_mut(offset as usize) else {
            return;
        };
        *slot = value;

        let virtual_start = self.vbe.virtual_start as BxPhyAddress;
        if offset < virtual_start {
            return;
        }
        let visible_offset = offset - virtual_start;
        if visible_offset >= self.vbe.visible_screen_size as BxPhyAddress {
            return;
        }

        let bpp_multiplier = self.vbe.bpp_multiplier.max(1) as BxPhyAddress;
        let pixel_offset = visible_offset / bpp_multiplier;
        let virtual_xres = self.vbe.virtual_xres.max(1) as BxPhyAddress;
        let x_tile = ((pixel_offset % virtual_xres) as u32) / VGA_X_TILESIZE;
        let y_tile = ((pixel_offset / virtual_xres) as u32) / VGA_Y_TILESIZE;
        core.mark_tile_updated(x_tile, y_tile);
    }

    #[cfg(feature = "alloc")]
    fn refresh_vbe_graphics<S: DisplaySink>(
        &mut self, core: &mut VgaCore,
        sink: &mut S,
        dirt: crate::display::card::Dirt<'_>,
    ) -> Refreshed {
        if self.vbe.enabled == 0 {
            return Refreshed::Unchanged;
        }

        let width = self.vbe.xres as u32;
        let height = self.vbe.yres as u32;
        if width == 0 || height == 0 {
            return Refreshed::Unchanged;
        }

        let dimension_changed = width != core.last_xres
            || height != core.last_yres
            || self.vbe.bpp as u32 != core.last_bpp;
        if dimension_changed {
            core.redraw_area(0, 0, width, height);
        } else if core.vga_mem_updated == 0 && dirt.reports_nothing() {
            // Both sources must be silent to skip the frame. Bailing out on the
            // device's own bitmap alone would mean a hypervisor's page report
            // never reached the tile loop below it, and the screen would freeze
            // with nothing in the device to say why.
            return Refreshed::Unchanged;
        }

        let vbe_mem_mask = core.vbe_memsize.saturating_sub(1);
        self.vbe.virtual_start &= vbe_mem_mask;

        let pitch = self.vbe.line_offset as u32;
        // Ahead of the tiles: a front end sizes its surface before receiving
        // pixels for it, which is the order the forwarding drain used.
        if dimension_changed {
            sink.dimension_update(Dimensions {
                width,
                height,
                font_width: 0,
                font_height: 0,
                bits_per_pixel: self.vbe.bpp as u8,
            });
        }
        let mut tile_rgba = [0u8; TILE_RGBA_BYTES];

        let bytes_per_pixel = u64::from(self.vbe.bpp_multiplier.max(1));
        for yc in (0..height).step_by(VGA_Y_TILESIZE as usize) {
            let y_tile = yc / VGA_Y_TILESIZE;
            for xc in (0..width).step_by(VGA_X_TILESIZE as usize) {
                let x_tile = xc / VGA_X_TILESIZE;
                let tile_index = y_tile as usize * core.num_x_tiles as usize + x_tile as usize;
                let tile_width = VGA_X_TILESIZE.min(width - xc);
                let tile_height = VGA_Y_TILESIZE.min(height - yc);

                // The union of what the device saw and what the hypervisor
                // reported (REPLAN decision 11). Under the software engine the
                // first term is the whole answer; under a hypervisor the
                // framebuffer is host RAM the guest writes without exiting, so
                // the device saw none of it and the page bitmap is.
                let self_tracked = core
                    .vga_tile_updated
                    .get(tile_index)
                    .copied()
                    .unwrap_or(false);
                let page_tracked = !matches!(dirt, crate::display::card::Dirt::SelfTracked)
                    && (0..tile_height).any(|r| {
                        let row = u64::from(self.vbe.virtual_start.wrapping_add((yc + r) * pitch))
                            & u64::from(vbe_mem_mask);
                        let start = row + u64::from(xc) * bytes_per_pixel;
                        dirt.covers(start, start + u64::from(tile_width) * bytes_per_pixel)
                    });
                if !self_tracked && !page_tracked {
                    continue;
                }

                let rgba = &mut tile_rgba[..(tile_width * tile_height * 4) as usize];

                for r in 0..tile_height {
                    let y = yc + r;
                    let row_addr = self.vbe.virtual_start.wrapping_add(y * pitch) & vbe_mem_mask;
                    for c in 0..tile_width {
                        let x = xc + c;
                        let pixel = match self.vbe.bpp {
                            VBE_DISPI_BPP_4 => {
                                let dac =
                                    core.get_vga_pixel(x as u16, y as u16, row_addr, 0xffff, false);
                                core.dac_index_to_rgba(dac)
                            }
                            VBE_DISPI_BPP_8 => {
                                let offset = (row_addr + x) & vbe_mem_mask;
                                let dac = core.vbe_memory[offset as usize];
                                core.dac_index_to_rgba(dac)
                            }
                            VBE_DISPI_BPP_15 => {
                                let offset = (row_addr + x * 2) & vbe_mem_mask;
                                let lo = core.vbe_memory[offset as usize] as u16;
                                let hi =
                                    core.vbe_memory[((offset + 1) & vbe_mem_mask) as usize] as u16;
                                let value = lo | (hi << 8);
                                let r = ((value >> 10) & 0x1f) as u8;
                                let g = ((value >> 5) & 0x1f) as u8;
                                let b = (value & 0x1f) as u8;
                                [
                                    (r << 3) | (r >> 2),
                                    (g << 3) | (g >> 2),
                                    (b << 3) | (b >> 2),
                                    0xff,
                                ]
                            }
                            VBE_DISPI_BPP_16 => {
                                let offset = (row_addr + x * 2) & vbe_mem_mask;
                                let lo = core.vbe_memory[offset as usize] as u16;
                                let hi =
                                    core.vbe_memory[((offset + 1) & vbe_mem_mask) as usize] as u16;
                                let value = lo | (hi << 8);
                                let r = ((value >> 11) & 0x1f) as u8;
                                let g = ((value >> 5) & 0x3f) as u8;
                                let b = (value & 0x1f) as u8;
                                [
                                    (r << 3) | (r >> 2),
                                    (g << 2) | (g >> 4),
                                    (b << 3) | (b >> 2),
                                    0xff,
                                ]
                            }
                            VBE_DISPI_BPP_24 => {
                                let offset = (row_addr + x * 3) & vbe_mem_mask;
                                let b = core.vbe_memory[offset as usize];
                                let g = core.vbe_memory[((offset + 1) & vbe_mem_mask) as usize];
                                let r = core.vbe_memory[((offset + 2) & vbe_mem_mask) as usize];
                                [r, g, b, 0xff]
                            }
                            VBE_DISPI_BPP_32 => {
                                let offset = (row_addr + x * 4) & vbe_mem_mask;
                                let b = core.vbe_memory[offset as usize];
                                let g = core.vbe_memory[((offset + 1) & vbe_mem_mask) as usize];
                                let r = core.vbe_memory[((offset + 2) & vbe_mem_mask) as usize];
                                [r, g, b, 0xff]
                            }
                            _ => [0, 0, 0, 0xff],
                        };
                        let dst = ((r * tile_width + c) * 4) as usize;
                        rgba[dst..dst + 4].copy_from_slice(&pixel);
                    }
                }

                if let Some(tile) = core.vga_tile_updated.get_mut(tile_index) {
                    *tile = false;
                }
                sink.graphics_tile_update(
                    rgba,
                    TilePos {
                        x: xc,
                        y: yc,
                        width: tile_width,
                        height: tile_height,
                    },
                );
            }
        }

        core.vga_mem_updated = 0;
        if dimension_changed {
            core.last_xres = width;
            core.last_yres = height;
            core.last_bpp = self.vbe.bpp as u32;
            core.last_fw = 0;
            core.last_fh = 0;
        }

        Refreshed::Frame
    }

    /// Enable PCI presence (`1234:1111`, class `0x030000`) and seed the config
    /// space. Bochs vga.cc `init_pci_conf` + `init_bar_mem`. Config-gated
    /// (`[display] pci_vga`), off by default.
    pub(crate) fn enable_pci(&mut self) {
        self.pci_enabled = true;
        self.init_pci_conf();
    }

    /// Whether the VGA is registered as a PCI device.
    pub fn pci_enabled(&self) -> bool {
        self.pci_enabled
    }

    /// Linear-framebuffer (BAR0) size in bytes.
    pub(crate) fn lfb_size(&self) -> u32 {
        self.vbe_memsize
    }

    /// Inspect the LFB relocation awaiting memory-handler re-registration.
    pub(crate) fn peek_pending_lfb_relocate(&self) -> Option<(u32, u32)> {
        self.pending_lfb_relocate
    }

    /// Commit the LFB relocation after memory-handler re-registration succeeds.
    pub(crate) fn commit_pending_lfb_relocate(&mut self) -> Option<(u32, u32)> {
        let relocate = self.pending_lfb_relocate.take()?;
        self.vbe.base_address = relocate.1;
        Some(relocate)
    }

    /// Inspect the BAR2 relocation awaiting memory-handler re-registration.
    pub(crate) fn peek_pending_mmio_relocate(&self) -> Option<(u32, u32)> {
        self.pending_mmio_base
            .map(|new_base| (self.mmio_base, new_base))
    }

    /// Commit the BAR2 relocation after memory-handler re-registration succeeds.
    pub(crate) fn commit_pending_mmio_relocate(&mut self) -> Option<(u32, u32)> {
        let new_base = self.pending_mmio_base.take()?;
        let relocate = (self.mmio_base, new_base);
        self.mmio_base = new_base;
        Some(relocate)
    }

    /// Returns the desired VGA BAR targets encoded in PCI configuration without
    /// changing handler registration or committing a queued relocation.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_mapping_target(&self) -> VgaSnapshotRestoreTarget {
        if !self.pci_enabled {
            return self.snapshot_v3_committed_mapping_target();
        }

        let lfb_bar = u32::from_le_bytes([
            self.pci_conf[0x10],
            self.pci_conf[0x11],
            self.pci_conf[0x12],
            self.pci_conf[0x13],
        ]);
        let mmio_bar = u32::from_le_bytes([
            self.pci_conf[0x18],
            self.pci_conf[0x19],
            self.pci_conf[0x1a],
            self.pci_conf[0x1b],
        ]);
        VgaSnapshotRestoreTarget {
            lfb_base: lfb_bar & !(self.vbe_memsize - 1),
            mmio_base: mmio_bar & !(PCI_VGA_MMIO_SIZE - 1),
        }
    }

    /// Returns the current handler identity for restore topology capture.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_committed_mapping_target(&self) -> VgaSnapshotRestoreTarget {
        VgaSnapshotRestoreTarget {
            lfb_base: self.vbe.base_address,
            mmio_base: self.mmio_base,
        }
    }

    /// Commits desired BAR identities after the parent has relocated its live
    /// memory handlers from their captured old ranges.
    #[cfg(feature = "std")]
    pub(crate) fn commit_snapshot_v3_mapping_target(&mut self, target: VgaSnapshotRestoreTarget) {
        self.vbe.base_address = target.lfb_base;
        self.mmio_base = target.mmio_base;
        self.pending_lfb_relocate = None;
        self.pending_mmio_base = None;
    }
}

impl crate::pci::PciDevice for StdVga {
    const DEVFUNC: u8 = crate::pci::pci_device(2, 0);
    type WriteEffects = VgaBarChange;

    /// Read the PCI config space. Reads back `0xFFFFFFFF` (no device) when PCI is
    /// disabled, so a gated-off VGA is invisible to enumeration.
    fn pci_read(&self, address: u8, io_len: u8) -> u32 {
        if !self.pci_enabled {
            return 0xFFFF_FFFF;
        }
        let mut value = 0u32;
        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr < 256 {
                value |= (self.pci_conf[addr] as u32) << (i * 8);
            }
        }
        value
    }

    /// Write PCI config space, handling BAR0 (LFB) and BAR2 (MMIO) sizing and
    /// queuing relocation. Mirrors Bochs `pci_write_handler_common` + vga.cc
    /// `pci_write_handler`. The caller must relocate memory handlers, then commit.
    fn pci_write(&mut self, address: u8, mut value: u32, io_len: u8) -> VgaBarChange {
        if !self.pci_enabled {
            return VgaBarChange::default();
        }

        // (base register, size) of the BAR this address falls in, if any.
        let bar: Option<(u8, u32)> = if (0x10..0x14).contains(&address) {
            Some((0x10, self.vbe_memsize)) // BAR0: LFB
        } else if (0x18..0x1C).contains(&address) {
            Some((0x18, PCI_VGA_MMIO_SIZE)) // BAR2: MMIO
        } else {
            None
        };

        let mut bar_change = 0u8;
        if let Some((base_reg, size)) = bar {
            // Size probe: a write of >= 0xfffffff0 must read back the size mask.
            if value >= 0xffff_fff0 {
                let low = self.pci_conf[base_reg as usize] & 0x0f;
                value = (value & !(size - 1)) | (low as u32);
                bar_change = 2; // marks a probe; never commits
            }
        }

        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= 256 {
                break;
            }
            let mut value8 = ((value >> (i * 8)) & 0xff) as u8;
            let oldval = self.pci_conf[addr];
            match bar {
                Some((base_reg, _)) if addr == base_reg as usize => {
                    // Aligned low byte of a MEM BAR: keep the type nibble.
                    value8 = (value8 & 0xf0) | (oldval & 0x0f);
                }
                Some(_) => {} // upper BAR bytes: stored verbatim
                None => match addr {
                    0x0C | 0x0D | 0x3C => {} // cache-line, latency, interrupt-line: writable
                    // Everything else (ids/status/class/header, command, unimplemented
                    // BARs, expansion ROM) is read-only.
                    _ => continue,
                },
            }
            if value8 != oldval {
                bar_change |= 1;
            }
            self.pci_conf[addr] = value8;
        }

        let mut change = VgaBarChange::default();
        if bar_change == 1 {
            if let Some((base_reg, size)) = bar {
                let raw = u32::from_le_bytes([
                    self.pci_conf[base_reg as usize],
                    self.pci_conf[base_reg as usize + 1],
                    self.pci_conf[base_reg as usize + 2],
                    self.pci_conf[base_reg as usize + 3],
                ]);
                let new_base = raw & !(size - 1);
                if base_reg == 0x10 {
                    if new_base != self.vbe.base_address {
                        // Defer the LFB handler move until memory re-registration succeeds.
                        self.pending_lfb_relocate = Some((self.vbe.base_address, new_base));
                        change.lfb = true;
                    }
                } else if new_base != self.mmio_base {
                    self.pending_mmio_base = Some(new_base);
                    change.mmio = true;
                }
            }
        }
        change
    }

}
