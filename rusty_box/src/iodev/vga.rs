//! VGA Display Controller
//!
//! Implements VGA text mode (80x25) for console output.
//! Based on Bochs vgacore.cc and vga.cc, simplified for text mode only.
//!
//! ## Text Mode Memory Layout
//!
//! Text mode uses memory at 0xB8000-0xBFFFF:
//! - Each character is 2 bytes: [character, attribute]
//! - 80 columns × 25 rows × 2 bytes = 4000 bytes per page
//! - Multiple pages can be stored in the 32KB region

#[cfg(not(feature = "std"))]
use alloc::vec;
use alloc::{string::String, vec::Vec};
use core::ffi::c_void;

use crate::{config::BxPhyAddress, memory::BxMemC, Result};

use super::BxDevicesC;

/// VGA text mode memory base address
const VGA_TEXT_MEM_BASE: BxPhyAddress = 0xB8000;
const VGA_TEXT_MEM_SIZE: usize = 0x8000; // 32KB
const VGA_TEXT_MEM_BASE_MONO: BxPhyAddress = 0xB0000;

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
            _ => unreachable!(),
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
            Self::MonoText32k => addr >= VGA_WINDOW_MONO_BASE && addr <= VGA_WINDOW_MONO_END,
            Self::ColorText32k => addr >= VGA_WINDOW_COLOR_BASE && addr <= VGA_WINDOW_COLOR_END,
            Self::Vga64k => addr >= VGA_WINDOW_GRAPHICS_BASE && addr <= VGA_WINDOW_VGA64K_END,
            Self::Ega128k => addr >= VGA_WINDOW_GRAPHICS_BASE && addr <= VGA_WINDOW_GRAPHICS_END,
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
const CRTC_INDEX_MASK: u8 = 0x1F;
const ATTR_INDEX_MASK: u8 = 0x1F;
const SEQ_INDEX_MASK: u8 = 0x07;
const GFX_INDEX_MASK: u8 = 0x0F;

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
    pub(crate) text_buffer: Vec<u8>,
    /// Text snapshot (old state) for comparison
    pub(crate) text_snapshot: Vec<u8>,
    /// Cursor address in text buffer
    pub(crate) cursor_address: u16,
    /// Text mode info
    pub(crate) tm_info: crate::gui::VgaTextModeInfo,
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
    text_memory: Vec<u8>,
    /// Current cursor position (row, col)
    cursor_pos: (usize, usize),
    /// Flag indicating text memory has changed (dirty)
    text_dirty: bool,
    /// Text buffer for GUI updates (new state)
    /// This is extracted from text_memory when update() is called
    text_buffer: Vec<u8>,
    /// Text snapshot for comparison (old state)
    /// Used to detect what changed between updates
    text_snapshot: Vec<u8>,
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

    /// Attribute controller: video_enabled (PAS = Palette Address Source)
    /// Bit 5 of the value written to port 0x3C0 when flip_flop=0
    /// Bochs: s.attribute_ctrl.video_enabled
    video_enabled: bool,

    // =====================================================================
    // Dimension tracking (matching Bochs vgacore.cc s.last_xres etc.)
    // Used to detect when dimension_update needs to be called on the GUI.
    // =====================================================================
    last_xres: u32,
    last_yres: u32,
    last_fw: u32,
    last_fh: u32,
    last_bpp: u32,
}

