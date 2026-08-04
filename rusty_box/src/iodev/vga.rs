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

use crate::{config::BxPhyAddress, memory::BxMemC, Result};
#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
    SNAPSHOT_SECTION_VERSION,
};


use super::BxDevicesC;

/// VGA text mode information
#[derive(Debug, Clone)]
pub struct VgaTextModeInfo {
    pub(crate) start_address: u16,
    pub(crate) cs_start: u8,
    pub(crate) cs_end: u8,
    pub(crate) line_offset: u16,
    pub(crate) line_compare: u16,
    pub(crate) h_panning: u8,
    pub(crate) v_panning: u8,
    pub(crate) line_graphics: bool,
    pub(crate) split_hpanning: bool,
    pub(crate) blink_flags: u8,
    pub(crate) actl_palette: [u8; 16],
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

/// Shift applied to 6-bit DAC components to reach 8-bit host colour.
/// Bochs: `s.dac_shift = 2` (vgacore.cc init_standard_vga).
const DAC_SHIFT: u8 = 2;

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

/// QEMU-compatible MMIO BAR2 size (4KB)
pub(crate) const PCI_VGA_MMIO_SIZE: u32 = 0x1000;
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

/// VGA update result - contains data needed for GUI update
/// This is returned by update() to allow no_std compatibility
pub(crate) struct VgaUpdateResult {
    /// Whether an update is needed
    pub(crate) needs_update: bool,
    /// Text buffer (new state)
    pub(crate) text_buffer: [u8; VGA_TEXT_MEM_SIZE],
    /// Text snapshot (old state) for comparison
    pub(crate) text_snapshot: [u8; VGA_TEXT_MEM_SIZE],
    /// Cursor address in text buffer
    pub(crate) cursor_address: u16,
    /// Text mode info
    pub(crate) tm_info: VgaTextModeInfo,
    /// Whether dimension_update should be called on the GUI
    pub(crate) dimension_changed: bool,
    /// Pixel width (for dimension_update)
    pub(crate) iwidth: u32,
    /// Pixel height (for dimension_update)
    pub(crate) iheight: u32,
    /// Font height in pixels (for dimension_update)
    pub(crate) fheight: u32,
    /// Font/char width in pixels (for dimension_update)
    pub(crate) fwidth: u32,
    /// The character generator changed this frame; the GUI must re-copy both
    /// charmaps. Bochs signals this to its GUI through `set_text_charmap`,
    /// which sets `bx_gui_c::charmap_updated` and forces a full text redraw.
    pub(crate) charmap_updated: bool,
}

#[cfg(feature = "alloc")]
pub(crate) enum VgaDisplayUpdate {
    Text(VgaUpdateResult),
    Graphics(VgaGraphicsUpdate),
}

#[cfg(feature = "alloc")]
pub(crate) struct VgaGraphicsUpdate {
    pub(crate) dimension_changed: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bpp: u16,
    pub(crate) tiles: Vec<VgaGraphicsTile>,
}

#[cfg(feature = "alloc")]
pub(crate) struct VgaGraphicsTile {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

/// VBE (Bochs VGA Extension) state, matching Bochs `bx_vga_c::vbe`.
#[derive(Debug, Clone)]
struct VbeState {
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
/// atomically, then call [`BxVgaC::commit_snapshot_v3_mapping_target`].
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VgaSnapshotRestoreTarget {
    pub(crate) lfb_base: u32,
    pub(crate) mmio_base: u32,
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

/// VGA controller state
#[derive(Debug)]
pub(crate) struct BxVgaC {
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
    pub(crate) graphics_regs: [u8; 9],
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
    // Bochs-aligned observability (debug-only but always-on, no globals)
    // =====================================================================
    /// Total handler invocations (incremented on every call to vga_mem_write_handler).
    probe_handler_calls: u64,
    /// Count of writes that were accepted by current `memory_mapping` window gating.
    probe_mapped_writes: u64,
    /// Count of writes that were ignored because they fell outside the selected window.
    probe_unmapped_writes: u64,
    /// First mapped write observed: (phys_addr, value, memory_mapping)
    probe_first_mapped: Option<(BxPhyAddress, u8, VgaMemoryMapping)>,
    /// First unmapped write observed: (phys_addr, value, memory_mapping)
    probe_first_unmapped: Option<(BxPhyAddress, u8, VgaMemoryMapping)>,

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
    /// boundary. A full-table republish is requested with `dac_all_dirty`.
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
    /// VBE extension state (DISPI registers, resolution, banking, etc.)
    vbe: VbeState,
    /// DDC monitor (EDID over I2C via VBE_DISPI register 0xB) — Bochs
    /// vga.h bx_ddc_c ddc. Internal I2C state is not snapshotted (Bochs
    /// persists only vbe.ddc_enabled, vga.cc register_state).
    ddc: crate::iodev::ddc::BxDdcC,
    /// Total VBE memory size in bytes (configurable, default 16MB)
    vbe_memsize: u32,
    #[cfg(feature = "alloc")]
    /// Bochs VBE linear framebuffer memory.
    vbe_memory: Vec<u8>,
    #[cfg(feature = "alloc")]
    /// Dirty state for Bochs 16x24 graphics tiles.
    vga_tile_updated: Vec<bool>,
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

    /// PCI configuration space (256 bytes). Only meaningful when `pci_enabled`.
    /// Bochs bx_vga_c::pci_conf. Mirrors `init_pci_conf(0x1234,0x1111,0,0x030000,0,0)`.
    pci_conf: [u8; 256],
    /// Whether this VGA registers as a PCI device (`1234:1111`, class `0300`).
    /// Config-gated (`[display] pci_vga`), default off. Preserved across reset.
    pci_enabled: bool,
    /// Committed BAR2 (VBE MMIO) base, or 0 when unmapped.
    mmio_base: u32,
    /// A BAR0 relocation awaiting successful LFB handler re-registration:
    /// `(old_base, new_base)`.
    pending_lfb_relocate: Option<(u32, u32)>,
    /// A BAR2 relocation awaiting successful MMIO handler registration. Its old
    /// base remains in `mmio_base` until commit.
    pending_mmio_base: Option<u32>,
}

impl Default for BxVgaC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxVgaC {
    /// Create a new VGA controller
    pub(crate) fn new() -> Self {
        let vbe_memsize = 16 << 20;
        let vbe = VbeState::default();
        let num_x_tiles = ((vbe.max_xres as u32 + VGA_X_TILESIZE - 1) / VGA_X_TILESIZE) as u16;
        let num_y_tiles = ((vbe.max_yres as u32 + VGA_Y_TILESIZE - 1) / VGA_Y_TILESIZE) as u16;
        let mut vga = Self {
            ddc: crate::iodev::ddc::BxDdcC::new(),
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

            probe_handler_calls: 0,
            probe_mapped_writes: 0,
            probe_unmapped_writes: 0,
            probe_first_mapped: None,
            probe_first_unmapped: None,

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
            pel_mask: 0xFF, // All palette entries visible
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

            // VBE state (defaults via VbeState::default())
            vbe,
            vbe_memsize,
            #[cfg(feature = "alloc")]
            vbe_memory: vec![0; vbe_memsize as usize],
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
            pci_conf: [0u8; 256],
            pci_enabled: false,
            mmio_base: 0,
            pending_lfb_relocate: None,
            pending_mmio_base: None,
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

    #[cfg(feature = "alloc")]
    /// Summary of VGA memory write activity (for headless debugging).
    pub(crate) fn probe_summary(&self) -> String {
        use core::fmt::Write;
        let mut s = String::new();
        writeln!(
            s,
            "handler_calls={} mapped_writes={} unmapped_writes={}",
            self.probe_handler_calls, self.probe_mapped_writes, self.probe_unmapped_writes
        )
        .ok();
        if let Some((addr, val, mm)) = self.probe_first_mapped {
            writeln!(
                s,
                "first_mapped: addr={:#x} val={:#02x} memory_mapping={:?}",
                addr, val, mm
            )
            .ok();
        } else {
            writeln!(s, "first_mapped: <none>").ok();
        }
        if let Some((addr, val, mm)) = self.probe_first_unmapped {
            writeln!(
                s,
                "first_unmapped: addr={:#x} val={:#02x} memory_mapping={:?}",
                addr, val, mm
            )
            .ok();
        } else {
            writeln!(s, "first_unmapped: <none>").ok();
        }
        s
    }

    /// Initialize VGA device
    pub(crate) fn init(&mut self, io: &mut BxDevicesC, mem: &mut BxMemC) -> Result<()> {
        tracing::debug!("Initializing VGA text mode");

        // Register I/O port handlers
        use super::DeviceId;

        // All VGA write handlers use mask 0x3 (byte+word) matching Bochs vgacore.cc.
        // Word writes are split into two byte writes in write_port().

        // Register all VGA ports with DeviceId::Vga
        let vga_ports: &[(u16, &str)] = &[
            (VGA_CRTC_INDEX_MONO, "VGA CRTC Index (mono)"),
            (VGA_CRTC_DATA_MONO, "VGA CRTC Data (mono)"),
            (VGA_CRTC_INDEX, "VGA CRTC Index"),
            (VGA_CRTC_DATA, "VGA CRTC Data"),
            (VGA_STATUS, "VGA Status"),
            (VGA_STATUS_MONO, "VGA Status (mono)"),
            (VGA_ATTRIB_ADDR, "VGA Attribute Address"),
            (VGA_ATTRIB_DATA, "VGA Attribute Data"),
            (VGA_SEQ_INDEX, "VGA Sequencer Index"),
            (VGA_SEQ_DATA, "VGA Sequencer Data"),
            (VGA_GRAPHICS_INDEX, "VGA Graphics Index"),
            (VGA_GRAPHICS_DATA, "VGA Graphics Data"),
            (VGA_MISC_OUTPUT, "VGA Misc Output Read"),
            (VGA_MISC_OUTPUT_WRITE, "VGA Misc Output Write"),
            (VGA_ENABLE, "VGA Enable"),
            (VGA_PEL_MASK, "VGA PEL Mask"),
            (VGA_DAC_STATE, "VGA DAC State"),
            (VGA_PEL_ADDR_WRITE, "VGA PEL Address Write"),
            (VGA_PEL_DATA, "VGA PEL Data"),
            (VBE_DISPI_IOPORT_INDEX, "Bochs VBE Index"),
            (VBE_DISPI_IOPORT_DATA, "Bochs VBE Data"),
            (0x3CA, "VGA EGA Compat"),
            (0x3CB, "VGA EGA Compat"),
            (0x3CD, "VGA EGA Compat"),
        ];
        for &(port, name) in vga_ports {
            io.register_io_handler(DeviceId::Vga, port, name, 0x3);
        }

        // Register memory handlers for VGA memory range (0xA0000-0xBFFFF)
        // This matches DEV_register_memory_handlers in vgacore.cc line 177
        let device_id = crate::memory::MemoryDeviceId::Vga(self as *mut BxVgaC);
        mem.register_memory_handlers(device_id, VGA_WINDOW_GRAPHICS_BASE, VGA_WINDOW_GRAPHICS_END)?;
        #[cfg(feature = "alloc")]
        {
            let begin = self.vbe.base_address as BxPhyAddress;
            let end = begin + self.vbe_memsize as BxPhyAddress - 1;
            let device_id = crate::memory::MemoryDeviceId::Vga(self as *mut BxVgaC);
            mem.register_memory_handlers(device_id, begin, end)?;
        }

        tracing::debug!("VGA initialized (80x25 text mode)");
        Ok(())
    }

    /// Reset VGA controller
    pub(crate) fn reset(&mut self) {
        // Save state that should persist across reset
        let has_icount_sync = self.has_icount_sync;
        let ips = self.ips;
        let preferred_mode = self.preferred_mode;
        // PCI/BAR state persists across reset (Bochs bx_vga_c::reset only re-applies
        // command/status; the BARs and the committed LFB base survive), so the
        // registered LFB memory handler stays consistent with vbe.base_address.
        let pci_enabled = self.pci_enabled;
        let pci_conf = self.pci_conf;
        let mmio_base = self.mmio_base;
        let lfb_base = self.vbe.base_address;
        *self = Self::new();
        self.has_icount_sync = has_icount_sync;
        self.ips = ips;
        self.preferred_mode = preferred_mode;
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
        self.apply_preferred_mode();
    }

    /// Set the pre-boot VBE mode: raise the DISPI capability ceiling so the guest
    /// may select up to this resolution, and seed the power-on dimensions.
    /// Preserved across `reset()`. Mirrors the DISPI MAX_XRES/MAX_YRES/MAX_BPP
    /// capability registers Bochs exposes (vga.cc). Reallocates the dirty-tile
    /// grid when the ceiling grows.
    pub(crate) fn set_preferred_mode(&mut self, xres: u16, yres: u16, bpp: u16) {
        self.preferred_mode = Some((xres, yres, bpp));
        self.apply_preferred_mode();
    }

    fn apply_preferred_mode(&mut self) {
        let Some((xres, yres, bpp)) = self.preferred_mode else {
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
        if num_x_tiles != self.num_x_tiles || num_y_tiles != self.num_y_tiles {
            self.num_x_tiles = num_x_tiles;
            self.num_y_tiles = num_y_tiles;
            #[cfg(feature = "alloc")]
            {
                self.vga_tile_updated = vec![true; num_x_tiles as usize * num_y_tiles as usize];
            }
        }
    }

    /// Initialize icount-based timing for retrace computation.
    /// Must be called after CPU initialization.
    pub(crate) fn set_icount_sync(&mut self, ips: u64) {
        self.has_icount_sync = true;
        self.ips = if ips > 0 { ips } else { 15_000_000 };
    }

    /// Returns the exact byte length of the standalone VGA v3 section payload.
    #[cfg(feature = "std")]
    pub(crate) fn snapshot_v3_len(&self) -> io::Result<u64> {
        self.validate_snapshot_v3_source()?;

        let text_len = snapshot_v3_usize_len(VGA_TEXT_MEM_SIZE)?;
        let planar_len = snapshot_v3_usize_len(VGA_MEM_SIZE)?;
        let pci_len = snapshot_v3_usize_len(self.pci_conf.len())?;
        let palette_len = checked_snapshot_len_mul(
            snapshot_v3_usize_len(self.pel_data.len())?,
            snapshot_v3_usize_len(usize::from(PEL_CYCLES_PER_COLOR))?,
        )?;
        let vbe_len = snapshot_v3_usize_len(self.vbe_memory.len())?;

        // Section version plus the scalar state written before every array.
        // 90 = 84 + feature_control (u8) + y_doublescan (bool)
        //         + charmap_address1/2 (2 x u16). The extracted charmap buffers
        //         are derived from planar memory and re-extracted on restore.
        let mut len = checked_snapshot_len_add(4, 90)?;
        for array_len in [
            pci_len,
            snapshot_v3_usize_len(self.crtc_regs.len())?,
            snapshot_v3_usize_len(self.attr_regs.len())?,
            snapshot_v3_usize_len(self.seq_regs.len())?,
            snapshot_v3_usize_len(self.graphics_regs.len())?,
            text_len,
            palette_len,
            snapshot_v3_usize_len(self.latch.len())?,
            planar_len,
        ] {
            len = checked_snapshot_len_add(len, 4)?;
            len = checked_snapshot_len_add(len, array_len)?;
        }

        // VBE backing storage is configured, not guest-sized, but its length is
        // encoded as u64 to prevent a format-dependent host-size conversion.
        len = checked_snapshot_len_add(len, 8)?;
        checked_snapshot_len_add(len, vbe_len)
    }

    /// Streams the complete standalone VGA v3 section, including its version.
    ///
    /// Fixed buffers are written straight to the destination; this method never
    /// creates a payload vector or a copy of the framebuffer.
    #[cfg(feature = "std")]
    pub(crate) fn save_snapshot_v3<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.validate_snapshot_v3_source()?;

        writer.write_u32(SNAPSHOT_SECTION_VERSION)?;
        writer.write_u8(self.crtc_index)?;
        writer.write_u8(self.attr_index)?;
        writer.write_bool(self.attr_flip_flop)?;
        writer.write_bool(self.video_enabled)?;
        writer.write_u8(self.seq_index)?;
        writer.write_u8(self.graphics_index)?;
        writer.write_u8(self.status_reg)?;
        writer.write_u8(self.misc_output)?;
        writer.write_bool(self.vga_enabled)?;

        writer.write_u8(self.pel_mask)?;
        writer.write_u8(self.dac_state)?;
        writer.write_u8(self.pel_write_addr)?;
        writer.write_u8(self.pel_read_addr)?;
        writer.write_u8(self.pel_write_cycle)?;
        writer.write_u8(self.pel_read_cycle)?;

        writer.write_bool(self.has_icount_sync)?;
        writer.write_u64(self.ips)?;
        writer.write_u32(self.vbe_memsize)?;
        writer.write_bool(self.preferred_mode.is_some())?;
        let preferred_mode = self.preferred_mode.unwrap_or((0, 0, 0));
        writer.write_u16(preferred_mode.0)?;
        writer.write_u16(preferred_mode.1)?;
        writer.write_u16(preferred_mode.2)?;

        let vbe = VgaSnapshotVbeState::from(&self.vbe);
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

        writer.write_u32(self.ext_start_addr)?;
        let mapping_target = self.snapshot_v3_mapping_target();
        // Bochs registers both in its VGA state list (vgacore.cc register_state:
        // "feature_control" and BXRS_PARAM_BOOL y_doublescan).
        writer.write_u8(self.feature_control as u8)?;
        writer.write_bool(self.y_doublescan)?;
        writer.write_u16(self.charmap_address1)?;
        writer.write_u16(self.charmap_address2)?;
        writer.write_bool(self.ext_y_dblsize)?;
        writer.write_bool(self.pci_enabled)?;
        writer.write_u32(mapping_target.lfb_base)?;
        writer.write_u32(mapping_target.mmio_base)?;

        write_snapshot_u32_len(writer, self.pci_conf.len())?;
        writer.write_bytes(&self.pci_conf)?;
        write_snapshot_u32_len(writer, self.crtc_regs.len())?;
        writer.write_bytes(&self.crtc_regs)?;
        write_snapshot_u32_len(writer, self.attr_regs.len())?;
        writer.write_bytes(&self.attr_regs)?;
        write_snapshot_u32_len(writer, self.seq_regs.len())?;
        writer.write_bytes(&self.seq_regs)?;
        write_snapshot_u32_len(writer, self.graphics_regs.len())?;
        writer.write_bytes(&self.graphics_regs)?;
        write_snapshot_u32_len(writer, self.text_memory.len())?;
        writer.write_bytes(&self.text_memory)?;

        let palette_len = checked_snapshot_len_mul(
            snapshot_v3_usize_len(self.pel_data.len())?,
            snapshot_v3_usize_len(usize::from(PEL_CYCLES_PER_COLOR))?,
        )?;
        writer.write_u32(
            u32::try_from(palette_len)
                .map_err(|_| invalid_vga_snapshot("VGA palette length does not fit u32"))?,
        )?;
        for color in &self.pel_data {
            writer.write_bytes(color)?;
        }

        write_snapshot_u32_len(writer, self.latch.len())?;
        writer.write_bytes(&self.latch)?;
        write_snapshot_u32_len(writer, self.vga_memory.len())?;
        writer.write_bytes(&self.vga_memory)?;
        writer.write_u64(snapshot_v3_usize_len(self.vbe_memory.len())?)?;
        writer.write_bytes(&self.vbe_memory)
    }

    /// Restores one bounded VGA v3 section and returns its desired BAR bases.
    ///
    /// Decoding never changes `vbe.base_address` or `mmio_base`, nor does it
    /// register a memory handler.  The caller must use the returned target only
    /// after the machine-level atomic relocation succeeds.
    #[cfg(feature = "std")]
    pub(crate) fn restore_snapshot_v3<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<VgaSnapshotRestoreTarget> {
        if reader.read_u32()? != SNAPSHOT_SECTION_VERSION {
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

        let pci_len = read_snapshot_u32_len(reader, self.pci_conf.len(), "PCI config")?;
        if pci_len != self.pci_conf.len() {
            return Err(invalid_vga_snapshot("VGA PCI config length mismatch"));
        }
        let mut saved_bar0 = 0u32;
        let mut saved_bar2 = 0u32;
        let expected_bar0_low = self.pci_conf[0x10] & 0x0f;
        let expected_bar2_low = self.pci_conf[0x18] & 0x0f;
        for index in 0..self.pci_conf.len() {
            let saved = reader.read_u8()?;
            let live = self.pci_conf[index];
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
            self.pci_conf[index] = saved;
        }
        if pci_enabled
            && (saved_bar0 & !(self.vbe_memsize - 1) != target.lfb_base
                || saved_bar2 & !(PCI_VGA_MMIO_SIZE - 1) != target.mmio_base)
        {
            return Err(invalid_vga_snapshot(
                "VGA desired BAR target disagrees with PCI configuration",
            ));
        }

        self.crtc_index = crtc_index;
        self.attr_index = attr_index;
        self.attr_flip_flop = attr_flip_flop;
        self.video_enabled = video_enabled;
        self.seq_index = seq_index;
        self.graphics_index = graphics_index;
        self.status_reg = status_reg;
        self.misc_output = misc_output;
        self.vga_enabled = vga_enabled;
        self.pel_mask = pel_mask;
        self.dac_state = dac_state;
        self.pel_write_addr = pel_write_addr;
        self.pel_read_addr = pel_read_addr;
        self.pel_write_cycle = pel_write_cycle;
        self.pel_read_cycle = pel_read_cycle;
        self.vbe.cur_dispi = saved_vbe.cur_dispi;
        self.vbe.max_xres = saved_vbe.max_xres;
        self.vbe.max_yres = saved_vbe.max_yres;
        self.vbe.max_bpp = saved_vbe.max_bpp;
        self.vbe.xres = saved_vbe.xres;
        self.vbe.yres = saved_vbe.yres;
        self.vbe.bpp = saved_vbe.bpp;
        self.vbe.bank = saved_vbe.bank;
        self.vbe.bank_granularity_kb = saved_vbe.bank_granularity_kb;
        self.vbe.enabled = saved_vbe.enabled;
        self.vbe.curindex = saved_vbe.curindex;
        self.vbe.offset_x = saved_vbe.offset_x;
        self.vbe.offset_y = saved_vbe.offset_y;
        self.vbe.virtual_xres = saved_vbe.virtual_xres;
        self.vbe.virtual_yres = saved_vbe.virtual_yres;
        self.vbe.get_capabilities = saved_vbe.get_capabilities;
        self.vbe.dac_8bit = saved_vbe.dac_8bit;
        self.vbe.ddc_enabled = saved_vbe.ddc_enabled;
        self.ext_start_addr = ext_start_addr;
        self.feature_control = feature_control;
        self.y_doublescan = y_doublescan;
        self.charmap_address1 = charmap_address1;
        self.charmap_address2 = charmap_address2;
        // Re-derive the character generators from the restored planar memory
        // (Bochs likewise rebuilds them from state rather than storing glyphs).
        self.update_charmap();
        self.ext_y_dblsize = ext_y_dblsize;
        self.pending_lfb_relocate = None;
        self.pending_mmio_base = None;

        read_snapshot_fixed_array(reader, &mut self.crtc_regs, "CRTC registers")?;
        read_snapshot_fixed_array(reader, &mut self.attr_regs, "attribute registers")?;
        read_snapshot_fixed_array(reader, &mut self.seq_regs, "sequencer registers")?;
        read_snapshot_fixed_array(reader, &mut self.graphics_regs, "graphics registers")?;
        read_snapshot_fixed_array(reader, &mut self.text_memory, "text memory")?;

        let palette_len = read_snapshot_u32_len(
            reader,
            self.pel_data
                .len()
                .checked_mul(usize::from(PEL_CYCLES_PER_COLOR))
                .ok_or_else(|| invalid_vga_snapshot("VGA palette length overflows"))?,
            "DAC palette",
        )?;
        if palette_len
            != self
                .pel_data
                .len()
                .checked_mul(usize::from(PEL_CYCLES_PER_COLOR))
                .ok_or_else(|| invalid_vga_snapshot("VGA palette length overflows"))?
        {
            return Err(invalid_vga_snapshot("VGA DAC palette length mismatch"));
        }
        for color in &mut self.pel_data {
            reader.read_bytes(color)?;
        }

        read_snapshot_fixed_array(reader, &mut self.latch, "graphics latch")?;
        read_snapshot_fixed_array(reader, &mut self.vga_memory, "planar VGA memory")?;

        let vbe_len = reader.read_len(self.vbe_memory.len())?;
        if vbe_len != self.vbe_memory.len() {
            return Err(invalid_vga_snapshot("VBE memory length does not match configuration"));
        }
        reader.read_bytes(&mut self.vbe_memory)?;

        Ok(target)
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

    /// Rebuilds parsed VGA state and invalidates all GUI-facing caches after a
    /// successful whole-machine snapshot restore.
    #[cfg(feature = "std")]
    pub(crate) fn rebuild_snapshot_v3_derived_state(&mut self) -> io::Result<()> {
        self.validate_vbe_snapshot_state(&VgaSnapshotVbeState::from(&self.vbe))?;
        self.validate_snapshot_v3_cache_topology()?;

        self.misc_color_emulation = (self.misc_output & MISC_OUT_COLOR_EMULATION) != 0;
        self.misc_enable_ram = (self.misc_output & MISC_OUT_ENABLE_RAM) != 0;
        self.misc_clock_select =
            (self.misc_output >> MISC_OUT_CLOCK_SEL_SHIFT) & MISC_OUT_CLOCK_SEL_MASK;
        self.misc_select_high_bank = (self.misc_output & MISC_OUT_HIGH_BANK) != 0;
        self.misc_horiz_sync_pol = (self.misc_output & MISC_OUT_HORIZ_POL) != 0;
        self.misc_vert_sync_pol = (self.misc_output & MISC_OUT_VERT_POL) != 0;
        self.seq_chain_four = (self.seq_regs[SEQ_REG_MEMORY_MODE] & 0x08) != 0;
        self.seq_odd_even_dis = (self.seq_regs[SEQ_REG_MEMORY_MODE] & 0x04) != 0;

        let (bpp_multiplier, line_offset) = vga_snapshot_vbe_layout(
            self.vbe.bpp,
            self.vbe.virtual_xres,
        )?;
        self.vbe.bpp_multiplier = bpp_multiplier;
        self.vbe.line_offset = line_offset;
        self.vbe.visible_screen_size = u32::from(line_offset)
            .checked_mul(u32::from(self.vbe.yres))
            .ok_or_else(|| invalid_vga_snapshot("VBE visible-screen size overflows"))?;
        self.vga_mem_mask = if self.vbe.enabled == VBE_DISPI_ENABLED {
            self.vbe_memsize
                .checked_sub(1)
                .ok_or_else(|| invalid_vga_snapshot("VBE memory size is zero"))?
        } else {
            u32::try_from(VGA_MEM_SIZE - 1)
                .map_err(|_| invalid_vga_snapshot("legacy VGA memory mask does not fit u32"))?
        };
        self.ext_offset = vga_snapshot_bank_offset(
            self.vbe.bank[0],
            self.vbe.bank_granularity_kb,
        )?;
        self.ext_read_offset = vga_snapshot_bank_offset(
            self.vbe.bank[1],
            self.vbe.bank_granularity_kb,
        )?;
        self.recompute_vbe_virtual_start();
        self.calculate_retrace_timing();

        let cursor_addr = (usize::from(self.crtc_regs[CRTC_CURSOR_LOC_HIGH]) << 8)
            | usize::from(self.crtc_regs[CRTC_CURSOR_LOC_LOW]);
        self.cursor_pos = (
            cursor_addr / BYTES_PER_ROW,
            (cursor_addr % BYTES_PER_ROW) / BYTES_PER_CHAR,
        );

        self.text_buffer.fill(0);
        self.text_snapshot.fill(0);
        self.text_dirty = true;
        self.text_buffer_update = true;
        self.vga_mem_updated = 1;
        self.last_xres = 0;
        self.last_yres = 0;
        self.last_fw = 0;
        self.last_fh = 0;
        self.last_bpp = 0;
        self.vga_tile_updated.fill(true);
        Ok(())
    }

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_source(&self) -> io::Result<()> {
        self.validate_vbe_snapshot_state(&VgaSnapshotVbeState::from(&self.vbe))?;
        self.validate_snapshot_v3_cache_topology()?;
        if snapshot_v3_usize_len(self.vbe_memory.len())?
            != u64::from(self.vbe_memsize)
        {
            return Err(invalid_vga_snapshot(
                "live VBE backing storage does not match configured size",
            ));
        }
        if self.pel_data.len() > bounds::MAX_SNAPSHOT_COUNT {
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
    ) -> io::Result<()> {
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
        if has_icount_sync != self.has_icount_sync || ips != self.ips {
            return Err(invalid_vga_snapshot("VGA retrace clock configuration mismatch"));
        }
        if vbe_memsize != self.vbe_memsize {
            return Err(invalid_vga_snapshot("VBE memory configuration mismatch"));
        }
        if preferred_mode != self.preferred_mode {
            return Err(invalid_vga_snapshot("VBE preferred-mode configuration mismatch"));
        }
        if pci_enabled != self.pci_enabled {
            return Err(invalid_vga_snapshot("VGA PCI enable configuration mismatch"));
        }
        self.validate_vbe_snapshot_state(vbe)?;
        validate_vga_snapshot_bar_base(target.lfb_base, self.vbe_memsize)?;
        validate_vga_snapshot_bar_base(target.mmio_base, PCI_VGA_MMIO_SIZE)
    }

    #[cfg(feature = "std")]
    fn validate_vbe_snapshot_state(&self, vbe: &VgaSnapshotVbeState) -> io::Result<()> {
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

    #[cfg(feature = "std")]
    fn validate_snapshot_v3_cache_topology(&self) -> io::Result<()> {
        let expected_x_tiles = (u32::from(self.vbe.max_xres)
            .checked_add(VGA_X_TILESIZE - 1)
            .ok_or_else(|| invalid_vga_snapshot("VGA horizontal tile count overflows"))?)
            / VGA_X_TILESIZE;
        let expected_y_tiles = (u32::from(self.vbe.max_yres)
            .checked_add(VGA_Y_TILESIZE - 1)
            .ok_or_else(|| invalid_vga_snapshot("VGA vertical tile count overflows"))?)
            / VGA_Y_TILESIZE;
        if u32::from(self.num_x_tiles) != expected_x_tiles
            || u32::from(self.num_y_tiles) != expected_y_tiles
        {
            return Err(invalid_vga_snapshot("VGA tile topology does not match configuration"));
        }
        let tile_count = expected_x_tiles
            .checked_mul(expected_y_tiles)
            .ok_or_else(|| invalid_vga_snapshot("VGA tile count overflows"))?;
        let tile_count = usize::try_from(tile_count)
            .map_err(|_| invalid_vga_snapshot("VGA tile count does not fit usize"))?;
        if self.vga_tile_updated.len() != tile_count {
            return Err(invalid_vga_snapshot("VGA tile cache capacity mismatch"));
        }
        Ok(())
    }

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

    /// Get current time in microseconds from icount.
    /// Returns a monotonically increasing value based on instructions executed.
    fn current_usec(&self, icount: u64) -> u64 {
        if !self.has_icount_sync {
            return 0;
        }
        if self.ips > 0 {
            (icount as u128 * 1_000_000 / self.ips as u128) as u64
        } else {
            0
        }
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
    pub(crate) fn read_port(&mut self, port: u16, io_len: u8, icount: u64) -> u32 {
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
            let lo = self.read_port(port, 1, icount);
            let hi = self.read_port(port.wrapping_add(1), 1, icount);
            return lo | (hi << 8);
        }
        match port {
            VBE_DISPI_IOPORT_INDEX => self.vbe.curindex as u32,
            VBE_DISPI_IOPORT_DATA => self.vbe_read_index(self.vbe.curindex) as u32,
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
                    let time_usec = self.current_usec(icount);
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
        if port == VBE_DISPI_IOPORT_INDEX {
            self.vbe.curindex = value as u16;
            return;
        }
        if port == VBE_DISPI_IOPORT_DATA {
            self.vbe_write_index(self.vbe.curindex, value as u16);
            return;
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

    #[cfg(feature = "alloc")]
    /// Get text mode screen contents as a string
    pub(crate) fn get_text_screen(&self) -> String {
        let mut result = String::new();

        // Our text_memory is flat: [char0, attr0, char1, attr1, ...] at offsets
        // (physical_addr & 0x7FFF). For 80x25 mode, each row is 160 bytes.
        // CRTC start address is in character cells (words).
        // Bochs renderers read the per-frame latch (s.CRTC.start_addr), not the
        // live registers, so a mid-frame write cannot tear the picture.
        let start_addr_words = self.crtc_start_addr;
        let start_address = (start_addr_words as usize) * BYTES_PER_CHAR;

        let mem_mask = VGA_TEXT_MEM_SIZE - 1; // 0x7fff

        for row in 0..TEXT_ROWS {
            let row_base = start_address + row * BYTES_PER_ROW;
            for col in 0..TEXT_COLS {
                let off = (row_base + col * BYTES_PER_CHAR) & mem_mask;
                let ch = self.text_memory.get(off).copied().unwrap_or(0);
                if (0x20..0x7F).contains(&ch) {
                    result.push(ch as char);
                } else if ch == 0 {
                    result.push(' ');
                } else {
                    result.push('?');
                }
            }
            // Trim trailing spaces
            let trimmed = result.trim_end_matches(' ');
            let trim_len = trimmed.len();
            result.truncate(trim_len);
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

    fn recompute_vbe_virtual_start(&mut self) {
        let mut virtual_start = self.vbe.offset_y as u32 * self.vbe.line_offset as u32;
        if self.vbe.bpp != VBE_DISPI_BPP_4 {
            virtual_start = virtual_start
                .wrapping_add(self.vbe.offset_x as u32 * self.vbe.bpp_multiplier as u32);
        } else {
            virtual_start = virtual_start.wrapping_add((self.vbe.offset_x as u32) >> 3);
        }
        self.vbe.virtual_start = virtual_start & self.vga_mem_mask;
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
        let shift = if self.vbe.dac_8bit { 0 } else { 2 };
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
    fn update_legacy_graphics(&mut self) -> Option<VgaGraphicsUpdate> {
        if self.vga_mem_updated == 0 {
            return None;
        }

        let (width, height) = self.determine_screen_dimensions();
        if width == 0 || height == 0 {
            return None;
        }
        // Bochs vgacore.cc update(): the graphics branch also bails out through
        // skip_update() once the dimensions are known.
        if self.skip_update() {
            return None;
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
        let mut tiles = Vec::new();

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
                let mut rgba = vec![0u8; (tile_width * tile_height * 4) as usize];
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
                tiles.push(VgaGraphicsTile {
                    x: xc,
                    y: yc,
                    width: tile_width,
                    height: tile_height,
                    rgba,
                });
            }
        }

        self.vga_mem_updated = 0;
        Some(VgaGraphicsUpdate {
            dimension_changed,
            width,
            height,
            bpp: 8,
            tiles,
        })
    }

    #[cfg(feature = "alloc")]
    fn update_vbe_graphics(&mut self) -> Option<VgaGraphicsUpdate> {
        if self.vbe.enabled == 0 {
            return None;
        }

        let width = self.vbe.xres as u32;
        let height = self.vbe.yres as u32;
        if width == 0 || height == 0 {
            return None;
        }

        let dimension_changed = width != self.last_xres
            || height != self.last_yres
            || self.vbe.bpp as u32 != self.last_bpp;
        if dimension_changed {
            self.redraw_area(0, 0, width, height);
        } else if self.vga_mem_updated == 0 {
            return None;
        }

        let vbe_mem_mask = self.vbe_memsize.saturating_sub(1);
        self.vbe.virtual_start &= vbe_mem_mask;

        let pitch = self.vbe.line_offset as u32;
        let mut tiles = Vec::new();

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
                let mut rgba = vec![0u8; (tile_width * tile_height * 4) as usize];

                for r in 0..tile_height {
                    let y = yc + r;
                    let row_addr = self.vbe.virtual_start.wrapping_add(y * pitch) & vbe_mem_mask;
                    for c in 0..tile_width {
                        let x = xc + c;
                        let pixel = match self.vbe.bpp {
                            VBE_DISPI_BPP_4 => {
                                let dac =
                                    self.get_vga_pixel(x as u16, y as u16, row_addr, 0xffff, false);
                                self.dac_index_to_rgba(dac)
                            }
                            VBE_DISPI_BPP_8 => {
                                let offset = (row_addr + x) & vbe_mem_mask;
                                let dac = self.vbe_memory[offset as usize];
                                self.dac_index_to_rgba(dac)
                            }
                            VBE_DISPI_BPP_15 => {
                                let offset = (row_addr + x * 2) & vbe_mem_mask;
                                let lo = self.vbe_memory[offset as usize] as u16;
                                let hi =
                                    self.vbe_memory[((offset + 1) & vbe_mem_mask) as usize] as u16;
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
                                let lo = self.vbe_memory[offset as usize] as u16;
                                let hi =
                                    self.vbe_memory[((offset + 1) & vbe_mem_mask) as usize] as u16;
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
                                let b = self.vbe_memory[offset as usize];
                                let g = self.vbe_memory[((offset + 1) & vbe_mem_mask) as usize];
                                let r = self.vbe_memory[((offset + 2) & vbe_mem_mask) as usize];
                                [r, g, b, 0xff]
                            }
                            VBE_DISPI_BPP_32 => {
                                let offset = (row_addr + x * 4) & vbe_mem_mask;
                                let b = self.vbe_memory[offset as usize];
                                let g = self.vbe_memory[((offset + 1) & vbe_mem_mask) as usize];
                                let r = self.vbe_memory[((offset + 2) & vbe_mem_mask) as usize];
                                [r, g, b, 0xff]
                            }
                            _ => [0, 0, 0, 0xff],
                        };
                        let dst = ((r * tile_width + c) * 4) as usize;
                        rgba[dst..dst + 4].copy_from_slice(&pixel);
                    }
                }

                if let Some(tile) = self.vga_tile_updated.get_mut(tile_index) {
                    *tile = false;
                }
                tiles.push(VgaGraphicsTile {
                    x: xc,
                    y: yc,
                    width: tile_width,
                    height: tile_height,
                    rgba,
                });
            }
        }

        self.vga_mem_updated = 0;
        if dimension_changed {
            self.last_xres = width;
            self.last_yres = height;
            self.last_bpp = self.vbe.bpp as u32;
            self.last_fw = 0;
            self.last_fh = 0;
        }

        Some(VgaGraphicsUpdate {
            dimension_changed,
            width,
            height,
            bpp: self.vbe.bpp,
            tiles,
        })
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
    /// the values already shifted from the 6-bit DAC to 8-bit like Bochs's
    /// `dac_shift` of 2.
    pub(crate) fn take_dac_palette_changes(&mut self) -> impl Iterator<Item = (u8, u8, u8, u8)> + '_ {
        let any = core::mem::take(&mut self.dac_any_dirty);
        (0..PEL_COLOR_COUNT).filter_map(move |i| {
            if !any || !core::mem::take(&mut self.dac_dirty[i]) {
                return None;
            }
            let entry = self.pel_data[i];
            Some((
                i as u8,
                entry[0] << DAC_SHIFT,
                entry[1] << DAC_SHIFT,
                entry[2] << DAC_SHIFT,
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

    #[cfg(feature = "alloc")]
    pub(crate) fn update(&mut self) -> Option<VgaDisplayUpdate> {
        if self.vbe.enabled != 0 {
            return self.update_vbe_graphics().map(VgaDisplayUpdate::Graphics);
        }

        let graphics_alpha = (self.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) != 0;
        let memory_mapping = VgaMemoryMapping::from_u8(
            (self.graphics_regs[GFX_REG_MISC] >> GFX_MISC_MEMORY_MAP_SHIFT)
                & GFX_MISC_MEMORY_MAP_MASK,
        );
        let is_text_mode = (!graphics_alpha)
            && (memory_mapping == VgaMemoryMapping::MonoText32k
                || memory_mapping == VgaMemoryMapping::ColorText32k);

        if is_text_mode {
            self.update_text_mode().map(VgaDisplayUpdate::Text)
        } else {
            self.update_legacy_graphics()
                .map(VgaDisplayUpdate::Graphics)
        }
    }

    #[cfg(not(feature = "alloc"))]
    pub(crate) fn update(&mut self) -> Option<VgaUpdateResult> {
        self.update_text_mode()
    }

    /// Update VGA display (matching vgacore.cc)
    /// This processes text mode and prepares data for GUI update
    /// Returns update result if an update is needed
    /// Must be no_std compatible (only uses core + alloc)
    fn update_text_mode(&mut self) -> Option<VgaUpdateResult> {
        // Check if we're in text mode (match Bochs `vgacore.cc` semantics).
        //
        // In Bochs, `s.graphics_ctrl.graphics_alpha` and `s.graphics_ctrl.memory_mapping`
        // are derived from the Graphics Controller register index 0x06:
        //   graphics_alpha = value & 0x01
        //   memory_mapping = (value >> 2) & 0x03
        //
        // Text mode when `graphics_alpha == 0`. Memory mapping selects which aperture
        // is active (B0000 vs B8000 for mono/color text).
        let graphics_alpha = (self.graphics_regs[GFX_REG_MISC] & GFX_MISC_GRAPHICS_ALPHA) != 0;
        let memory_mapping = VgaMemoryMapping::from_u8(
            (self.graphics_regs[GFX_REG_MISC] >> GFX_MISC_MEMORY_MAP_SHIFT)
                & GFX_MISC_MEMORY_MAP_MASK,
        );
        let is_text_mode = (!graphics_alpha)
            && (memory_mapping == VgaMemoryMapping::MonoText32k
                || memory_mapping == VgaMemoryMapping::ColorText32k);

        if !is_text_mode {
            return None;
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
            return None;
        }

        // Keep a copy of the previous snapshot for the GUI diff.
        // We'll update `self.text_snapshot` to the new state at the end of this call.
        let old_snapshot = self.text_snapshot.clone();

        // Calculate text mode parameters (matching vgacore.cc). The start
        // address comes from the per-frame latch, as in Bochs's renderers
        // (`tm_info.start_address = (s.CRTC.start_addr << 1)`).
        let start_addr = self.crtc_start_addr;
        let start_address = start_addr << 1;

        let cs_start = self.crtc_regs[CRTC_CURSOR_START] & CRTC_CURSOR_START_MASK;
        let cs_end = self.crtc_regs[CRTC_CURSOR_END] & CRTC_CURSOR_END_MASK;

        // Line offset: CRTC offset register is in dwords; our text buffer is interleaved
        // (char+attr pairs), so each row = crtc_offset * 4 bytes.
        // Bochs planar uses * 2 (one byte per char in plane 0); we use * 4 for interleaved.
        let mut line_offset = (self.crtc_regs[CRTC_OFFSET] as u16) * 4;
        if line_offset == 0 {
            // Default to 80 columns * 2 bytes per char (interleaved)
            line_offset = (TEXT_COLS * BYTES_PER_CHAR) as u16;
        }

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

        // Calculate rows and cols (matching vgacore.cc)
        let mut cols = (self.crtc_regs[CRTC_HORIZ_DISPLAY_END] + 1) as usize;
        let mut msl = (self.crtc_regs[CRTC_MAX_SCAN_LINE] & CRTC_MSL_MASK) as usize;
        let vde = (self.crtc_regs[CRTC_VERT_DISPLAY_END] as usize)
            + (((self.crtc_regs[CRTC_OVERFLOW] & CRTC_OVERFLOW_VDE_BIT8) as usize) << 7)
            + (((self.crtc_regs[CRTC_OVERFLOW] & CRTC_OVERFLOW_VDE_BIT9) as usize) << 3);

        // Workaround for update() calls before VGABIOS init (matching vgacore.cc)
        if cols == 1 || msl == 0 {
            cols = TEXT_COLS;
        }
        if msl == 0 {
            msl = 15;
        }

        let rows = if msl > 0 {
            (vde + 1) / (msl + 1)
        } else {
            TEXT_ROWS
        };
        let rows = rows.min(TEXT_ROWS); // Cap at 25 rows

        // Calculate cursor address (matching vgacore.cc)
        let cursor_addr = ((self.crtc_regs[CRTC_CURSOR_LOC_HIGH] as u16) << 8)
            | (self.crtc_regs[CRTC_CURSOR_LOC_LOW] as u16);
        let cursor_address = cursor_addr * 2; // Convert to byte offset

        // Validate cursor address
        let max_addr = start_address + (line_offset * rows as u16);
        let cursor_address = if cursor_address < start_address || cursor_address > max_addr {
            0x7fff // Invalid cursor
        } else {
            cursor_address
        };

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

        // Always return update result if in text mode (original always calls text_update_common).
        // The GUI will compare old/new to determine what actually changed.
        let needs_update = self.vga_mem_updated > 0;

        // Prepare new state for the GUI.
        let new_buffer = self.text_buffer.clone();

        // Update internal snapshot after preparing the return values.
        if self.vga_mem_updated > 0 {
            self.text_snapshot[..visible_size].copy_from_slice(&self.text_buffer[..visible_size]);
            self.vga_mem_updated = 0;
            self.text_dirty = false;
        }

        // Compute dimension_update parameters (matching vgacore.cc)
        let c_width = if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_8DOT_CHAR) != 0 {
            8u32
        } else {
            9u32
        };
        // x_dotclockdiv2 = sequencer.reg1 bit 3 (vgacore.cc)
        let x_dotclockdiv2 =
            (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_DOTCLOCKDIV2) != 0;
        let c_width = if x_dotclockdiv2 {
            c_width << 1
        } else {
            c_width
        };
        let i_width = c_width * cols as u32;
        let i_height = (vde + 1) as u32;
        let fh = (msl + 1) as u32;

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
        }

        Some(VgaUpdateResult {
            needs_update,
            text_buffer: new_buffer,
            text_snapshot: old_snapshot,
            cursor_address,
            tm_info,
            dimension_changed,
            iwidth: i_width,
            iheight: i_height,
            fheight: fh,
            fwidth: c_width,
            charmap_updated,
        })
    }
}

/// VGA memory read handler (called from memory system)
/// Based on bx_vgacore_c::mem_read / mem_read_handler in vgacore.cc
/// Implements read mode 0 (return selected plane) and read mode 1 (color compare).
/// Loads latch register on every read.
impl BxVgaC {
    #[cfg(feature = "alloc")]
    fn vbe_mem_read_byte(&mut self, addr: BxPhyAddress) -> u8 {
        let offset = if addr >= self.vbe.base_address as BxPhyAddress {
            addr - self.vbe.base_address as BxPhyAddress
        } else if addr < 0xB0000 {
            self.vbe.bank[1] as BxPhyAddress
                * ((self.vbe.bank_granularity_kb as BxPhyAddress) << 10)
                + (addr & 0xffff)
        } else {
            return 0;
        };

        self.vbe_memory.get(offset as usize).copied().unwrap_or(0)
    }

    #[cfg(feature = "alloc")]
    fn vbe_mem_write_byte(&mut self, addr: BxPhyAddress, value: u8) {
        let offset = if addr >= self.vbe.base_address as BxPhyAddress {
            addr - self.vbe.base_address as BxPhyAddress
        } else if addr < 0xB0000 {
            self.vbe.bank[0] as BxPhyAddress
                * ((self.vbe.bank_granularity_kb as BxPhyAddress) << 10)
                + (addr & 0xffff)
        } else {
            return;
        };

        let Some(slot) = self.vbe_memory.get_mut(offset as usize) else {
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
        self.mark_tile_updated(x_tile, y_tile);
    }

    pub(crate) fn mem_read(&mut self, addr: BxPhyAddress, len: u32, data: &mut [u8]) -> bool {
        // BAR2 (VBE MMIO) window is registered as a VGA memory handler; route it
        // to the dispi MMIO handler rather than the framebuffer/legacy path.
        if self.is_mmio_addr(addr) {
            return self.vbe_mmio_read(addr, len, data);
        }
        for (i, current_addr) in (addr..(addr + len as u64)).enumerate() {
            if let Some(byte) = data.get_mut(i) {
                #[cfg(feature = "alloc")]
                {
                    if self.vbe.enabled != 0 && self.vbe.bpp != VBE_DISPI_BPP_4 {
                        *byte = self.vbe_mem_read_byte(current_addr);
                        continue;
                    }
                    if current_addr >= self.vbe.base_address as BxPhyAddress {
                        let offset = current_addr - self.vbe.base_address as BxPhyAddress;
                        if self.seq_chain_four && offset < 0x40000 {
                            // Bochs vga.cc mem_read: chain-4 LFB accesses go
                            // straight to bx_vgacore_c::mem_read(offset) with the
                            // raw offset — the full 256KB is addressable. Wrapping
                            // to 128KB and re-entering at the legacy window base
                            // re-applied window gating (mapping 1 returns 0xFF past
                            // 64KB and 128-256KB aliased downward).
                            *byte = vga_mem_read_byte(self, offset);
                        } else {
                            *byte = 0xff;
                        }
                        continue;
                    }
                }
                *byte = vga_mem_read_byte(self, current_addr);
            }
        }
        true
    }

    /// VGA memory write handler (called from memory system)
    /// Based on bx_vgacore_c::mem_write / mem_write_handler in vgacore.cc
    /// Implements all 4 write modes with full planar memory support.
    pub(crate) fn mem_write(&mut self, addr: BxPhyAddress, len: u32, data: &[u8]) -> bool {
        if self.is_mmio_addr(addr) {
            return self.vbe_mmio_write(addr, len, data);
        }
        self.probe_handler_calls = self.probe_handler_calls.wrapping_add(1);
        for (i, current_addr) in (addr..(addr + len as u64)).enumerate() {
            if let Some(&value) = data.get(i) {
                #[cfg(feature = "alloc")]
                {
                    if self.vbe.enabled != 0 && self.vbe.bpp != VBE_DISPI_BPP_4 {
                        self.vbe_mem_write_byte(current_addr, value);
                        continue;
                    }
                    if current_addr >= self.vbe.base_address as BxPhyAddress {
                        let offset = current_addr - self.vbe.base_address as BxPhyAddress;
                        if self.seq_chain_four && offset < 0x40000 {
                            // Bochs vga.cc mem_write: chain-4 LFB writes go
                            // straight to bx_vgacore_c::mem_write(offset, value)
                            // with the raw offset (full 256KB), not wrapped to
                            // 128KB through the legacy window.
                            vga_mem_write_byte(self, offset, value);
                        }
                        continue;
                    }
                }
                vga_mem_write_byte(self, current_addr, value);
            }
        }
        true
    }
}

fn vga_storage_get(vga: &BxVgaC, index: usize) -> u8 {
    #[cfg(feature = "alloc")]
    if vga.vbe.enabled != 0 && vga.vbe.bpp == VBE_DISPI_BPP_4 {
        return vga.vbe_memory.get(index).copied().unwrap_or(0);
    }

    vga.vga_memory.get(index).copied().unwrap_or(0)
}

fn vga_storage_set(vga: &mut BxVgaC, index: usize, value: u8) {
    #[cfg(feature = "alloc")]
    if vga.vbe.enabled != 0 && vga.vbe.bpp == VBE_DISPI_BPP_4 {
        if let Some(slot) = vga.vbe_memory.get_mut(index) {
            *slot = value;
        }
        if let Some(slot) = vga.vga_memory.get_mut(index) {
            *slot = value;
        }
        return;
    }

    if let Some(slot) = vga.vga_memory.get_mut(index) {
        *slot = value;
    }
}

/// Read a single byte from VGA memory. Matches Bochs vgacore.cc `mem_read`.
fn vga_mem_read_byte(vga: &mut BxVgaC, addr: BxPhyAddress) -> u8 {
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
fn vga_mem_write_byte(vga: &mut BxVgaC, addr: BxPhyAddress, value: u8) {
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

    // Update probe counters
    vga.probe_mapped_writes = vga.probe_mapped_writes.wrapping_add(1);
    if vga.probe_first_mapped.is_none() {
        let mm = VgaMemoryMapping::from_u8(memory_mapping);
        vga.probe_first_mapped = Some((addr, value, mm));
    }

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
// When PCI is enabled the BAR2 window is registered as a memory handler on BAR2
// commit; mem_read/mem_write detect the MMIO range (is_mmio_addr) and route to
// vbe_mmio_read / vbe_mmio_write. BAR0 is the linear framebuffer.

/// Result of a PCI config write: which BAR (if any) queued a new base for
/// transactional memory-handler relocation.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct VgaBarChange {
    pub lfb: bool,
    pub mmio: bool,
}

impl BxVgaC {
    /// Enable PCI presence (`1234:1111`, class `0x030000`) and seed the config
    /// space. Bochs vga.cc `init_pci_conf` + `init_bar_mem`. Config-gated
    /// (`[display] pci_vga`), off by default.
    pub(crate) fn enable_pci(&mut self) {
        self.pci_enabled = true;
        self.init_pci_conf();
    }

    /// Whether the VGA is registered as a PCI device.
    pub(crate) fn pci_enabled(&self) -> bool {
        self.pci_enabled
    }

    /// Linear-framebuffer (BAR0) size in bytes.
    pub(crate) fn lfb_size(&self) -> u32 {
        self.vbe_memsize
    }

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

    /// Read the PCI config space. Reads back `0xFFFFFFFF` (no device) when PCI is
    /// disabled, so a gated-off VGA is invisible to enumeration.
    pub(crate) fn pci_read(&self, address: u8, io_len: u8) -> u32 {
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
    pub(crate) fn pci_write(&mut self, address: u8, mut value: u32, io_len: u8) -> VgaBarChange {
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

    /// Whether `addr` falls in the committed BAR2 MMIO window.
    pub(crate) fn is_mmio_addr(&self, addr: BxPhyAddress) -> bool {
        self.pci_enabled
            && self.mmio_base != 0
            && addr >= self.mmio_base as BxPhyAddress
            && addr < self.mmio_base as BxPhyAddress + PCI_VGA_MMIO_SIZE as BxPhyAddress
    }

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
    pub(crate) fn vbe_mmio_read(&mut self, addr: BxPhyAddress, len: u32, data: &mut [u8]) -> bool {
        let offset = (addr & 0xFFF) as u32;
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

        true
    }

    /// MMIO write handler for BAR2.
    ///
    /// Translates MMIO offset writes into VBE register writes.
    /// Matches Bochs `bx_vga_c::vbe_mmio_write_handler`.
    pub(crate) fn vbe_mmio_write(&mut self, addr: BxPhyAddress, len: u32, data: &[u8]) -> bool {
        let offset = (addr & 0xFFF) as u32;

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
                return true;
            }
        };

        if offset >= PCI_VGA_BOCHS_OFFSET && offset < PCI_VGA_BOCHS_OFFSET + PCI_VGA_BOCHS_SIZE {
            let reg_offset = offset - PCI_VGA_BOCHS_OFFSET;
            let index = (reg_offset >> 1) as u16;
            self.vbe.curindex = index;
            self.vbe_write_index(index, value as u16);
        }

        true
    }

    /// Handle a VBE data-port write (port 0x01CF or MMIO-dispatched).
    ///
    /// Matches the Bochs `vbe_write` / `vbe_write_handler` logic.
    fn vbe_write_index(&mut self, index: u16, value16: u16) {
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
                    let mut nb = (self.vbe_memsize >> 10) / self.vbe.bank_granularity_kb as u32;
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
                    self.ext_offset =
                        self.vbe.bank[0] as u32 * ((self.vbe.bank_granularity_kb as u32) << 10);
                    self.ext_read_offset =
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
                    self.ext_offset = 0;
                    self.ext_read_offset = 0;
                    self.vga_mem_mask = self.vbe_memsize.saturating_sub(1);
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
                        if self.vbe_memory.len() != self.vbe_memsize as usize {
                            self.vbe_memory.resize(self.vbe_memsize as usize, 0);
                        }
                        if (value16 & VBE_DISPI_NOCLEARMEM) == 0 {
                            self.vbe_memory.fill(0);
                        }
                        self.redraw_area(0, 0, self.vbe.xres as u32, self.vbe.yres as u32);
                    }
                    #[cfg(not(feature = "alloc"))]
                    if (value16 & VBE_DISPI_NOCLEARMEM) == 0 {
                        self.vga_memory.fill(0);
                    }

                    if self.vbe.bpp != VBE_DISPI_BPP_4 {
                        self.last_bpp = self.vbe.bpp as u32;
                        self.last_fh = 0;
                    }
                } else if (value16 & VBE_DISPI_ENABLED) == 0 && self.vbe.enabled != 0 {
                    // Disabling VBE mode — return to legacy VGA
                    self.text_buffer_update = true;
                    self.last_yres = 0;
                    self.ext_offset = 0;
                    self.ext_read_offset = 0;
                    self.vga_mem_mask = (VGA_MEM_SIZE - 1) as u32;
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
                    self.ext_offset = 0;
                    self.ext_read_offset = 0;
                }

                // Handle 8-bit DAC mode change
                let new_dac_8bit = (value16 & VBE_DISPI_8BIT_DAC) != 0;
                if new_dac_8bit != self.vbe.dac_8bit {
                    if new_dac_8bit {
                        for i in 0..256 {
                            self.pel_data[i][0] <<= 2;
                            self.pel_data[i][1] <<= 2;
                            self.pel_data[i][2] <<= 2;
                        }
                    } else {
                        for i in 0..256 {
                            self.pel_data[i][0] >>= 2;
                            self.pel_data[i][1] >>= 2;
                            self.pel_data[i][2] >>= 2;
                        }
                    }
                    self.vbe.dac_8bit = new_dac_8bit;
                    needs_update = true;
                }
            }
            VBE_DISPI_INDEX_X_OFFSET => {
                self.vbe.offset_x = value16;
                self.recompute_vbe_virtual_start();
                needs_update = true;
            }
            VBE_DISPI_INDEX_Y_OFFSET => {
                self.vbe.offset_y = value16;
                self.recompute_vbe_virtual_start();
                needs_update = true;
            }
            VBE_DISPI_INDEX_VIRT_WIDTH => {
                let new_width = value16;
                let new_height = if self.vbe.bpp != VBE_DISPI_BPP_4 {
                    (self.vbe_memsize / self.vbe.bpp_multiplier as u32) / new_width as u32
                } else {
                    (self.vbe_memsize << 1) / new_width as u32
                };
                let (final_width, final_height) = if new_height as u16 >= self.vbe.yres {
                    (new_width, new_height as u16)
                } else {
                    // Cannot fit: recalculate width for yres
                    let h = self.vbe.yres;
                    let w = if self.vbe.bpp != VBE_DISPI_BPP_4 {
                        (self.vbe_memsize / self.vbe.bpp_multiplier as u32) / h as u32
                    } else {
                        (self.vbe_memsize << 1) / h as u32
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
                self.recompute_vbe_virtual_start();
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
            self.vga_mem_updated = 1;
            #[cfg(feature = "alloc")]
            self.redraw_area(0, 0, self.vbe.xres as u32, self.vbe.yres as u32);
        }
    }
}

#[cfg(feature = "std")]
fn invalid_vga_snapshot(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn snapshot_v3_usize_len(len: usize) -> io::Result<u64> {
    let len = u64::try_from(len)
        .map_err(|_| invalid_vga_snapshot("VGA buffer length does not fit u64"))?;
    if len > bounds::MAX_SNAPSHOT_SECTION_LEN {
        return Err(invalid_vga_snapshot(
            "VGA buffer length exceeds snapshot section bound",
        ));
    }
    Ok(len)
}

#[cfg(feature = "std")]
fn write_snapshot_u32_len<W: Write>(writer: &mut W, len: usize) -> io::Result<()> {
    let len = snapshot_v3_usize_len(len)?;
    writer.write_u32(
        u32::try_from(len)
            .map_err(|_| invalid_vga_snapshot("VGA fixed-buffer length does not fit u32"))?,
    )
}

#[cfg(feature = "std")]
fn read_snapshot_u32_len<R: Read>(
    reader: &mut SnapshotReader<R>,
    maximum: usize,
    _description: &'static str,
) -> io::Result<usize> {
    reader.read_count(maximum)
}

#[cfg(feature = "std")]
fn read_snapshot_fixed_array<R: Read, const N: usize>(
    reader: &mut SnapshotReader<R>,
    bytes: &mut [u8; N],
    description: &'static str,
) -> io::Result<()> {
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
fn vga_snapshot_vbe_layout(bpp: u16, virtual_xres: u16) -> io::Result<(u8, u16)> {
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
fn vga_snapshot_bank_offset(bank: u16, bank_granularity_kb: u16) -> io::Result<u32> {
    u32::from(bank)
        .checked_mul(
            u32::from(bank_granularity_kb)
                .checked_mul(1024)
                .ok_or_else(|| invalid_vga_snapshot("VBE bank granularity overflows"))?,
        )
        .ok_or_else(|| invalid_vga_snapshot("VBE bank offset overflows"))
}

#[cfg(feature = "std")]
fn validate_vga_snapshot_bar_base(base: u32, span: u32) -> io::Result<()> {
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
    #[cfg(feature = "std")]
    use std::io::Cursor;


    fn write_vbe(vga: &mut BxVgaC, index: u16, value: u16) {
        vga.write_port(VBE_DISPI_IOPORT_INDEX, index as u32, 2);
        vga.write_port(VBE_DISPI_IOPORT_DATA, value as u32, 2);
    }

    fn pci_vga() -> BxVgaC {
        let mut vga = BxVgaC::new();
        vga.enable_pci();
        vga
    }

    #[test]
    fn pci_disabled_is_invisible_to_enumeration() {
        let vga = BxVgaC::new();
        assert!(!vga.pci_enabled());
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
        assert_eq!(vga.vbe.base_address, 0xE000_0000);

        assert_eq!(
            vga.commit_pending_lfb_relocate(),
            Some((0xE000_0000, 0xE800_0000))
        );
        assert_eq!(vga.vbe.base_address, 0xE800_0000);
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
        assert!(vga.is_mmio_addr(0xF000_0500));
        assert!(!vga.is_mmio_addr(0xF010_0500));

        assert_eq!(
            vga.commit_pending_mmio_relocate(),
            Some((0xF000_0000, 0xF010_0000))
        );
        assert!(!vga.is_mmio_addr(0xF000_0500));
        assert!(vga.is_mmio_addr(0xF010_0500));
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
        assert!(vga.pci_enabled());
        assert_eq!(vga.pci_read(0x00, 4), 0x1111_1234);
        assert_eq!(vga.pci_read(0x10, 4), 0xE800_0008); // BAR persists
        assert_eq!(vga.pci_read(0x04, 1) & 0x03, 0x03); // command re-applied
    }

    #[test]
    fn preferred_mode_raises_caps_reallocs_tiles_and_survives_reset() {
        let mut vga = BxVgaC::new();
        let default_x_tiles = vga.num_x_tiles;

        // 1920 exceeds the built-in 1600 cap, forcing a cap raise + tile regrow.
        vga.set_preferred_mode(1920, 1080, 32);

        assert!(vga.vbe.max_xres >= 1920);
        assert!(vga.vbe.max_yres >= 1080);
        assert_eq!(vga.vbe.xres, 1920);
        assert_eq!(vga.vbe.yres, 1080);
        assert_eq!(vga.vbe.bpp, 32);
        assert!(vga.num_x_tiles > default_x_tiles);
        assert_eq!(
            vga.num_x_tiles,
            ((vga.vbe.max_xres as u32).div_ceil(VGA_X_TILESIZE)) as u16
        );
        assert_eq!(
            vga.vga_tile_updated.len(),
            vga.num_x_tiles as usize * vga.num_y_tiles as usize
        );

        // Reset re-defaults the device but must re-apply the preferred mode.
        vga.reset();
        assert!(vga.vbe.max_xres >= 1920);
        assert_eq!(vga.vbe.xres, 1920);
        assert_eq!(vga.vbe.yres, 1080);
    }

    #[test]
    fn preferred_mode_never_lowers_default_caps() {
        let mut vga = BxVgaC::new();
        let (dx, dy) = (vga.vbe.max_xres, vga.vbe.max_yres);

        // A small mode must not shrink the built-in capability ceiling.
        vga.set_preferred_mode(800, 600, 16);

        assert_eq!(vga.vbe.max_xres, dx);
        assert_eq!(vga.vbe.max_yres, dy);
        assert_eq!(vga.vbe.xres, 800);
        assert_eq!(vga.vbe.yres, 600);
    }

    #[test]
    fn vbe_io_ports_program_mode_and_lfb_update_returns_rgba_tile() {
        let mut vga = BxVgaC::new();

        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 2);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 2);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_32);
        write_vbe(
            &mut vga,
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED,
        );

        vga.mem_write(
            VBE_DISPI_LFB_PHYSICAL_ADDRESS as BxPhyAddress,
            4,
            &[0x11, 0x22, 0x33, 0x44],
        );

        let Some(VgaDisplayUpdate::Graphics(update)) = vga.update() else {
            panic!("expected VBE graphics update");
        };
        assert_eq!(update.width, 2);
        assert_eq!(update.height, 2);
        assert_eq!(update.bpp, 32);
        let tile = update
            .tiles
            .iter()
            .find(|tile| tile.x == 0 && tile.y == 0)
            .expect("missing origin tile");
        assert_eq!(&tile.rgba[0..4], &[0x33, 0x22, 0x11, 0xff]);
    }

    #[test]
    fn vbe_4bpp_bank_write_offsets_legacy_vga_memory() {
        let mut vga = BxVgaC::new();
        vga.seq_chain_four = true;
        vga.seq_odd_even_dis = true;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);

        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[0x5a]);

        assert_eq!(vga.vga_memory[0], 0);
        assert_eq!(vga.vga_memory[0x10000], 0x5a);
    }

    #[test]
    fn vbe_4bpp_chain_four_bank_write_updates_vbe_backing() {
        let mut vga = BxVgaC::new();
        vga.seq_chain_four = true;
        vga.seq_odd_even_dis = true;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);

        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[0x5a]);

        assert_eq!(vga.vbe_memory[0x10000], 0x5a);
    }

    #[test]
    fn vbe_4bpp_planar_bank_write_is_addressable() {
        let mut vga = BxVgaC::new();
        vga.seq_odd_even_dis = true;
        vga.seq_regs[SEQ_REG_MAP_MASK] = 0x01;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;
        vga.graphics_regs[GFX_REG_BIT_MASK] = 0xff;
        vga.graphics_regs[GFX_REG_READ_MAP_SELECT] = 0;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);

        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[0x3c]);
        let mut byte = [0];
        vga.mem_read(VGA_WINDOW_GRAPHICS_BASE, 1, &mut byte);

        assert_eq!(byte[0], 0x3c);
    }

    #[test]
    fn vbe_4bpp_read_bank_is_independent_from_write_bank() {
        let mut vga = BxVgaC::new();
        vga.seq_chain_four = true;
        vga.seq_odd_even_dis = true;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, VBE_DISPI_BANK_WR | 1);
        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[0x7b]);

        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, VBE_DISPI_BANK_WR);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, VBE_DISPI_BANK_RD | 1);

        let mut byte = [0];
        vga.mem_read(VGA_WINDOW_GRAPHICS_BASE, 1, &mut byte);

        assert_eq!(byte[0], 0x7b);
    }

    #[test]
    fn disabling_vbe_clears_legacy_bank_offset() {
        let mut vga = BxVgaC::new();
        vga.seq_chain_four = true;
        vga.seq_odd_even_dis = true;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;

        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_4);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BANK, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);

        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[0xa5]);

        assert_eq!(vga.vga_memory[0], 0xa5);
    }

    #[test]
    fn vbe_8bpp_dac_change_redraws_existing_tile() {
        let mut vga = BxVgaC::new();

        write_vbe(&mut vga, VBE_DISPI_INDEX_XRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_YRES, 1);
        write_vbe(&mut vga, VBE_DISPI_INDEX_BPP, VBE_DISPI_BPP_8);
        write_vbe(&mut vga, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED);
        vga.pel_data[5] = [0x3f, 0, 0];
        vga.mem_write(VBE_DISPI_LFB_PHYSICAL_ADDRESS as BxPhyAddress, 1, &[5]);

        let Some(VgaDisplayUpdate::Graphics(first)) = vga.update() else {
            panic!("expected first graphics update");
        };
        assert_eq!(&first.tiles[0].rgba[0..4], &[0xfc, 0x00, 0x00, 0xff]);

        vga.write_port(VGA_PEL_ADDR_WRITE, 5, 1);
        vga.write_port(VGA_PEL_DATA, 0, 1);
        vga.write_port(VGA_PEL_DATA, 0x3f, 1);
        vga.write_port(VGA_PEL_DATA, 0, 1);

        let Some(VgaDisplayUpdate::Graphics(second)) = vga.update() else {
            panic!("expected palette-only graphics update");
        };
        assert_eq!(&second.tiles[0].rgba[0..4], &[0x00, 0xfc, 0x00, 0xff]);
    }

    #[test]
    fn legacy_chain_four_graphics_update_returns_palette_rgba_tile() {
        let mut vga = BxVgaC::new();
        vga.vga_enabled = true;
        vga.video_enabled = true;
        vga.seq_regs[SEQ_REG_RESET] = 0x03;
        vga.seq_regs[SEQ_REG_MEMORY_MODE] = 0x0e;
        vga.seq_chain_four = true;
        vga.seq_odd_even_dis = true;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;
        vga.graphics_regs[GFX_REG_GRAPHICS_MODE] = 2 << 5;
        vga.crtc_regs[CRTC_HORIZ_DISPLAY_END] = 0;
        vga.crtc_regs[CRTC_VERT_DISPLAY_END] = 0;
        vga.crtc_regs[CRTC_VERT_BLANK_START] = 0;
        vga.crtc_regs[CRTC_MODE_CONTROL] = 0x40;
        vga.pel_data[5] = [0x3f, 0, 0];

        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[5]);

        let Some(VgaDisplayUpdate::Graphics(update)) = vga.update() else {
            panic!("expected legacy graphics update");
        };
        let tile = update
            .tiles
            .iter()
            .find(|tile| tile.x == 0 && tile.y == 0)
            .expect("missing origin tile");
        assert_eq!(&tile.rgba[0..4], &[0xfc, 0x00, 0x00, 0xff]);
    }
    #[test]
    fn legacy_graphics_register_change_redraws_without_memory_write() {
        let mut vga = BxVgaC::new();
        vga.vga_enabled = true;
        vga.video_enabled = true;
        vga.seq_regs[SEQ_REG_RESET] = 0x03;
        vga.seq_regs[SEQ_REG_MEMORY_MODE] = 0x0e;
        vga.seq_chain_four = true;
        vga.seq_odd_even_dis = true;
        vga.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        // Attribute controller mode control bit 0 must agree with the graphics
        // controller, or Bochs skip_update() treats it as a mode set in flight.
        vga.attr_regs[0x10] |= 0x01;
        vga.graphics_regs[GFX_REG_GRAPHICS_MODE] = 2 << 5;
        vga.crtc_regs[CRTC_HORIZ_DISPLAY_END] = 0;
        vga.crtc_regs[CRTC_VERT_DISPLAY_END] = 0;
        vga.crtc_regs[CRTC_VERT_BLANK_START] = 0;
        vga.crtc_regs[CRTC_MODE_CONTROL] = 0x40;

        vga.mem_write(VGA_WINDOW_GRAPHICS_BASE, 1, &[5]);
        assert!(matches!(vga.update(), Some(VgaDisplayUpdate::Graphics(_))));
        assert!(vga.update().is_none());

        vga.write_port(VGA_CRTC_INDEX, CRTC_OFFSET as u32, 1);
        vga.write_port(VGA_CRTC_DATA, 1, 1);

        assert!(matches!(vga.update(), Some(VgaDisplayUpdate::Graphics(_))));
    }

    #[test]
    fn vbe_virtual_offset_wraps_and_redraws() {
        let mut vga = BxVgaC::new();

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

        let wrapped_start = vga.vbe.virtual_start as usize;
        assert!(wrapped_start < vga.vbe_memory.len());
        vga.vbe_memory[wrapped_start..wrapped_start + 4].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        let Some(VgaDisplayUpdate::Graphics(update)) = vga.update() else {
            panic!("expected wrapped VBE graphics update");
        };
        let tile = update
            .tiles
            .iter()
            .find(|tile| tile.x == 0 && tile.y == 0)
            .expect("missing origin tile");
        assert_eq!(&tile.rgba[0..4], &[0x33, 0x22, 0x11, 0xff]);
    }

    // ---- Finding #5: write to 0x3CC (Misc Output *read* port) is ignored ----
    // Bochs vgacore.cc write: `case 0x03cc: /* Graphics 1 Position (EGA) */ // ignore`.
    // The real Misc Output write port is 0x3C2.
    #[test]
    fn misc_output_write_to_0x3cc_is_ignored() {
        let mut vga = BxVgaC::new();

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
        assert_eq!(vga.read_port(VGA_MISC_OUTPUT, 1, 0), 0xAB);
    }

    // ---- Finding #6a: Sequencer index is stored unmasked; out-of-range DATA
    // writes are no-ops (Bochs vgacore.cc write: `default:` case does nothing) ----
    #[test]
    fn sequencer_out_of_range_index_data_write_is_noop() {
        let mut vga = BxVgaC::new();
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
        let mut vga = BxVgaC::new();
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

        let data = vga.read_port(VGA_CRTC_DATA, 1, 0);
        assert_eq!(
            data, 0x33,
            "0x3D5 must read back latch[read_map_select], not CR2"
        );
    }

    // ---- Finding #6c: Graphics Controller index is stored unmasked; out-of-range
    // DATA writes are no-ops (Bochs vgacore.cc write: `default:` case does nothing) ----
    #[test]
    fn graphics_out_of_range_index_data_write_is_noop() {
        let mut vga = BxVgaC::new();
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
        let mut vga = BxVgaC::new();
        vga.write_port(VGA_CRTC_INDEX, CRTC_OVERFLOW as u32, 1);
        vga.crtc_regs[CRTC_OVERFLOW] = 0x5A;
        let word = vga.read_port(VGA_CRTC_INDEX, 2, 0);
        assert_eq!(word & 0xFF, CRTC_OVERFLOW as u32, "low byte = index reg");
        assert_eq!((word >> 8) & 0xFF, 0x5A, "high byte = data reg");
    }

    // Bochs vgacore.cc write cases 0x03ba/0x03da: `feature_control = value & 0x08`;
    // read case 0x03ca returns it; read case 0x03db returns 0.
    #[test]
    fn feature_control_round_trips_via_3da_and_3ca() {
        let mut vga = BxVgaC::new();
        assert_eq!(vga.read_port(0x3CA, 1, 0), 0x00, "reset value is 0");

        vga.write_port(VGA_STATUS, 0xFF, 1);
        assert_eq!(
            vga.read_port(0x3CA, 1, 0),
            0x08,
            "only bit 3 of a 0x3DA write is retained"
        );

        // The 0x3BA alias is gated off while in color emulation — Bochs
        // vgacore.cc write_handler returns early for 0x3b0-0x3bf when
        // misc_output.color_emulation is set — so it must NOT clear the value.
        vga.write_port(VGA_STATUS_MONO, 0x00, 1);
        assert_eq!(
            vga.read_port(0x3CA, 1, 0),
            0x08,
            "mono-port write must be ignored in color emulation mode"
        );

        // Writing 0 through the active (color) port does clear it.
        vga.write_port(VGA_STATUS, 0x00, 1);
        assert_eq!(vga.read_port(0x3CA, 1, 0), 0x00);

        // 0x3DB is the high byte of a 16-bit read from 0x3DA: Bochs returns 0.
        assert_eq!(vga.read_port(0x3DB, 1, 0), 0x00);
    }

    // Bochs vgacore.cc keeps sequencer registers as decomposed fields, so a
    // read-back only exposes the retained bits (write case 0x03c5 / read case
    // 0x03c5). Reset register 0's falling edge also clears char-map select.
    #[test]
    fn sequencer_registers_mask_on_store_like_bochs() {
        let mut vga = BxVgaC::new();
        let write_seq = |vga: &mut BxVgaC, index: u32, value: u32| {
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
        let mut vga = BxVgaC::new();

        // Two distinct glyph patterns at the plane-2 bytes for offsets
        // 0x0000 (map index 0) and 0x4000 (map index 1).
        vga.vga_memory[2] = 0xA5; // byte 0 of the map at address 0x0000
        vga.vga_memory[6] = 0x3C; // byte 1 of the same map
        vga.vga_memory[(0x4000usize << 2) + 2] = 0x5A; // byte 0 of the map at 0x4000

        // A non-zero maximum-scan-line is required before Bochs applies the
        // selection, so program CRTC 9 first.
        vga.write_port(VGA_CRTC_INDEX, CRTC_MAX_SCAN_LINE as u32, 1);
        vga.write_port(VGA_CRTC_DATA, 0x0F, 1);

        // Sequencer register 3 = 0 selects charmap A = B = offset 0x0000.
        vga.write_port(VGA_SEQ_INDEX, 3, 1);
        vga.write_port(VGA_SEQ_DATA, 0x00, 1);
        assert_eq!(vga.charmap_address1, 0x0000);
        assert_eq!(vga.charmap_address2, 0x0000);
        assert_ne!(vga.vga_mem_updated & VGA_MEM_UPDATED_CHARMAP, 0);

        vga.update_charmap();
        assert_eq!(vga.charmap(0)[0], 0xA5, "plane-2 byte 0");
        assert_eq!(vga.charmap(0)[1], 0x3C, "plane-2 byte 1 (stride 4)");
        assert_eq!(
            vga.charmap(1)[0],
            0xA5,
            "equal addresses must publish the same glyphs to both maps"
        );

        // Register 3 = 0x04 selects map B = index 1 -> offset 0x4000.
        vga.write_port(VGA_SEQ_INDEX, 3, 1);
        vga.write_port(VGA_SEQ_DATA, 0x04, 1);
        assert_eq!(vga.charmap_address1, 0x0000);
        assert_eq!(vga.charmap_address2, 0x4000);
        vga.update_charmap();
        assert_eq!(vga.charmap(0)[0], 0xA5, "map 0 unchanged");
        assert_eq!(vga.charmap(1)[0], 0x5A, "map 1 now reads the 0x4000 glyphs");

        // A sequencer reset (reset1 falling edge) clears the selection.
        vga.write_port(VGA_SEQ_INDEX, 0, 1);
        vga.write_port(VGA_SEQ_DATA, 0x03, 1);
        vga.write_port(VGA_SEQ_DATA, 0x00, 1);
        assert_eq!(vga.charmap_address1, 0);
        assert_eq!(vga.charmap_address2, 0);
    }

    // Bochs vgacore.cc write case 0x03c0 data-write mode: per-register bit masks.
    #[test]
    fn attribute_registers_mask_on_store_like_bochs() {
        let mut vga = BxVgaC::new();
        let write_attr = |vga: &mut BxVgaC, index: u32, value: u32| {
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
        let mut vga = BxVgaC::new();
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
        let mut vga = BxVgaC::new();
        vga.attr_index = 5;
        vga.attr_regs[5] = 0x11;
        vga.write_port(VGA_ATTRIB_DATA, 0xFF, 1);
        assert_eq!(vga.attr_regs[5], 0x11, "0x3C1 write must not modify attr regs");
        assert_eq!(vga.read_port(VGA_MISC_OUTPUT_WRITE, 1, 0), 0x00, "0x3C2 read = 0");
    }

    // ---- Finding #7: CR11 bit 7 write-protects CRTC registers 0-7 ----
    // Bochs vgacore.cc write: when `CRTC.reg[0x11] & 0x80` is set, writes to
    // CRTC indices 0x00-0x06 are dropped and a write to 0x07 updates only bit 4.
    #[test]
    fn crtc_write_protect_locks_registers_0_to_7() {
        let mut vga = BxVgaC::new();

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

        source.seq_regs[SEQ_REG_MEMORY_MODE] = 0x04;
        source.graphics_regs[GFX_REG_MISC] =
            GFX_MISC_GRAPHICS_ALPHA | (VgaMemoryMapping::Vga64k as u8) << GFX_MISC_MEMORY_MAP_SHIFT;
        source.graphics_regs[GFX_REG_READ_MAP_SELECT] = 2;
        source.vga_memory[0x24 * 4 + 2] = 0xA1;
        source.text_memory[0x42] = b'V';
        source.vbe_memory[0x1234] = 0xB2;
        source.latch = [0x11, 0x22, 0x33, 0x44];
        source.status_reg = 0xA0;
        source.write_port(VGA_PEL_ADDR_WRITE, 0x4D, 1);
        source.write_port(VGA_PEL_DATA, 0x12, 1);
        source.write_port(VGA_PEL_DATA, 0x23, 1);
        source.write_port(VGA_PEL_DATA, 0x34, 1);

        let mut saved = Vec::new();
        source.save_snapshot_v3(&mut saved).unwrap();
        assert_eq!(source.snapshot_v3_len().unwrap(), saved.len() as u64);

        let mut restored = pci_vga();
        restored.pci_write(0x10, 0xD000_0000, 4);
        restored.commit_pending_lfb_relocate();
        restored.pci_write(0x18, 0xF100_0000, 4);
        restored.commit_pending_mmio_relocate();
        let live_mapping = restored.snapshot_v3_committed_mapping_target();
        restored.vga_memory[0x24 * 4 + 2] = 0;
        restored.text_memory[0x42] = 0;
        restored.vbe_memory[0x1234] = 0;
        restored.latch = [0; 4];
        restored.status_reg = 0;
        restored.text_dirty = false;
        restored.text_buffer_update = false;
        restored.vga_mem_updated = 0;
        restored.text_buffer.fill(0xFF);
        restored.text_snapshot.fill(0xFF);
        restored.last_xres = 1;
        restored.last_yres = 1;
        restored.last_fw = 1;
        restored.last_fh = 1;
        restored.last_bpp = 1;
        restored.vga_tile_updated.fill(false);

        let mut reader = SnapshotReader::new(Cursor::new(saved.clone()), saved.len() as u64).unwrap();
        let target = restored.restore_snapshot_v3(&mut reader).unwrap();
        reader.finish_exact().unwrap();

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
        assert!(restored.text_dirty);
        assert!(restored.text_buffer_update);
        assert_eq!(restored.vga_mem_updated, 1);
        assert!(restored.vga_tile_updated.iter().all(|dirty| *dirty));
        assert!(restored.text_buffer.iter().all(|byte| *byte == 0));
        assert!(restored.text_snapshot.iter().all(|byte| *byte == 0));
        assert_eq!(
            (
                restored.last_xres,
                restored.last_yres,
                restored.last_fw,
                restored.last_fh,
                restored.last_bpp,
            ),
            (0, 0, 0, 0, 0)
        );

        let mut value = [0];
        restored.mem_read(target.lfb_base as BxPhyAddress + 0x1234, 1, &mut value);
        assert_eq!(value, [0xB2], "VBE backing memory must survive restore");

        restored.write_port(VGA_DAC_STATE, 0x4D, 1);
        assert_eq!(restored.read_port(VGA_PEL_DATA, 1, 0), 0x12);
        assert_eq!(restored.read_port(VGA_PEL_DATA, 1, 0), 0x23);
        assert_eq!(restored.read_port(VGA_PEL_DATA, 1, 0), 0x34);
        restored.write_port(VGA_CRTC_INDEX, 0x22, 1);
        assert_eq!(restored.read_port(VGA_CRTC_DATA, 1, 0), 0x33);
        assert_eq!(restored.read_port(VGA_STATUS, 1, 0), 0xA9);

        write_vbe(&mut restored, VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
        restored.mem_read(VGA_WINDOW_GRAPHICS_BASE + 0x24, 1, &mut value);
        assert_eq!(value, [0xA1], "planar memory must survive restore");
        assert_eq!(restored.get_text_memory()[0x42], b'V');
    }

    #[cfg(feature = "std")]
    #[test]
    fn vga_snapshot_rejects_oversized_pci_config_length() {
        let source = BxVgaC::new();
        let mut saved = Vec::new();
        source.save_snapshot_v3(&mut saved).unwrap();
        // The PCI config length immediately follows the fixed 84-byte scalar prefix.
        saved[88..92].copy_from_slice(&257u32.to_le_bytes());

        let mut restored = BxVgaC::new();
        let mut reader = SnapshotReader::new(Cursor::new(saved.clone()), saved.len() as u64).unwrap();
        let error = restored.restore_snapshot_v3(&mut reader).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }
}
