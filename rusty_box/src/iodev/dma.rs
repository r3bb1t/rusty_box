//! 8237 DMA Controller Emulation
//!
//! The PC has two DMA controllers:
//! - DMA1 (8-bit): Channels 0-3 (ports 0x00-0x0F, 0x87, 0x83, 0x81, 0x82)
//! - DMA2 (16-bit): Channels 4-7 (ports 0xC0-0xDF, 0x8F, 0x8B, 0x89, 0x8A)
//!
//! Channel 4 is used for cascading DMA1 to DMA2.

use core::ffi::c_void;

/// DMA1 base ports (8-bit DMA)
pub const DMA1_ADDR_CH0: u16 = 0x0000;
pub const DMA1_COUNT_CH0: u16 = 0x0001;
pub const DMA1_ADDR_CH1: u16 = 0x0002;
pub const DMA1_COUNT_CH1: u16 = 0x0003;
pub const DMA1_ADDR_CH2: u16 = 0x0004;
pub const DMA1_COUNT_CH2: u16 = 0x0005;
pub const DMA1_ADDR_CH3: u16 = 0x0006;
pub const DMA1_COUNT_CH3: u16 = 0x0007;
pub const DMA1_STATUS: u16 = 0x0008;
pub const DMA1_COMMAND: u16 = 0x0008;
pub const DMA1_REQUEST: u16 = 0x0009;
pub const DMA1_MASK: u16 = 0x000A;
pub const DMA1_MODE: u16 = 0x000B;
pub const DMA1_CLEAR_FF: u16 = 0x000C;
pub const DMA1_MASTER_CLEAR: u16 = 0x000D;
pub const DMA1_CLEAR_MASK: u16 = 0x000E;
pub const DMA1_WRITE_ALL_MASK: u16 = 0x000F;

/// DMA2 base ports (16-bit DMA)
pub const DMA2_ADDR_CH4: u16 = 0x00C0;
pub const DMA2_COUNT_CH4: u16 = 0x00C2;
pub const DMA2_ADDR_CH5: u16 = 0x00C4;
pub const DMA2_COUNT_CH5: u16 = 0x00C6;
pub const DMA2_ADDR_CH6: u16 = 0x00C8;
pub const DMA2_COUNT_CH6: u16 = 0x00CA;
pub const DMA2_ADDR_CH7: u16 = 0x00CC;
pub const DMA2_COUNT_CH7: u16 = 0x00CE;
pub const DMA2_STATUS: u16 = 0x00D0;
pub const DMA2_COMMAND: u16 = 0x00D0;
pub const DMA2_REQUEST: u16 = 0x00D2;
pub const DMA2_MASK: u16 = 0x00D4;
pub const DMA2_MODE: u16 = 0x00D6;
pub const DMA2_CLEAR_FF: u16 = 0x00D8;
pub const DMA2_MASTER_CLEAR: u16 = 0x00DA;
pub const DMA2_CLEAR_MASK: u16 = 0x00DC;
pub const DMA2_WRITE_ALL_MASK: u16 = 0x00DE;

/// Page register ports
pub const DMA_PAGE_CH0: u16 = 0x0087;
pub const DMA_PAGE_CH1: u16 = 0x0083;
pub const DMA_PAGE_CH2: u16 = 0x0081;
pub const DMA_PAGE_CH3: u16 = 0x0082;
pub const DMA_PAGE_CH5: u16 = 0x008B;
pub const DMA_PAGE_CH6: u16 = 0x0089;
pub const DMA_PAGE_CH7: u16 = 0x008A;
pub const DMA_PAGE_REFRESH: u16 = 0x008F;

/// DMA transfer modes
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DmaTransferMode {
    Demand = 0,
    Single = 1,
    Block = 2,
    Cascade = 3,
}

/// DMA transfer direction
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DmaDirection {
    Verify = 0,
    Write = 1, // Memory to I/O
    Read = 2,  // I/O to Memory
    Invalid = 3,
}