impl Default for BxVgaC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxVgaC {
    /// Create a new VGA controller
    pub(crate) fn new() -> Self {
        let mut vga = Self {
            crtc_index: 0,
            crtc_regs: [0; 25],
            attr_index: 0,
            attr_flip_flop: false,
            attr_regs: [0; 21],
            seq_index: 0,
            seq_regs: [0; 5],
            graphics_index: 0,
            graphics_regs: [0; 9],
            status_reg: 0x00,
            // Bochs init_standard_vga(): color_emulation=1, enable_ram=1,
            // horiz_sync_pol=1, vert_sync_pol=1, clock_select=0, select_high_bank=0
            // = 0b11000011 = 0xC3
            misc_output: 0xC3,
            text_memory: vec![0; VGA_TEXT_MEM_SIZE],
            cursor_pos: (0, 0),
            text_dirty: false,
            // Bochs keeps text buffers sized for the whole aperture (0x8000 for mapping 2/3).
            text_buffer: vec![0; VGA_TEXT_MEM_SIZE],
            text_snapshot: vec![0; VGA_TEXT_MEM_SIZE],
            vga_mem_updated: 0,
            text_buffer_update: true, // Initial update needed

            probe_handler_calls: 0,
            probe_mapped_writes: 0,
            probe_unmapped_writes: 0,
            probe_first_mapped: None,
            probe_first_unmapped: None,

            // VGA Enable and PEL/DAC registers
            vga_enabled: true, // VGA enabled by default
            pel_mask: 0xFF,    // All palette entries visible
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

            video_enabled: false, // PAS bit, set by 0x3C0 address writes

            last_xres: 0,
            last_yres: 0,
            last_fw: 0,
            last_fh: 0,
            last_bpp: 8, // Bochs: s.last_bpp = 8
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
        tracing::info!("Initializing VGA text mode");

        // Register I/O port handlers
        let vga_ptr = self as *mut BxVgaC as *mut c_void;

        // All VGA write handlers use mask 0x3 (byte+word) matching Bochs vgacore.cc:208-235.
        // Word writes are split into two byte writes in write_port().

        // CRTC registers (mono) (0x3B4-0x3B5)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_CRTC_INDEX_MONO,
            "VGA CRTC Index (mono)",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_CRTC_DATA_MONO,
            "VGA CRTC Data (mono)",
            0x3,
        );

        // CRTC registers (0x3D4-0x3D5)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_CRTC_INDEX,
            "VGA CRTC Index",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_CRTC_DATA,
            "VGA CRTC Data",
            0x3,
        );

