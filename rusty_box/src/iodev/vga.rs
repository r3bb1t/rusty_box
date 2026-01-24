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

use core::ffi::c_void;
use alloc::vec::Vec;

use crate::{
    config::BxPhyAddress,
    memory::BxMemC,
    Result,
};

use super::{BxDevicesC, IoReadHandlerT, IoWriteHandlerT};

/// VGA text mode memory base address
const VGA_TEXT_MEM_BASE: BxPhyAddress = 0xB8000;
const VGA_TEXT_MEM_SIZE: usize = 0x8000; // 32KB

/// VGA I/O ports
const VGA_CRTC_INDEX: u16 = 0x3D4;
const VGA_CRTC_DATA: u16 = 0x3D5;
const VGA_STATUS: u16 = 0x3DA;
const VGA_ATTRIB_ADDR: u16 = 0x3C0;
const VGA_ATTRIB_DATA: u16 = 0x3C1;
const VGA_MISC_OUTPUT: u16 = 0x3CC;
const VGA_SEQ_INDEX: u16 = 0x3C4;
const VGA_SEQ_DATA: u16 = 0x3C5;
const VGA_GRAPHICS_INDEX: u16 = 0x3CE;
const VGA_GRAPHICS_DATA: u16 = 0x3CF;

/// CRTC register indices
const CRTC_CURSOR_START: u8 = 0x0A;
const CRTC_CURSOR_END: u8 = 0x0B;
const CRTC_CURSOR_LOC_HIGH: u8 = 0x0E;
const CRTC_CURSOR_LOC_LOW: u8 = 0x0F;

/// Text mode dimensions
const TEXT_COLS: usize = 80;
const TEXT_ROWS: usize = 25;
const BYTES_PER_CHAR: usize = 2;
const BYTES_PER_ROW: usize = TEXT_COLS * BYTES_PER_CHAR;