/// State for a single DMA channel
#[derive(Debug, Clone, Default)]
pub struct DmaChannel {
    /// Channel number (0-7)
    pub(crate) number: u8,
    /// Current address register
    pub(crate) current_addr: u16,
    /// Current count register
    pub(crate) current_count: u16,
    /// Base address register
    pub(crate) base_addr: u16,
    /// Base count register
    pub(crate) base_count: u16,
    /// Page register (bits 16-23 of address)
    pub(crate) page: u8,
    /// Channel mode
    pub(crate) mode: u8,
    /// Channel is masked (disabled)
    pub(crate) masked: bool,
    /// Transfer complete
    pub(crate) tc: bool,
    /// Request pending
    pub(crate) request: bool,
}

impl DmaChannel {
    /// Create a new DMA channel
    pub fn new(number: u8) -> Self {
        Self {
            number,
            masked: true, // Masked by default
            ..Default::default()
        }
    }

    /// Get the full 24-bit address
    pub fn get_address(&self) -> u32 {
        if self.number >= 4 {
            // 16-bit DMA: address is shifted left by 1
            ((self.page as u32) << 16) | ((self.current_addr as u32) << 1)
        } else {
            // 8-bit DMA
            ((self.page as u32) << 16) | (self.current_addr as u32)
        }
    }

    /// Get transfer direction
    pub fn direction(&self) -> DmaDirection {
        match (self.mode >> 2) & 0x03 {
            0 => DmaDirection::Verify,
            1 => DmaDirection::Write,
            2 => DmaDirection::Read,
            _ => DmaDirection::Invalid,
        }
    }

    /// Get transfer mode
    pub fn transfer_mode(&self) -> DmaTransferMode {
        match (self.mode >> 6) & 0x03 {
            0 => DmaTransferMode::Demand,
            1 => DmaTransferMode::Single,
            2 => DmaTransferMode::Block,
            _ => DmaTransferMode::Cascade,
        }
    }

    /// Check if auto-init is enabled
    pub fn auto_init(&self) -> bool {
        (self.mode & 0x10) != 0
    }

    /// Check if address decrement mode
    pub fn decrement(&self) -> bool {
        (self.mode & 0x20) != 0
    }
}

/// 8237 DMA Controller (one chip)
#[derive(Debug, Clone)]
pub struct Dma8237 {
    /// Channels (0-3 or 4-7)
    pub(crate) channels: [DmaChannel; 4],
    /// Command register
    pub(crate) command: u8,
    /// Status register
    pub(crate) status: u8,
    /// Flip-flop for address/count access
    pub(crate) flip_flop: bool,
    /// Controller number (0=DMA1, 1=DMA2)
    pub(crate) controller_num: u8,
}

impl Dma8237 {
    /// Create a new DMA controller
    pub fn new(controller_num: u8) -> Self {
        let base_channel = controller_num * 4;
        Self {
            channels: [
                DmaChannel::new(base_channel),
                DmaChannel::new(base_channel + 1),
                DmaChannel::new(base_channel + 2),
                DmaChannel::new(base_channel + 3),
            ],
            command: 0,
            status: 0,
            flip_flop: false,
            controller_num,
        }
    }

    /// Reset the controller
    pub fn reset(&mut self) {
        self.command = 0;
        self.status = 0;
        self.flip_flop = false;
        for channel in &mut self.channels {
            channel.masked = true;
            channel.tc = false;
            channel.request = false;
        }
    }
}

/// Dual DMA Controller System
#[derive(Debug)]
pub struct BxDmaC {
    /// DMA1 (8-bit channels 0-3)
    pub(crate) dma1: Dma8237,
    /// DMA2 (16-bit channels 4-7)
    pub(crate) dma2: Dma8237,
    /// Extra page registers
    pub(crate) extra_pages: [u8; 8],
}

impl Default for BxDmaC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxDmaC {
    /// Create a new DMA controller system
    pub fn new() -> Self {
        Self {
            dma1: Dma8237::new(0),
            dma2: Dma8237::new(1),
            extra_pages: [0; 8],
        }
    }

    /// Initialize the DMA controllers
    pub fn init(&mut self) {
        tracing::info!("DMA: Initializing 8237 DMA Controllers");
        self.reset();

        // Channel 4 is used for cascade
        self.dma2.channels[0].mode = 0xC0; // Cascade mode
        self.dma2.channels[0].masked = false;
        tracing::debug!("DMA: Channel 4 configured for cascade");
    }

    /// Reset the DMA controllers
    pub fn reset(&mut self) {
        self.dma1.reset();
        self.dma2.reset();
        self.extra_pages = [0; 8];
    }

