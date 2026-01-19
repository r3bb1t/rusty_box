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