/// VGA update result - contains data needed for GUI update
/// This is returned by update() to allow no_std compatibility
pub(crate) struct VgaUpdateResult {
    /// Whether an update is needed
    pub needs_update: bool,
    /// Text buffer (new state)
    pub text_buffer: Vec<u8>,
    /// Text snapshot (old state) for comparison
    pub text_snapshot: Vec<u8>,
    /// Cursor address in text buffer
    pub cursor_address: u16,
    /// Text mode info
    pub tm_info: crate::gui::VgaTextModeInfo,
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
    graphics_regs: [u8; 9],
    /// Status register value
    status_reg: u8,
    /// Misc output register
    misc_output: u8,
    /// Text mode memory buffer
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
            misc_output: 0x67, // Color mode, 80x25 text
            text_memory: vec![0; VGA_TEXT_MEM_SIZE],
            cursor_pos: (0, 0),
            text_dirty: false,
            text_buffer: vec![0; TEXT_COLS * TEXT_ROWS * BYTES_PER_CHAR],
            text_snapshot: vec![0; TEXT_COLS * TEXT_ROWS * BYTES_PER_CHAR],
            vga_mem_updated: 0,
            text_buffer_update: true, // Initial update needed
        };

        // Initialize CRTC registers for 80x25 text mode
        vga.crtc_regs[0] = 0x5F; // Horizontal total
        vga.crtc_regs[1] = 0x4F; // Horizontal display end
        vga.crtc_regs[2] = 0x50; // Start horizontal blanking
        vga.crtc_regs[3] = 0x82; // End horizontal blanking
        vga.crtc_regs[4] = 0x55; // Start horizontal retrace
        vga.crtc_regs[5] = 0x81; // End horizontal retrace
        vga.crtc_regs[6] = 0xBF; // Vertical total
        vga.crtc_regs[7] = 0x1F; // Overflow
        vga.crtc_regs[8] = 0x00; // Preset row scan
        vga.crtc_regs[9] = 0x4F; // Maximum scan line
        vga.crtc_regs[10] = 0x0D; // Cursor start (scan line)
        vga.crtc_regs[11] = 0x0E; // Cursor end (scan line)
        vga.crtc_regs[12] = 0x00; // Start address high
        vga.crtc_regs[13] = 0x00; // Start address low
        vga.crtc_regs[14] = 0x00; // Cursor location high
        vga.crtc_regs[15] = 0x00; // Cursor location low
        vga.crtc_regs[16] = 0x9C; // Vertical retrace start
        vga.crtc_regs[17] = 0x8E; // Vertical retrace end
        vga.crtc_regs[18] = 0x8F; // Vertical display end
        vga.crtc_regs[19] = 0x28; // Offset
        vga.crtc_regs[20] = 0x1F; // Underline location
        vga.crtc_regs[21] = 0x96; // Vertical blank start
        vga.crtc_regs[22] = 0xB9; // Vertical blank end
        vga.crtc_regs[23] = 0xA3; // Mode control
        vga.crtc_regs[24] = 0xFF; // Line compare

        // Initialize sequencer
        vga.seq_regs[0] = 0x03; // Reset
        vga.seq_regs[1] = 0x00; // Clocking mode
        vga.seq_regs[2] = 0x03; // Map mask
        vga.seq_regs[3] = 0x00; // Character map select
        vga.seq_regs[4] = 0x02; // Memory mode

        // Initialize graphics controller
        vga.graphics_regs[0] = 0x00; // Set/Reset
        vga.graphics_regs[1] = 0x00; // Enable Set/Reset
        vga.graphics_regs[2] = 0x00; // Color Compare
        vga.graphics_regs[3] = 0x00; // Data Rotate
        vga.graphics_regs[4] = 0x00; // Read Map Select
        vga.graphics_regs[5] = 0x10; // Graphics Mode (text mode)
        vga.graphics_regs[6] = 0x0E; // Misc (text mode, B8000)
        vga.graphics_regs[7] = 0x00; // Color Don't Care
        vga.graphics_regs[8] = 0xFF; // Bit Mask

        // Initialize attribute controller
        vga.attr_regs[0] = 0x00; // Palette 0-15
        for i in 1..16 {
            vga.attr_regs[i] = i as u8;
        }
        vga.attr_regs[16] = 0x0F; // Attribute mode control
        vga.attr_regs[17] = 0x00; // Overscan color
        vga.attr_regs[18] = 0x0F; // Color plane enable
        vga.attr_regs[19] = 0x08; // Horizontal pixel panning
        vga.attr_regs[20] = 0x00; // Color select

        vga
    }

    /// Initialize VGA device
    pub(crate) fn init(&mut self, io: &mut BxDevicesC, mem: &mut BxMemC) -> Result<()> {
        tracing::info!("Initializing VGA text mode");

        // Register I/O port handlers
        let vga_ptr = self as *mut BxVgaC as *mut c_void;

        // CRTC registers (0x3D4-0x3D5)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_CRTC_INDEX,
            "VGA CRTC Index",
            0x1,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_CRTC_DATA,
            "VGA CRTC Data",
            0x1,
        );

        // Status register (0x3DA)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_STATUS,
            "VGA Status",
            0x1,
        );

        // Attribute controller (0x3C0-0x3C1)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_ATTRIB_ADDR,
            "VGA Attribute Address",
            0x1,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_ATTRIB_DATA,
            "VGA Attribute Data",
            0x1,
        );

        // Sequencer (0x3C4-0x3C5)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_SEQ_INDEX,
            "VGA Sequencer Index",
            0x1,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_SEQ_DATA,
            "VGA Sequencer Data",
            0x1,
        );

        // Graphics controller (0x3CE-0x3CF)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_GRAPHICS_INDEX,
            "VGA Graphics Index",
            0x1,
        );
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_GRAPHICS_DATA,
            "VGA Graphics Data",
            0x1,
        );

        // Misc output (0x3CC)
        io.register_io_handler(
            vga_ptr,
            vga_read_handler,
            vga_write_handler,
            VGA_MISC_OUTPUT,
            "VGA Misc Output",
            0x1,
        );

        // Register memory handlers for VGA memory range (0xA0000-0xBFFFF)
        // This matches DEV_register_memory_handlers in vgacore.cc line 177
        let vga_ptr_const = vga_ptr as *const c_void;
        mem.register_memory_handlers(
            vga_ptr_const,
            vga_mem_read_handler,
            vga_mem_write_handler,
            0xA0000,  // Start of VGA memory range
            0xBFFFF,  // End of VGA memory range
        )?;

        tracing::info!("VGA initialized (80x25 text mode)");
        Ok(())
    }

    /// Reset VGA controller
    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }

    /// Read from I/O port
    pub(crate) fn read_port(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            VGA_CRTC_INDEX => self.crtc_index as u32,
            VGA_CRTC_DATA => {
                if self.crtc_index < 25 {
                    self.crtc_regs[self.crtc_index as usize] as u32
                } else {
                    0
                }
            }
            VGA_STATUS => {
                // Status register: bit 0 = display enable, bit 3 = vertical retrace
                // Toggle bit 0 for display enable status
                self.status_reg ^= 0x01;
                (self.status_reg | 0x08) as u32 // Always set bit 3 (vertical retrace)
            }
            VGA_ATTRIB_ADDR | VGA_ATTRIB_DATA => {
                // Attribute controller: reading toggles flip-flop
                self.attr_flip_flop = !self.attr_flip_flop;
                if self.attr_flip_flop {
                    // Reading index
                    self.attr_index as u32
                } else {
                    // Reading data
                    if self.attr_index < 21 {
                        self.attr_regs[self.attr_index as usize] as u32
                    } else {
                        0
                    }
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
            _ => {
                tracing::trace!("VGA read from unhandled port {:#x}", port);
                0xFF
            }
        }
    }

    /// Write to I/O port
    pub(crate) fn write_port(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            VGA_CRTC_INDEX => {
                self.crtc_index = value & 0x1F; // Only 5 bits
            }
            VGA_CRTC_DATA => {
                if self.crtc_index < 25 {
                    self.crtc_regs[self.crtc_index as usize] = value;
                    
                    // Update cursor position if cursor location registers changed
                    if self.crtc_index == CRTC_CURSOR_LOC_HIGH {
                        let cursor_addr = ((value as u16) << 8) | (self.crtc_regs[CRTC_CURSOR_LOC_LOW as usize] as u16);
                        self.cursor_pos = ((cursor_addr as usize / BYTES_PER_ROW), (cursor_addr as usize % BYTES_PER_ROW) / BYTES_PER_CHAR);
                    } else if self.crtc_index == CRTC_CURSOR_LOC_LOW {
                        let cursor_addr = ((self.crtc_regs[CRTC_CURSOR_LOC_HIGH as usize] as u16) << 8) | (value as u16);
                        self.cursor_pos = ((cursor_addr as usize / BYTES_PER_ROW), (cursor_addr as usize % BYTES_PER_ROW) / BYTES_PER_CHAR);
                    }
                }
            }
            VGA_ATTRIB_ADDR => {
                // Writing to 0x3C0 toggles flip-flop
                if self.attr_flip_flop {
                    // Writing data
                    if self.attr_index < 21 {
                        self.attr_regs[self.attr_index as usize] = value;
                    }
                } else {
                    // Writing index
                    self.attr_index = value & 0x1F;
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
                self.seq_index = value & 0x07; // Only 3 bits
            }
            VGA_SEQ_DATA => {
                if self.seq_index < 5 {
                    self.seq_regs[self.seq_index as usize] = value;
                }
            }
            VGA_GRAPHICS_INDEX => {
                self.graphics_index = value & 0x0F; // Only 4 bits
            }
            VGA_GRAPHICS_DATA => {
                if self.graphics_index < 9 {
                    self.graphics_regs[self.graphics_index as usize] = value;
                }
            }
            VGA_MISC_OUTPUT => {
                self.misc_output = value;
            }
            _ => {
                tracing::trace!("VGA write to unhandled port {:#x} = {:#x}", port, value);
            }
        }
    }

    /// Read from text mode memory
    pub(crate) fn read_memory(&self, addr: BxPhyAddress, len: usize) -> Vec<u8> {
        let offset = (addr - VGA_TEXT_MEM_BASE) as usize;
        if offset + len <= self.text_memory.len() {
            self.text_memory[offset..offset + len].to_vec()
        } else {
            vec![0; len]
        }
    }

    /// Write to text mode memory
    pub(crate) fn write_memory(&mut self, addr: BxPhyAddress, data: &[u8]) {
        let offset = (addr - VGA_TEXT_MEM_BASE) as usize;
        if offset + data.len() <= self.text_memory.len() {
            self.text_memory[offset..offset + data.len()].copy_from_slice(data);
        }
    }

    /// Get text mode screen contents as a string
    pub(crate) fn get_text_screen(&self) -> String {
        let mut result = String::new();
        for row in 0..TEXT_ROWS {
            for col in 0..TEXT_COLS {
                let offset = (row * BYTES_PER_ROW) + (col * BYTES_PER_CHAR);
                if offset + 1 < self.text_memory.len() {
                    let ch = self.text_memory[offset] as char;
                    result.push(ch);
                }
            }
            result.push('\n');
        }
        result
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
        // Check if we're in text mode
        // graphics_regs[5] bit 4 = 0 means text mode (graphics_alpha = 0)
        // graphics_regs[6] bits 2-3 indicate memory mapping:
        //   00 = A0000-AFFFF, 01 = A0000-BFFFF, 10 = B0000-B7FFF, 11 = B8000-BFFFF (text mode)
        let graphics_alpha = (self.graphics_regs[5] & 0x10) == 0;
        let memory_mapping = (self.graphics_regs[6] >> 2) & 0x03;
        let is_text_mode = graphics_alpha && (memory_mapping == 3);
        
        if !is_text_mode {
            return None;
        }

        // Calculate text mode parameters (matching vgacore.cc:1601-1632)
        let start_addr = ((self.crtc_regs[12] as u16) << 8) | (self.crtc_regs[13] as u16);
        let start_address = (start_addr << 1) as u16;
        
        let cs_start = self.crtc_regs[10] & 0x3f;
        let cs_end = self.crtc_regs[11] & 0x1f;
        
        // Line offset: CRTC reg[19] is offset register
        let mut line_offset = (self.crtc_regs[19] as u16) * 2; // Convert to bytes
        if line_offset == 0 {
            // Default to 80 columns * 2 bytes
            line_offset = (TEXT_COLS * BYTES_PER_CHAR) as u16;
        }
        
        let line_compare = 0; // TODO: Calculate from CRTC registers if needed
        let h_panning = self.attr_regs[19] & 0x0f;
        let v_panning = self.crtc_regs[8] & 0x1f;
        let line_graphics = (self.attr_regs[16] & 0x04) != 0;
        let split_hpanning = (self.attr_regs[16] & 0x20) != 0;
        let blink_flags = 0u8; // TODO: Calculate from attribute controller
        
        // Build palette (matching vgacore.cc:1629-1632)
        let mut actl_palette = [0u8; 16];
        for i in 0..16 {
            actl_palette[i] = self.attr_regs[i] & 0x0f; // Simplified - no pel.mask for now
        }
        
        // Calculate rows and cols (matching vgacore.cc:1634-1648)
        let mut cols = (self.crtc_regs[1] + 1) as usize;
        let mut msl = (self.crtc_regs[9] & 0x1f) as usize;
        let vde = (self.crtc_regs[18] as usize) + 
                  (((self.crtc_regs[7] & 0x02) as usize) << 7) +
                  (((self.crtc_regs[7] & 0x40) as usize) << 3);
        
        // Workaround for update() calls before VGABIOS init (matching vgacore.cc:1639-1643)
        if cols == 1 || msl == 0 {
            cols = TEXT_COLS;
        }
        if msl == 0 {
            msl = 15;
        }
        
        let rows = if msl > 0 { (vde + 1) / (msl + 1) } else { TEXT_ROWS };
        let rows = rows.min(TEXT_ROWS); // Cap at 25 rows
        
        // Calculate cursor address (matching vgacore.cc:1671-1676)
        let cursor_addr = ((self.crtc_regs[14] as u16) << 8) | (self.crtc_regs[15] as u16);
        let cursor_address = cursor_addr * 2; // Convert to byte offset
        
        // Validate cursor address
        let max_addr = start_address + (line_offset * rows as u16);
        let cursor_address = if cursor_address < start_address || cursor_address > max_addr {
            0x7fff // Invalid cursor
        } else {
            cursor_address
        };
        
        // Copy from VGA memory to text_buffer if needed (matching vgacore.cc:1677-1683)
        if self.text_buffer_update {
            // For text mode (memory_mapping = 3), text_snap_size[3] = 0x8000 (32KB)
            // In original: copies from s.memory[i*4] (char) and s.memory[i*4+1] (attr)
            // Our text_memory is already 2-byte format, so copy directly
            let size = 0x8000.min(self.text_buffer.len()).min(self.text_memory.len());
            self.text_buffer[..size].copy_from_slice(&self.text_memory[..size]);
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
        
        // Always return update result if in text mode (original always calls text_update_common)
        // The GUI will compare old and new to determine what actually changed
        let needs_update = self.vga_mem_updated > 0;
        
        // Copy buffer to snapshot if memory was updated (matching vgacore.cc:1687-1691)
        if self.vga_mem_updated > 0 {
            let copy_size = (start_address as usize + (line_offset as usize * rows))
                .min(self.text_buffer.len())
                .min(self.text_snapshot.len());
            if copy_size > start_address as usize {
                let start = start_address as usize;
                self.text_snapshot[start..copy_size]
                    .copy_from_slice(&self.text_buffer[start..copy_size]);
            }
            self.vga_mem_updated = 0;
        }
        
        Some(VgaUpdateResult {
            needs_update,
            text_buffer: self.text_buffer.clone(),
            text_snapshot: self.text_snapshot.clone(),
            cursor_address,
            tm_info,
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
    
    // For text mode (memory_mapping = 3), handle 0xB8000-0xBFFFF
    if addr >= VGA_TEXT_MEM_BASE && addr < VGA_TEXT_MEM_BASE + VGA_TEXT_MEM_SIZE as u64 {
        let mut current_addr = addr;
        let mut data_ptr = data as *mut u8;
        
        // Process each byte (matching original mem_read_handler loop)
        for _ in 0..len {
            let offset = (current_addr - VGA_TEXT_MEM_BASE) as usize;
            if offset < vga.text_memory.len() {
                unsafe {
                    *data_ptr = vga.text_memory[offset];
                    data_ptr = data_ptr.add(1);
                }
            }
            current_addr += 1;
        }
        tracing::trace!("VGA mem read: addr={:#x}, len={}", addr, len);
        return true;
    }
    
    false
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
    // Log ALL VGA memory writes to see if handler is being called
    static mut TOTAL_WRITES: u64 = 0;
    unsafe {
        TOTAL_WRITES += 1;
        if TOTAL_WRITES <= 20 {
            tracing::info!("VGA mem_write_handler called #{}: addr={:#x}, len={}", TOTAL_WRITES, addr, len);
        }
    }
    
    if param.is_null() || data.is_null() {
        tracing::warn!("VGA mem_write_handler: null param or data");
        return false;
    }
    
    let vga = unsafe { &mut *(param as *mut BxVgaC) };
    
    // Handle all VGA memory range (0xA0000-0xBFFFF)
    // For text mode, BIOS writes to 0xB8000-0xBFFFF
    if addr >= 0xA0000 && addr <= 0xBFFFF {
        // Check if this is text mode memory (0xB8000-0xBFFFF)
        if addr >= VGA_TEXT_MEM_BASE && addr < VGA_TEXT_MEM_BASE + VGA_TEXT_MEM_SIZE as u64 {
            let mut current_addr = addr;
            let mut data_ptr = data as *const u8;
            
            // Log first few writes to see if we're getting any
            static mut WRITE_COUNT: u64 = 0;
            unsafe {
                WRITE_COUNT += 1;
                if WRITE_COUNT <= 10 {
                    tracing::info!("VGA TEXT MEM WRITE #{}: addr={:#x}, len={}", WRITE_COUNT, addr, len);
                }
            }
            
            // Process each byte (matching original mem_write_handler loop)
            for _ in 0..len {
                let offset = (current_addr - VGA_TEXT_MEM_BASE) as usize;
                if offset < vga.text_memory.len() {
                    unsafe {
                        let old_val = vga.text_memory[offset];
                        vga.text_memory[offset] = *data_ptr;
                        if old_val != *data_ptr {
                            vga.text_dirty = true; // Mark text memory as dirty
                            // Set vga_mem_updated flag (matching vgacore.cc:1852, 2180)
                            // For text mode, we set bit 0 (plane 0) or appropriate bit
                            vga.vga_mem_updated |= 1; // Mark that text memory was updated
                            if WRITE_COUNT <= 5 {
                                tracing::info!("  offset={:#x}: {:#02x} -> {:#02x}", offset, old_val, *data_ptr);
                            }
                        }
                        data_ptr = data_ptr.add(1);
                    }
                }
                current_addr += 1;
            }
            return true;
        } else {
            // Other VGA memory ranges (graphics mode, etc.) - for now just acknowledge
            tracing::trace!("VGA mem write (non-text): addr={:#x}, len={}", addr, len);
            return true;
        }
    }
    
    false
}