    /// Read from DMA I/O port
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        match port {
            // DMA1 status
            DMA1_STATUS => {
                let status = self.dma1.status;
                self.dma1.status &= 0xF0; // Clear TC bits on read
                status as u32
            }
            // DMA2 status
            DMA2_STATUS => {
                let status = self.dma2.status;
                self.dma2.status &= 0xF0;
                status as u32
            }
            // DMA1 address/count registers
            0x0000..=0x0007 => self.read_addr_count(&mut self.dma1.clone(), port as u8),
            // DMA2 address/count registers
            0x00C0..=0x00CF => {
                self.read_addr_count(&mut self.dma2.clone(), ((port - 0xC0) >> 1) as u8)
            }
            // Page registers
            DMA_PAGE_CH0 => self.dma1.channels[0].page as u32,
            DMA_PAGE_CH1 => self.dma1.channels[1].page as u32,
            DMA_PAGE_CH2 => self.dma1.channels[2].page as u32,
            DMA_PAGE_CH3 => self.dma1.channels[3].page as u32,
            DMA_PAGE_CH5 => self.dma2.channels[1].page as u32,
            DMA_PAGE_CH6 => self.dma2.channels[2].page as u32,
            DMA_PAGE_CH7 => self.dma2.channels[3].page as u32,
            DMA_PAGE_REFRESH => self.extra_pages[0] as u32,
            _ => {
                tracing::trace!("DMA: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    fn read_addr_count(&mut self, dma: &mut Dma8237, reg: u8) -> u32 {
        let channel_num = (reg >> 1) as usize;
        let is_count = (reg & 1) != 0;

        if channel_num >= 4 {
            return 0xFF;
        }

        let value = if is_count {
            dma.channels[channel_num].current_count
        } else {
            dma.channels[channel_num].current_addr
        };

        let byte = if dma.flip_flop {
            dma.flip_flop = false;
            (value >> 8) as u8
        } else {
            dma.flip_flop = true;
            (value & 0xFF) as u8
        };

        // Update original
        if dma.controller_num == 0 {
            self.dma1 = dma.clone();
        } else {
            self.dma2 = dma.clone();
        }

        byte as u32
    }

    /// Write to DMA I/O port
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            // DMA1 registers
            0x0000..=0x0007 => self.write_addr_count(0, port as u8, value),
            DMA1_COMMAND => self.dma1.command = value,
            DMA1_REQUEST => self.set_request(0, value),
            DMA1_MASK => self.set_mask(0, value),
            DMA1_MODE => self.set_mode(0, value),
            DMA1_CLEAR_FF => self.dma1.flip_flop = false,
            DMA1_MASTER_CLEAR => self.dma1.reset(),
            DMA1_CLEAR_MASK => self.clear_mask(0),
            DMA1_WRITE_ALL_MASK => self.write_all_mask(0, value),

            // DMA2 registers
            0x00C0..=0x00CF => self.write_addr_count(1, ((port - 0xC0) >> 1) as u8, value),
            DMA2_COMMAND => self.dma2.command = value,
            DMA2_REQUEST => self.set_request(1, value),
            DMA2_MASK => self.set_mask(1, value),
            DMA2_MODE => self.set_mode(1, value),
            DMA2_CLEAR_FF => self.dma2.flip_flop = false,
            DMA2_MASTER_CLEAR => self.dma2.reset(),
            DMA2_CLEAR_MASK => self.clear_mask(1),
            DMA2_WRITE_ALL_MASK => self.write_all_mask(1, value),

            // Page registers
            DMA_PAGE_CH0 => self.dma1.channels[0].page = value,
            DMA_PAGE_CH1 => self.dma1.channels[1].page = value,
            DMA_PAGE_CH2 => self.dma1.channels[2].page = value,
            DMA_PAGE_CH3 => self.dma1.channels[3].page = value,
            DMA_PAGE_CH5 => self.dma2.channels[1].page = value,
            DMA_PAGE_CH6 => self.dma2.channels[2].page = value,
            DMA_PAGE_CH7 => self.dma2.channels[3].page = value,
            DMA_PAGE_REFRESH => self.extra_pages[0] = value,

            _ => {
                tracing::trace!("DMA: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    fn write_addr_count(&mut self, controller: u8, reg: u8, value: u8) {
        let dma = if controller == 0 {
            &mut self.dma1
        } else {
            &mut self.dma2
        };
        let channel_num = (reg >> 1) as usize;
        let is_count = (reg & 1) != 0;

        if channel_num >= 4 {
            return;
        }

        if is_count {
            if dma.flip_flop {
                dma.channels[channel_num].base_count =
                    (dma.channels[channel_num].base_count & 0x00FF) | ((value as u16) << 8);
                dma.channels[channel_num].current_count = dma.channels[channel_num].base_count;
            } else {
                dma.channels[channel_num].base_count =
                    (dma.channels[channel_num].base_count & 0xFF00) | (value as u16);
            }
        } else {
            if dma.flip_flop {
                dma.channels[channel_num].base_addr =
                    (dma.channels[channel_num].base_addr & 0x00FF) | ((value as u16) << 8);
                dma.channels[channel_num].current_addr = dma.channels[channel_num].base_addr;
            } else {
                dma.channels[channel_num].base_addr =
                    (dma.channels[channel_num].base_addr & 0xFF00) | (value as u16);
            }
        }
        dma.flip_flop = !dma.flip_flop;
    }

    fn set_request(&mut self, controller: u8, value: u8) {
        let dma = if controller == 0 {
            &mut self.dma1
        } else {
            &mut self.dma2
        };
        let channel = (value & 0x03) as usize;
        dma.channels[channel].request = (value & 0x04) != 0;
    }

    fn set_mask(&mut self, controller: u8, value: u8) {
        let dma = if controller == 0 {
            &mut self.dma1
        } else {
            &mut self.dma2
        };
        let channel = (value & 0x03) as usize;
        dma.channels[channel].masked = (value & 0x04) != 0;
    }

    fn set_mode(&mut self, controller: u8, value: u8) {
        let dma = if controller == 0 {
            &mut self.dma1
        } else {
            &mut self.dma2
        };
        let channel = (value & 0x03) as usize;
        dma.channels[channel].mode = value;
        tracing::debug!(
            "DMA: Channel {} mode set to {:#04x}",
            channel + (controller * 4) as usize,
            value
        );
    }

    fn clear_mask(&mut self, controller: u8) {
        let dma = if controller == 0 {
            &mut self.dma1
        } else {
            &mut self.dma2
        };
        for channel in &mut dma.channels {
            channel.masked = false;
        }
    }

    fn write_all_mask(&mut self, controller: u8, value: u8) {
        let dma = if controller == 0 {
            &mut self.dma1
        } else {
            &mut self.dma2
        };
        for (i, channel) in dma.channels.iter_mut().enumerate() {
            channel.masked = (value & (1 << i)) != 0;
        }
    }

    /// Get a channel reference
    pub fn get_channel(&self, channel_num: u8) -> Option<&DmaChannel> {
        if channel_num < 4 {
            Some(&self.dma1.channels[channel_num as usize])
        } else if channel_num < 8 {
            Some(&self.dma2.channels[(channel_num - 4) as usize])
        } else {
            None
        }
    }

    /// Acknowledge transfer complete
    pub fn set_tc(&mut self, channel_num: u8) {
        if channel_num < 4 {
            self.dma1.status |= 1 << channel_num;
            self.dma1.channels[channel_num as usize].tc = true;
        } else if channel_num < 8 {
            let ch = channel_num - 4;
            self.dma2.status |= 1 << ch;
            self.dma2.channels[ch as usize].tc = true;
        }
    }
}

/// DMA read handler for I/O port infrastructure
pub fn dma_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let dma = unsafe { &mut *(this_ptr as *mut BxDmaC) };
    dma.read(port, io_len)
}

/// DMA write handler for I/O port infrastructure
pub fn dma_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let dma = unsafe { &mut *(this_ptr as *mut BxDmaC) };
    dma.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dma_creation() {
        let dma = BxDmaC::new();
        assert_eq!(dma.dma1.controller_num, 0);
        assert_eq!(dma.dma2.controller_num, 1);
    }

    #[test]
    fn test_dma_channel_address() {
        let mut channel = DmaChannel::new(2);
        channel.current_addr = 0x1234;
        channel.page = 0x56;

        assert_eq!(channel.get_address(), 0x00561234);
    }
}