        // Status register (0x3DA)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_STATUS,
            "VGA Status",
            0x3,
        );

        // Status register (mono) (0x3BA)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_STATUS_MONO,
            "VGA Status (mono)",
            0x3,
        );

        // Attribute controller (0x3C0-0x3C1)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_ATTRIB_ADDR,
            "VGA Attribute Address",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_ATTRIB_DATA,
            "VGA Attribute Data",
            0x3,
        );

        // Sequencer (0x3C4-0x3C5)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_SEQ_INDEX,
            "VGA Sequencer Index",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_SEQ_DATA,
            "VGA Sequencer Data",
            0x3,
        );

        // Graphics controller (0x3CE-0x3CF)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_GRAPHICS_INDEX,
            "VGA Graphics Index",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_GRAPHICS_DATA,
            "VGA Graphics Data",
            0x3,
        );

        // Misc output READ (0x3CC) - reads the misc output register
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_MISC_OUTPUT,
            "VGA Misc Output Read",
            0x3,
        );

        // Misc output WRITE - CRITICAL for BIOS to set color mode
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_MISC_OUTPUT_WRITE,
            "VGA Misc Output Write",
            0x3,
        );

        // VGA Enable
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_ENABLE,
            "VGA Enable",
            0x3,
        );

        // PEL Mask
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_PEL_MASK,
            "VGA PEL Mask",
            0x3,
        );

        // DAC State Read / PEL Address Read Mode Write
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_DAC_STATE,
            "VGA DAC State",
            0x3,
        );

        // PEL Address Write Mode
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_PEL_ADDR_WRITE,
            "VGA PEL Address Write",
            0x3,
        );

        // PEL Data Register
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_PEL_DATA,
            "VGA PEL Data",
            0x3,
        );

        // EGA compatibility ports (0x3CA, 0x3CB, 0x3CD)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            0x3CA,
            "VGA EGA Compat",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            0x3CB,
            "VGA EGA Compat",
            0x3,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            0x3CD,
            "VGA EGA Compat",
            0x3,
        );

        // Register memory handlers for VGA memory range (0xA0000-0xBFFFF)
        // This matches DEV_register_memory_handlers in vgacore.cc line 177
        let vga_ptr_const = vga_ptr as *const c_void;
        mem.register_memory_handlers(
            vga_ptr_const,
            vga_mem_read_handler,
            vga_mem_write_handler,
            VGA_WINDOW_GRAPHICS_BASE,
            VGA_WINDOW_GRAPHICS_END,
        )?;

        tracing::info!("VGA initialized (80x25 text mode)");
        Ok(())
    }

    /// Reset VGA controller
    pub(crate) fn reset(&mut self) {
        *self = Self::new();
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
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07,
            0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
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
        for i in 0..8 {
            self.pel_data[0x38 + i] = dac_colors[8 + i];
        }

        // Force text buffer refresh
        self.text_buffer_update = true;
        self.vga_mem_updated = 1;
    }

    /// Read from I/O port
    pub(crate) fn read_port(&mut self, port: u16, _io_len: u8) -> u32 {
        // Bochs vgacore.cc:487-494: port gating based on color_emulation
        if port >= 0x3B0 && port <= 0x3BF && self.misc_color_emulation {
            return 0xFF; // mono ports disabled in color mode
        }
        if port >= 0x3D0 && port <= 0x3DF && !self.misc_color_emulation {
            return 0xFF; // color ports disabled in mono mode
        }
        match port {
            VGA_CRTC_INDEX | VGA_CRTC_INDEX_MONO => self.crtc_index as u32,
            VGA_CRTC_DATA | VGA_CRTC_DATA_MONO => {
                if self.crtc_index < 25 {
                    self.crtc_regs[self.crtc_index as usize] as u32
                } else {
                    0
                }
            }
            VGA_STATUS | VGA_STATUS_MONO => {
                // Input Status Register 1 (0x3DA / 0x3BA)
                // Matching Bochs vgacore.cc:501-530
                // bit 0: Display Enable (1 = in blanking period)
                // bit 3: Vertical Retrace (1 = in vertical retrace)
                // Toggle both bits to simulate display cycling through
                // active → hblank → vblank → vretrace phases.
                // VGA BIOS waits for bit 3 transitions (0→1 and 1→0).
                self.status_reg ^= VGA_STATUS_TOGGLE_MASK; // toggle Display Enable and Vert Retrace
                                                           // Reading this port resets the attribute flip-flop (Bochs line 529)
                self.attr_flip_flop = false;
                self.status_reg as u32
            }
            VGA_ATTRIB_ADDR => {
                // Bochs vgacore.cc:534-544: read returns (video_enabled<<5)|address
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
                // Bochs vgacore.cc:546-571: read attribute data register
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

            // Misc Output Write port - write-only, return 0xFF on read
            VGA_MISC_OUTPUT_WRITE => 0xFF,

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

            // EGA compatibility ports - return 0
            0x3CA | 0x3CB | 0x3CD => 0x00,

            _ => {
                0xFF
            }
        }
    }

    /// Write to I/O port
    pub(crate) fn write_port(&mut self, port: u16, value: u32, io_len: u8) {
        // Bochs vgacore.cc:812-817: port gating based on color_emulation
        if port >= 0x3B0 && port <= 0x3BF && self.misc_color_emulation {
            return; // mono ports disabled in color mode
        }
        if port >= 0x3D0 && port <= 0x3DF && !self.misc_color_emulation {
            return; // color ports disabled in mono mode
        }
        // Word writes: split into two byte writes (Bochs vgacore.cc:806-809)
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
            VGA_CRTC_DATA | VGA_CRTC_DATA_MONO => {
                if self.crtc_index < 25 {
                    self.crtc_regs[self.crtc_index as usize] = value;

                    // Update cursor position if cursor location registers changed
                    if self.crtc_index as usize == CRTC_CURSOR_LOC_HIGH {
                        let cursor_addr =
                            ((value as u16) << 8) | (self.crtc_regs[CRTC_CURSOR_LOC_LOW] as u16);
                        self.cursor_pos = (
                            (cursor_addr as usize / BYTES_PER_ROW),
                            (cursor_addr as usize % BYTES_PER_ROW) / BYTES_PER_CHAR,
                        );
                    } else if self.crtc_index as usize == CRTC_CURSOR_LOC_LOW {
                        let cursor_addr =
                            ((self.crtc_regs[CRTC_CURSOR_LOC_HIGH] as u16) << 8) | (value as u16);
                        self.cursor_pos = (
                            (cursor_addr as usize / BYTES_PER_ROW),
                            (cursor_addr as usize % BYTES_PER_ROW) / BYTES_PER_CHAR,
                        );
                    } else if self.crtc_index as usize == CRTC_START_ADDR_HIGH
                           || self.crtc_index as usize == CRTC_START_ADDR_LOW {
                        self.text_buffer_update = true;
                    }
                }
            }
            VGA_ATTRIB_ADDR => {
                // Writing to 0x3C0 toggles flip-flop
                // Bochs vgacore.cc:821-843
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
                    if self.attr_index < 21 {
                        self.attr_regs[self.attr_index as usize] = value;
                    }
                }
                self.attr_flip_flop = !self.attr_flip_flop;
            }
            VGA_ATTRIB_DATA => {
                // Writing to 0x3C1 is not standard, but some code may try
                if self.attr_index < 21 {
                    self.attr_regs[self.attr_index as usize] = value;
                }
            }
            VGA_SEQ_INDEX => {
                self.seq_index = value & SEQ_INDEX_MASK;
            }
            VGA_SEQ_DATA => {
                if self.seq_index < 5 {
                    self.seq_regs[self.seq_index as usize] = value;
                }
            }
            VGA_GRAPHICS_INDEX => {
                self.graphics_index = value & GFX_INDEX_MASK;
            }
            VGA_GRAPHICS_DATA => {
                if self.graphics_index < 9 {
                    let old_value = self.graphics_regs[self.graphics_index as usize];
                    self.graphics_regs[self.graphics_index as usize] = value;

                    // Special handling for Miscellaneous Graphics register
                    // This controls memory_mapping which affects which address range is active
                    if self.graphics_index as usize == GFX_REG_MISC {
                        let old_mapping =
                            (old_value >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK;
                        let new_mapping =
                            (value >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK;
                        if old_mapping != new_mapping {
                            tracing::info!(
                                "VGA memory_mapping changed: {:?} -> {:?} (value: {:#04x} -> {:#04x})",
                                VgaMemoryMapping::from_u8(old_mapping),
                                VgaMemoryMapping::from_u8(new_mapping),
                                old_value,
                                value
                            );
                            self.text_buffer_update = true;
                        }
                    }
                }
            }

            // Misc Output Read port (0x3CC) - also accept writes for compatibility
            VGA_MISC_OUTPUT => {
                self.misc_output = value;
                self.misc_color_emulation = (value & MISC_OUT_COLOR_EMULATION) != 0;
                self.misc_enable_ram = (value & MISC_OUT_ENABLE_RAM) != 0;
                self.misc_clock_select =
                    (value >> MISC_OUT_CLOCK_SEL_SHIFT) & MISC_OUT_CLOCK_SEL_MASK;
                self.misc_select_high_bank = (value & MISC_OUT_HIGH_BANK) != 0;
                self.misc_horiz_sync_pol = (value & MISC_OUT_HORIZ_POL) != 0;
                self.misc_vert_sync_pol = (value & MISC_OUT_VERT_POL) != 0;
            }

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
                tracing::info!(
                    "VGA Misc Output Write: {:#04x} (color_emulation={}, enable_ram={})",
                    value,
                    self.misc_color_emulation,
                    self.misc_enable_ram
                );
            }

            // VGA Enable
            VGA_ENABLE => {
                self.vga_enabled = (value & 0x01) != 0;
                tracing::debug!("VGA Enable: {}", self.vga_enabled);
            }

            // PEL Mask
            VGA_PEL_MASK => {
                self.pel_mask = value;
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
                self.pel_data[self.pel_write_addr as usize][self.pel_write_cycle as usize] = value;
                self.pel_write_cycle += 1;
                if self.pel_write_cycle >= PEL_CYCLES_PER_COLOR {
                    self.pel_write_cycle = 0;
                    self.pel_write_addr = self.pel_write_addr.wrapping_add(1);
                }
            }

            // EGA compatibility ports - ignore writes
            0x3CA | 0x3CB | 0x3CD => {
                // Ignore (EGA compatibility)
            }

            _ => {
            }
        }
    }

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

    /// Get text mode screen contents as a string
    pub(crate) fn get_text_screen(&self) -> String {
        let mut result = String::new();

        // Our text_memory is flat: [char0, attr0, char1, attr1, ...] at offsets
        // (physical_addr & 0x7FFF). For 80x25 mode, each row is 160 bytes.
        // CRTC start address is in character cells (words).
        let start_addr_words = ((self.crtc_regs[CRTC_START_ADDR_HIGH] as u16) << 8)
            | (self.crtc_regs[CRTC_START_ADDR_LOW] as u16);
        let start_address = (start_addr_words as usize) * BYTES_PER_CHAR;

        let mem_mask = VGA_TEXT_MEM_SIZE - 1; // 0x7fff

        for row in 0..TEXT_ROWS {
            let row_base = start_address + row * BYTES_PER_ROW;
            for col in 0..TEXT_COLS {
                let off = (row_base + col * BYTES_PER_CHAR) & mem_mask;
                let ch = self.text_memory.get(off).copied().unwrap_or(0);
                if ch >= 0x20 && ch < 0x7F {
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
            if ch >= 0x20 && ch < 0x7F && ch != b' ' {
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
                if ch >= 0x20 && ch < 0x7F {
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

    /// Update VGA display (matching vgacore.cc:1598-1693)
    /// This processes text mode and prepares data for GUI update
    /// Returns update result if an update is needed
    /// Must be no_std compatible (only uses core + alloc)
    pub(crate) fn update(&mut self) -> Option<VgaUpdateResult> {
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

        // Keep a copy of the previous snapshot for the GUI diff.
        // We'll update `self.text_snapshot` to the new state at the end of this call.
        let old_snapshot = self.text_snapshot.clone();

        // Calculate text mode parameters (matching vgacore.cc:1601-1632)
        let start_addr = ((self.crtc_regs[CRTC_START_ADDR_HIGH] as u16) << 8)
            | (self.crtc_regs[CRTC_START_ADDR_LOW] as u16);
        let start_address = (start_addr << 1) as u16;

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

        let line_compare = 0; // TODO: Calculate from CRTC registers if needed
        let h_panning = self.attr_regs[ATTR_REG_HORIZ_PIXEL_PAN] & ATTR_HPANNING_MASK;
        let v_panning = self.crtc_regs[CRTC_PRESET_ROW_SCAN] & CRTC_PRESET_ROW_MASK;
        let line_graphics = (self.attr_regs[ATTR_REG_MODE_CONTROL] & ATTR_MODE_LINE_GRAPHICS) != 0;
        let split_hpanning =
            (self.attr_regs[ATTR_REG_MODE_CONTROL] & ATTR_MODE_SPLIT_HPANNING) != 0;
        let blink_flags = 0u8; // TODO: Calculate from attribute controller

        // Build palette (matching vgacore.cc:1629-1632)
        let mut actl_palette = [0u8; 16];
        for i in 0..16 {
            actl_palette[i] = self.attr_regs[i] & 0x0f; // Simplified - no pel.mask for now
        }

        // Calculate rows and cols (matching vgacore.cc:1634-1648)
        let mut cols = (self.crtc_regs[CRTC_HORIZ_DISPLAY_END] + 1) as usize;
        let mut msl = (self.crtc_regs[CRTC_MAX_SCAN_LINE] & CRTC_MSL_MASK) as usize;
        let vde = (self.crtc_regs[CRTC_VERT_DISPLAY_END] as usize)
            + (((self.crtc_regs[CRTC_OVERFLOW] & CRTC_OVERFLOW_VDE_BIT8) as usize) << 7)
            + (((self.crtc_regs[CRTC_OVERFLOW] & CRTC_OVERFLOW_VDE_BIT9) as usize) << 3);

        // Workaround for update() calls before VGABIOS init (matching vgacore.cc:1639-1643)
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

        // Calculate cursor address (matching vgacore.cc:1671-1676)
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
        let tm_info = crate::gui::VgaTextModeInfo {
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

        // Compute dimension_update parameters (matching vgacore.cc:1653-1666)
        let c_width = if (self.seq_regs[SEQ_REG_CLOCKING_MODE] & SEQ_CLOCKING_8DOT_CHAR) != 0 {
            8u32
        } else {
            9u32
        };
        // x_dotclockdiv2 = sequencer.reg1 bit 3 (vgacore.cc:938)
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

        // Only signal dimension change when something actually changed (vgacore.cc:1657-1659)
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
        })
    }
}

/// VGA read handler (called from I/O port system)
pub(super) fn vga_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let vga = unsafe { &mut *(this_ptr as *mut BxVgaC) };
    vga.read_port(port, io_len)
}

/// VGA write handler (called from I/O port system)
pub(super) fn vga_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let vga = unsafe { &mut *(this_ptr as *mut BxVgaC) };
    vga.write_port(port, value, io_len);
}

/// VGA memory read handler (called from memory system)
/// Based on bx_vgacore_c::mem_read_handler in vgacore.cc
/// Processes bytes one at a time, matching original implementation
pub(super) fn vga_mem_read_handler(
    addr: crate::config::BxPhyAddress,
    len: u32,
    data: *mut c_void,
    param: *const c_void,
) -> bool {
    if param.is_null() || data.is_null() {
        return false;
    }

    let vga = unsafe { &*(param as *const BxVgaC) };

    // Match Bochs window gating (vgacore.cc:1723..1738):
    // only the selected window maps to VGA memory; others read as 0xff.
    let memory_mapping = VgaMemoryMapping::from_u8(
        (vga.graphics_regs[GFX_REG_MISC] >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK,
    );
    let mut current_addr = addr;
    let mut data_ptr = data as *mut u8;

    for _ in 0..len {
        let val = if memory_mapping.contains_addr(current_addr) {
            let offset = (current_addr - memory_mapping.window_base()) as usize;
            vga.text_memory.get(offset).copied().unwrap_or(0xff)
        } else {
            0xff
        };

        unsafe {
            *data_ptr = val;
            data_ptr = data_ptr.add(1);
        }
        current_addr += 1;
    }

    true
}

/// VGA memory write handler (called from memory system)
/// Based on bx_vgacore_c::mem_write_handler in vgacore.cc
/// Processes bytes one at a time, matching original implementation
pub(super) fn vga_mem_write_handler(
    addr: crate::config::BxPhyAddress,
    len: u32,
    data: *mut c_void,
    param: *const c_void,
) -> bool {
    if param.is_null() || data.is_null() {
        return false;
    }

    let vga = unsafe { &mut *(param as *mut BxVgaC) };
    vga.probe_handler_calls = vga.probe_handler_calls.wrapping_add(1);

    // Match Bochs window gating (vgacore.cc:1826..1842):
    // only the selected window maps to VGA memory; writes outside the window are ignored.
    let memory_mapping = VgaMemoryMapping::from_u8(
        (vga.graphics_regs[GFX_REG_MISC] >> GFX_MISC_MEMORY_MAP_SHIFT) & GFX_MISC_MEMORY_MAP_MASK,
    );
    // Sequencer map mask (reg 2): bits 0-3 select which planes to write.
    // In text mode: plane 0 = characters, plane 1 = attributes, plane 2 = fonts.
    // Only update text_memory when planes 0/1 are being written.
    let map_mask = vga.seq_regs[SEQ_REG_MAP_MASK] & SEQ_MAP_MASK_PLANES;
    let is_text_plane_write = (map_mask & SEQ_MAP_MASK_TEXT_PLANES) != 0;

    let mut current_addr = addr;
    let mut data_ptr = data as *const u8;

    for _ in 0..len {
        let mapped = memory_mapping.contains_addr(current_addr);

        if mapped && is_text_plane_write {
            // Calculate offset relative to the window base.
            let window_base = memory_mapping.window_base();
            let offset = (current_addr - window_base) as usize;
            if offset < vga.text_memory.len() {
                unsafe {
                    let new_val = *data_ptr;
                    vga.probe_mapped_writes = vga.probe_mapped_writes.wrapping_add(1);
                    if vga.probe_first_mapped.is_none() {
                        vga.probe_first_mapped = Some((current_addr, new_val, memory_mapping));
                    }
                    let old_val = vga.text_memory[offset];
                    vga.text_memory[offset] = new_val;
                    if old_val != new_val {
                        vga.text_dirty = true;
                        vga.vga_mem_updated |= 1;
                    }
                    data_ptr = data_ptr.add(1);
                }
            } else {
                unsafe { data_ptr = data_ptr.add(1) };
            }
        } else {
            // Font plane write or unmapped — consume data byte but don't update text buffer
            unsafe {
                if !mapped {
                    let new_val = *data_ptr;
                    vga.probe_unmapped_writes = vga.probe_unmapped_writes.wrapping_add(1);
                    if vga.probe_first_unmapped.is_none() {
                        vga.probe_first_unmapped = Some((current_addr, new_val, memory_mapping));
                    }
                }
                data_ptr = data_ptr.add(1);
            };
        }

        current_addr += 1;
    }

    true
}
