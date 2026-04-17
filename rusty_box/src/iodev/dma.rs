#![allow(dead_code)]
//! 8237 DMA Controller Emulation
//!
//! Matches Bochs `iodev/dma.cc` and `iodev/dma.h`.
//!
//! The PC has two DMA controllers:
//! - DMA1 (8-bit): Channels 0-3 (ports 0x00-0x0F, page regs 0x87, 0x83, 0x81, 0x82)
//! - DMA2 (16-bit): Channels 4-7 (ports 0xC0-0xDF, page regs 0x8F, 0x8B, 0x89, 0x8A)
//!
//! Channel 4 is used for cascading DMA1 to DMA2.
//!
//! ## DMA Transfer Machinery
//!
//! The DMA transfer flow (Bochs dma.cc):
//! 1. Device asserts DRQ via `set_drq(channel, true)`
//! 2. `control_hrq()` checks for unmasked DRQ, asserts HRQ to CPU via `pc_system.set_hrq(true)`
//! 3. CPU acknowledges with HLDA at next instruction boundary
//! 4. `raise_hlda()` performs the actual data transfer:
//!    - Finds highest-priority channel with active DRQ
//!    - Calls registered device handler (dmaRead8/dmaWrite8 or dmaRead16/dmaWrite16)
//!    - Updates address/count, handles TC (terminal count)
//!
//! Currently no devices register DMA handlers (IDE uses PIO, floppy not implemented),
//! so the machinery exists structurally but no actual data transfers occur.


/// DMA buffer size for transfers (Bochs dma.h BX_DMA_BUFFER_SIZE = 512)
const BX_DMA_BUFFER_SIZE: usize = 512;

/// DMA transfer mode constants (Bochs dma.cc enum)
const DMA_MODE_DEMAND: u8 = 0;
const DMA_MODE_SINGLE: u8 = 1;
const DMA_MODE_BLOCK: u8 = 2;
const DMA_MODE_CASCADE: u8 = 3;

/// Index to find channel from page register number (Bochs dma.cc)
/// channelindex[address - 0x81] maps page register port to channel number.
/// Only indices [0],[1],[2],[6] are used (ports 0x81,0x82,0x83,0x87).
const CHANNEL_INDEX: [u8; 7] = [2, 3, 1, 0, 0, 0, 0];

/// DMA channel mode fields (Bochs dma.h)
#[derive(Debug, Clone, Default)]
pub struct DmaChannelMode {
    /// Transfer mode type: 0=demand, 1=single, 2=block, 3=cascade
    pub(crate) mode_type: u8,
    /// Address decrement (0=increment, 1=decrement)
    pub(crate) address_decrement: bool,
    /// Auto-init enable
    pub(crate) autoinit_enable: bool,
    /// Transfer type: 0=verify, 1=write (I/O to memory), 2=read (memory to I/O), 3=invalid
    pub(crate) transfer_type: u8,
}

/// State for a single DMA channel (Bochs dma.h)
#[derive(Debug, Clone, Default)]
pub struct DmaChannel {
    /// Channel mode fields (Bochs dma.h)
    pub(crate) mode: DmaChannelMode,
    /// Base address register (Bochs dma.h)
    pub(crate) base_address: u16,
    /// Current address register (Bochs dma.h)
    pub(crate) current_address: u16,
    /// Base count register (Bochs dma.h)
    pub(crate) base_count: u16,
    /// Current count register (Bochs dma.h)
    pub(crate) current_count: u16,
    /// Page register (bits 16-23 of physical address, Bochs dma.h)
    pub(crate) page_reg: u8,
    /// Channel in use by a device (Bochs dma.h)
    pub(crate) used: bool,
}

/// 8237 DMA Controller state (one chip, Bochs dma.h)
#[derive(Debug, Clone)]
pub struct Dma8237 {
    /// DMA Request lines (Bochs dma.h)
    pub(crate) drq: [bool; 4],
    /// DMA Acknowledge lines (Bochs dma.h)
    pub(crate) dack: [bool; 4],
    /// Mask bits per channel (Bochs dma.h)
    pub(crate) mask: [bool; 4],
    /// Flip-flop for address/count byte access (Bochs dma.h)
    pub(crate) flip_flop: bool,
    /// Status register (Bochs dma.h)
    pub(crate) status_reg: u8,
    /// Command register (Bochs dma.h)
    pub(crate) command_reg: u8,
    /// Controller disabled (command register bit 2, Bochs dma.h)
    pub(crate) ctrl_disabled: bool,
    /// Channel state (Bochs dma.h)
    pub(crate) chan: [DmaChannel; 4],
}

impl Dma8237 {
    fn new() -> Self {
        Self {
            drq: [false; 4],
            dack: [false; 4],
            mask: [true, true, true, true], // Masked by default on reset
            flip_flop: false,
            status_reg: 0,
            command_reg: 0,
            ctrl_disabled: false,
            chan: [
                DmaChannel::default(),
                DmaChannel::default(),
                DmaChannel::default(),
                DmaChannel::default(),
            ],
        }
    }

    /// Reset controller (Bochs dma.cc reset_controller)
    fn reset(&mut self) {
        self.mask = [true, true, true, true];
        self.ctrl_disabled = false;
        self.command_reg = 0;
        self.status_reg = 0;
        self.flip_flop = false;
    }
}

/// DMA 8-bit channel handler function pointer types (Bochs dma.h)
///
/// dmaRead8: called during DMA read (memory-to-I/O). Device reads from buffer.
///   `data_byte`: buffer containing data read from memory
///   `maxlen`: maximum number of bytes
///   Returns: number of bytes actually consumed
///
/// dmaWrite8: called during DMA write (I/O-to-memory). Device fills buffer.
///   `data_byte`: buffer for device to fill with data
///   `maxlen`: maximum number of bytes
///   Returns: number of bytes written to buffer
pub type DmaRead8Handler = fn(data_byte: &[u8], maxlen: u16) -> u16;
pub type DmaWrite8Handler = fn(data_byte: &mut [u8], maxlen: u16) -> u16;

/// DMA 16-bit channel handler function pointer types (Bochs dma.h)
pub type DmaRead16Handler = fn(data_word: &[u16], maxlen: u16) -> u16;
pub type DmaWrite16Handler = fn(data_word: &mut [u16], maxlen: u16) -> u16;

/// Per-channel DMA handler registrations (Bochs dma.h)
#[derive(Debug, Default)]
struct DmaHandlers {
    dma_read8: Option<DmaRead8Handler>,
    dma_write8: Option<DmaWrite8Handler>,
    dma_read16: Option<DmaRead16Handler>,
    dma_write16: Option<DmaWrite16Handler>,
}

/// Dual DMA Controller System (Bochs dma.h)
#[derive(Debug)]
pub struct BxDmaC {
    /// Controller state: s[0] = DMA1, s[1] = DMA2 (Bochs dma.h)
    pub(crate) s: [Dma8237; 2],
    /// Hold Acknowledge (Bochs dma.h)
    hlda: bool,
    /// Terminal Count (Bochs dma.h)
    tc: bool,
    /// Extra page registers (Bochs dma.h)
    pub(crate) ext_page_reg: [u8; 16],
    /// Per-channel DMA handlers (Bochs dma.h, h[4])
    handlers: [DmaHandlers; 4],
    /// Pointer to emulator RAM for DMA physical read/write.
    /// Matches Bochs DEV_MEM_READ_PHYSICAL_DMA / DEV_MEM_WRITE_PHYSICAL_DMA.
    memory_base: Option<core::ptr::NonNull<u8>>,
    /// Length of the memory region pointed to by memory_base.
    memory_len: usize,
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
            s: [Dma8237::new(), Dma8237::new()],
            hlda: false,
            tc: false,
            ext_page_reg: [0; 16],
            handlers: [
                DmaHandlers::default(),
                DmaHandlers::default(),
                DmaHandlers::default(),
                DmaHandlers::default(),
            ],
            memory_base: None,
            memory_len: 0,
        }
    }

    /// Initialize the DMA controllers (Bochs dma.cc)
    pub fn init(&mut self) {
        tracing::debug!("DMA: Initializing 8237 DMA Controllers");

        for i in 0..2 {
            for j in 0..4 {
                self.s[i].drq[j] = false;
                self.s[i].dack[j] = false;
            }
        }
        self.hlda = false;
        self.tc = false;

        // Init all channel state (Bochs dma.cc)
        for i in 0..2 {
            for c in 0..4 {
                self.s[i].chan[c].mode.mode_type = 0;
                self.s[i].chan[c].mode.address_decrement = false;
                self.s[i].chan[c].mode.autoinit_enable = false;
                self.s[i].chan[c].mode.transfer_type = 0;
                self.s[i].chan[c].base_address = 0;
                self.s[i].chan[c].current_address = 0;
                self.s[i].chan[c].base_count = 0;
                self.s[i].chan[c].current_count = 0;
                self.s[i].chan[c].page_reg = 0;
                self.s[i].chan[c].used = false;
            }
        }
        self.ext_page_reg = [0; 16];

        // Channel 4 is cascade (Bochs dma.cc)
        self.s[1].chan[0].used = true;
        tracing::debug!("DMA: channel 4 used by cascade");
    }

    /// Set RAM pointer for physical DMA transfers.
    /// Must be called after init() with valid pointer to emulator RAM.
    /// Matches Bochs DEV_MEM_READ_PHYSICAL_DMA / DEV_MEM_WRITE_PHYSICAL_DMA.
    pub fn set_memory_ptrs(
        &mut self,
        mem_base: *mut u8,
        mem_len: usize,
    ) {
        self.memory_base = core::ptr::NonNull::new(mem_base);
        self.memory_len = mem_len;
    }

    /// Reset the DMA controllers (Bochs dma.cc)
    pub fn reset(&mut self) {
        self.reset_controller(0);
        self.reset_controller(1);
    }

    /// Reset a single controller (Bochs dma.cc)
    fn reset_controller(&mut self, num: usize) {
        self.s[num].mask = [true, true, true, true];
        self.s[num].ctrl_disabled = false;
        self.s[num].command_reg = 0;
        self.s[num].status_reg = 0;
        self.s[num].flip_flop = false;
    }

    // -----------------------------------------------------------------------
    // DMA handler registration (Bochs dma.cc)
    // -----------------------------------------------------------------------

    /// Register an 8-bit DMA channel handler (Bochs dma.cc)
    pub fn register_dma8_channel(
        &mut self,
        channel: usize,
        dma_read: DmaRead8Handler,
        dma_write: DmaWrite8Handler,
        name: &str,
    ) -> bool {
        if channel > 3 {
            tracing::error!("registerDMA8Channel: invalid channel number({})", channel);
            return false;
        }
        if self.s[0].chan[channel].used {
            tracing::error!("registerDMA8Channel: channel({}) already in use", channel);
            return false;
        }
        tracing::debug!("DMA: channel {} used by {}", channel, name);
        self.handlers[channel].dma_read8 = Some(dma_read);
        self.handlers[channel].dma_write8 = Some(dma_write);
        self.s[0].chan[channel].used = true;
        true
    }

    /// Register a 16-bit DMA channel handler (Bochs dma.cc)
    pub fn register_dma16_channel(
        &mut self,
        channel: usize,
        dma_read: DmaRead16Handler,
        dma_write: DmaWrite16Handler,
        name: &str,
    ) -> bool {
        if !(4..=7).contains(&channel) {
            tracing::error!(
                "registerDMA16Channel: invalid channel number({})",
                channel
            );
            return false;
        }
        let ch = channel & 0x03;
        if self.s[1].chan[ch].used {
            tracing::error!("registerDMA16Channel: channel({}) already in use", channel);
            return false;
        }
        tracing::debug!("DMA: channel {} used by {}", channel, name);
        self.handlers[ch].dma_read16 = Some(dma_read);
        self.handlers[ch].dma_write16 = Some(dma_write);
        self.s[1].chan[ch].used = true;
        true
    }

    /// Unregister a DMA channel (Bochs dma.cc)
    pub fn unregister_dma_channel(&mut self, channel: usize) -> bool {
        let ma_sl = if channel > 3 { 1 } else { 0 };
        let ch = channel & 0x03;
        self.s[ma_sl].chan[ch].used = false;
        tracing::debug!("DMA: channel {} no longer used", channel);
        true
    }

    /// Get Terminal Count state (Bochs dma.cc)
    pub fn get_tc(&self) -> bool {
        self.tc
    }

    // -----------------------------------------------------------------------
    // DMA control logic (Bochs dma.cc)
    // -----------------------------------------------------------------------

    /// Set DRQ line for a channel (Bochs dma.cc)
    pub fn set_drq(&mut self, channel: usize, val: bool) {
        if channel > 7 {
            tracing::error!("set_DRQ() channel > 7");
            return;
        }
        let ma_sl = if channel > 3 { 1 } else { 0 };
        let ch = channel & 0x03;
        self.s[ma_sl].drq[ch] = val;

        if !self.s[ma_sl].chan[ch].used {
            tracing::error!("set_DRQ(): channel {} not connected to device", channel);
            return;
        }

        if !val {
            // Clear request bit in status reg (Bochs dma.cc)
            self.s[ma_sl].status_reg &= !(1 << (ch + 4));
            self.control_hrq(ma_sl);
            return;
        }

        // Set request bit in status reg (Bochs dma.cc)
        self.s[ma_sl].status_reg |= 1 << (ch + 4);

        // Validate mode type (Bochs dma.cc)
        let mode_type = self.s[ma_sl].chan[ch].mode.mode_type;
        if mode_type != DMA_MODE_SINGLE
            && mode_type != DMA_MODE_DEMAND
            && mode_type != DMA_MODE_CASCADE
        {
            tracing::error!("set_DRQ: mode_type({:#04x}) not handled", mode_type);
        }

        // Boundary check (Bochs dma.cc)
        let dma_base = ((self.s[ma_sl].chan[ch].page_reg as u32) << 16)
            | ((self.s[ma_sl].chan[ch].base_address as u32) << ma_sl);
        let dma_roof = if !self.s[ma_sl].chan[ch].mode.address_decrement {
            dma_base.wrapping_add((self.s[ma_sl].chan[ch].base_count as u32) << ma_sl)
        } else {
            dma_base.wrapping_sub((self.s[ma_sl].chan[ch].base_count as u32) << ma_sl)
        };
        let boundary_mask: u32 = 0x7fff0000 << ma_sl;
        if (dma_base & boundary_mask) != (dma_roof & boundary_mask) {
            tracing::debug!("dma_base = {:#010x}", dma_base);
            tracing::debug!(
                "dma_base_count = {:#010x}",
                self.s[ma_sl].chan[ch].base_count
            );
            tracing::debug!("dma_roof = {:#010x}", dma_roof);
            tracing::warn!("request outside {}k boundary", 64 << ma_sl);
        }

        self.control_hrq(ma_sl);
    }

    /// Check DRQ lines and assert/deassert HRQ to CPU (Bochs dma.cc)
    fn control_hrq(&mut self, ma_sl: usize) {
        // Do nothing if controller is disabled (Bochs dma.cc)
        if self.s[ma_sl].ctrl_disabled {
            return;
        }

        // Deassert HRQ if no DRQ is pending (Bochs dma.cc)
        if (self.s[ma_sl].status_reg & 0xF0) == 0 {
            if ma_sl != 0 {
                // DMA2: deassert HRQ to CPU.
                // NOTE: pc_system is not wired here yet. When DMA devices are
                // connected, the I/O dispatch path must provide pc_system so
                // control_hrq can signal HRQ to the CPU.
                tracing::trace!("DMA: would deassert HRQ (pc_system not wired)");
            } else {
                // DMA1: clear cascade DRQ on DMA2 channel 0
                self.set_drq(4, false);
            }
            return;
        }

        // Find highest priority channel with active unmasked DRQ (Bochs dma.cc)
        for ch in 0..4 {
            if (self.s[ma_sl].status_reg & (1 << (ch + 4))) != 0 && !self.s[ma_sl].mask[ch] {
                if ma_sl != 0 {
                    // DMA2: assert HRQ to CPU (see deassert note above).
                    tracing::trace!("DMA: would assert HRQ (pc_system not wired)")
                } else {
                    // DMA1: send DRQ to cascade channel of the master
                    self.set_drq(4, true);
                }
                break;
            }
        }
    }

    /// Raise HLDA — perform the actual DMA transfer (Bochs dma.cc)
    ///
    /// Called by the CPU at instruction boundary when HRQ is asserted.
    /// Finds the highest-priority channel with active DRQ, performs data transfer
    /// via registered handlers, and updates address/count.
    ///
    /// The `mem_read_physical` and `mem_write_physical` closures provide access
    /// to physical memory for DMA transfers (matching Bochs DEV_MEM_READ_PHYSICAL_DMA
    /// and DEV_MEM_WRITE_PHYSICAL_DMA).
    pub fn raise_hlda(&mut self) {
        let mut ma_sl: usize = 0;

        self.hlda = true;

        // Find highest priority channel on DMA2 first (Bochs dma.cc)
        let mut channel: usize = 4; // sentinel: no channel found
        for ch in 0..4 {
            if (self.s[1].status_reg & (1 << (ch + 4))) != 0 && !self.s[1].mask[ch] {
                ma_sl = 1;
                channel = ch;
                break;
            }
        }

        // If channel 0 on DMA2 (cascade), look at DMA1 (Bochs dma.cc)
        if channel == 0 {
            self.s[1].dack[0] = true;
            channel = 4; // reset sentinel
            for ch in 0..4 {
                if (self.s[0].status_reg & (1 << (ch + 4))) != 0 && !self.s[0].mask[ch] {
                    ma_sl = 0;
                    channel = ch;
                    break;
                }
            }
        }

        // No channel found — wait till unmasked (Bochs dma.cc)
        if channel >= 4 {
            return;
        }

        // Compute physical address (Bochs dma.cc)
        let phy_addr: u32 = ((self.s[ma_sl].chan[channel].page_reg as u32) << 16)
            | ((self.s[ma_sl].chan[channel].current_address as u32) << (ma_sl as u32));

        // Compute maxlen and TC (Bochs dma.cc)
        let mut maxlen: u16;
        if !self.s[ma_sl].chan[channel].mode.address_decrement {
            maxlen = (self.s[ma_sl].chan[channel].current_count + 1) << (ma_sl as u16);
            self.tc = (maxlen as usize) <= BX_DMA_BUFFER_SIZE;
            if (maxlen as usize) > BX_DMA_BUFFER_SIZE {
                maxlen = BX_DMA_BUFFER_SIZE as u16;
            }
        } else {
            self.tc = self.s[ma_sl].chan[channel].current_count == 0;
            maxlen = 1 << (ma_sl as u16);
        }

        let mut buffer = [0u8; BX_DMA_BUFFER_SIZE];
        let len: u16;

        let transfer_type = self.s[ma_sl].chan[channel].mode.transfer_type;

        match transfer_type {
            1 => {
                // Write: DMA controlled transfer of bytes from I/O to Memory
                // (Bochs dma.cc)
                if ma_sl == 0 {
                    if let Some(handler) = self.handlers[channel].dma_write8 {
                        len = handler(&mut buffer, maxlen);
                    } else {
                        tracing::error!("DMA: no dmaWrite handler for channel {}", channel);
                        return;
                    }
                } else {
                    if let Some(handler) = self.handlers[channel].dma_write16 {
                        let word_buf = Self::buffer_as_word_slice_mut(&mut buffer);
                        len = handler(word_buf, maxlen / 2);
                    } else {
                        tracing::error!("DMA: no dmaWrite handler for channel {}", channel);
                        return;
                    }
                }

                // Write buffer to physical memory
                self.mem_write_physical_dma(phy_addr, len as u32, &buffer);
            }
            2 => {
                // Read: DMA controlled transfer of bytes from Memory to I/O
                // (Bochs dma.cc)

                // Read from physical memory into buffer
                self.mem_read_physical_dma(phy_addr, maxlen as u32, &mut buffer);

                if ma_sl == 0 {
                    if let Some(handler) = self.handlers[channel].dma_read8 {
                        len = handler(&buffer, maxlen);
                    } else {
                        len = maxlen;
                    }
                } else {
                    if let Some(handler) = self.handlers[channel].dma_read16 {
                        let word_buf = Self::buffer_as_word_slice(&buffer);
                        len = handler(word_buf, maxlen / 2);
                    } else {
                        len = maxlen;
                    }
                }
            }
            0 => {
                // Verify transfer (Bochs dma.cc)
                if ma_sl == 0 {
                    if let Some(handler) = self.handlers[channel].dma_write8 {
                        len = handler(&mut buffer, 1);
                    } else {
                        tracing::error!("DMA: no dmaWrite handler for channel {}", channel);
                        return;
                    }
                } else {
                    if let Some(handler) = self.handlers[channel].dma_write16 {
                        let word_buf = Self::buffer_as_word_slice_mut(&mut buffer);
                        len = handler(word_buf, 1);
                    } else {
                        tracing::error!("DMA: no dmaWrite handler for channel {}", channel);
                        return;
                    }
                }
            }
            _ => {
                // transfer_type 3 is undefined (Bochs dma.cc)
                tracing::error!("DMA: hlda: transfer_type 3 is undefined");
                return;
            }
        }

        // Set DACK (Bochs dma.cc)
        self.s[ma_sl].dack[channel] = true;

        // Update address and count (Bochs dma.cc)
        if !self.s[ma_sl].chan[channel].mode.address_decrement {
            self.s[ma_sl].chan[channel].current_address =
                self.s[ma_sl].chan[channel].current_address.wrapping_add(len);
        } else {
            self.s[ma_sl].chan[channel].current_address =
                self.s[ma_sl].chan[channel].current_address.wrapping_sub(1);
        }
        self.s[ma_sl].chan[channel].current_count =
            self.s[ma_sl].chan[channel].current_count.wrapping_sub(len);

        // Check for count expiration (0xFFFF means underflow, Bochs dma.cc)
        if self.s[ma_sl].chan[channel].current_count == 0xFFFF {
            // Count expired — done with transfer
            // Assert TC, deassert HRQ & DACK
            self.s[ma_sl].status_reg |= 1 << channel; // Hold TC in status reg

            if !self.s[ma_sl].chan[channel].mode.autoinit_enable {
                // Set mask bit if not in autoinit mode (Bochs dma.cc)
                self.s[ma_sl].mask[channel] = true;
            } else {
                // Autoinit: reload count and base address (Bochs dma.cc)
                self.s[ma_sl].chan[channel].current_address =
                    self.s[ma_sl].chan[channel].base_address;
                self.s[ma_sl].chan[channel].current_count =
                    self.s[ma_sl].chan[channel].base_count;
            }

            // Clear TC, HLDA, HRQ, DACK (Bochs dma.cc)
            self.tc = false;
            self.hlda = false;
            // NOTE: pc_system not wired here; HRQ deassert is a no-op.
            // When DMA devices are connected, raise_hlda must receive
            // &mut BxPcSystemC to call set_hrq(false).
            self.s[ma_sl].dack[channel] = false;
            if ma_sl == 0 {
                // Clear cascade (Bochs dma.cc)
                self.set_drq(4, false);
                self.s[1].dack[0] = false;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Physical memory access for DMA transfers
    // Matches Bochs DEV_MEM_READ_PHYSICAL_DMA / DEV_MEM_WRITE_PHYSICAL_DMA
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Safe memory accessors — centralize all raw pointer arithmetic here
    // -----------------------------------------------------------------------

    /// Read a single byte from emulator RAM at `offset`.
    /// Returns 0xFF if out of bounds or no memory is attached.
    #[inline]
    fn read_memory_byte(&self, offset: usize) -> u8 {
        match self.memory_base {
            Some(ptr) if offset < self.memory_len => {
                // SAFETY: bounds checked above; pointer valid for emulator lifetime.
                unsafe { *ptr.as_ptr().add(offset) }
            }
            _ => 0xFF,
        }
    }

    /// Write a single byte to emulator RAM at `offset`.
    /// Silently drops the write if out of bounds or no memory is attached.
    #[inline]
    fn write_memory_byte(&mut self, offset: usize, value: u8) {
        match self.memory_base {
            Some(ptr) if offset < self.memory_len => {
                // SAFETY: bounds checked above; pointer valid for emulator lifetime.
                unsafe { *ptr.as_ptr().add(offset) = value; }
            }
            _ => {}
        }
    }

    /// Reinterpret a `&[u8; BX_DMA_BUFFER_SIZE]` as `&[u16]` for 16-bit DMA.
    #[inline]
    fn buffer_as_word_slice(buffer: &[u8; BX_DMA_BUFFER_SIZE]) -> &[u16] {
        // SAFETY: BX_DMA_BUFFER_SIZE is 512, even and power-of-two aligned on stack.
        // The array is stack-allocated with natural alignment >= 2 for u16.
        // Length BX_DMA_BUFFER_SIZE / 2 stays within the buffer.
        unsafe {
            core::slice::from_raw_parts(
                buffer.as_ptr() as *const u16,
                BX_DMA_BUFFER_SIZE / 2,
            )
        }
    }

    /// Reinterpret a `&mut [u8; BX_DMA_BUFFER_SIZE]` as `&mut [u16]` for 16-bit DMA.
    #[inline]
    fn buffer_as_word_slice_mut(buffer: &mut [u8; BX_DMA_BUFFER_SIZE]) -> &mut [u16] {
        // SAFETY: same as buffer_as_word_slice; mutable borrow is exclusive.
        unsafe {
            core::slice::from_raw_parts_mut(
                buffer.as_mut_ptr() as *mut u16,
                BX_DMA_BUFFER_SIZE / 2,
            )
        }
    }

    /// Read physical memory for DMA transfer.
    /// Uses raw memory pointer set during init. Safe because the pointer
    /// is valid for the lifetime of the emulator and DMA only accesses
    /// conventional memory (< 16MB).
    fn mem_read_physical_dma(&self, addr: u32, len: u32, buffer: &mut [u8]) {
        for i in 0..(len as usize).min(buffer.len()) {
            buffer[i] = self.read_memory_byte(addr as usize + i);
        }
    }

    /// Write physical memory for DMA transfer.
    fn mem_write_physical_dma(&mut self, addr: u32, len: u32, buffer: &[u8]) {
        for (i, &byte) in buffer[..(len as usize).min(buffer.len())].iter().enumerate() {
            self.write_memory_byte(addr as usize + i, byte);
        }
    }

    // -----------------------------------------------------------------------
    // I/O port read handler (Bochs dma.cc)
    // -----------------------------------------------------------------------

    /// Read from DMA I/O port (Bochs dma.cc)
    pub fn read(&mut self, address: u16, _io_len: u8) -> u32 {
        let ma_sl: usize = if address >= 0xC0 { 1 } else { 0 };

        match address {
            // Current address registers (Bochs dma.cc)
            0x00 | 0x02 | 0x04 | 0x06 | 0xC0 | 0xC4 | 0xC8 | 0xCC => {
                let channel =
                    ((address >> (1 + ma_sl as u16)) & 0x03) as usize;
                if !self.s[ma_sl].flip_flop {
                    self.s[ma_sl].flip_flop = true;
                    (self.s[ma_sl].chan[channel].current_address & 0xFF) as u32
                } else {
                    self.s[ma_sl].flip_flop = false;
                    (self.s[ma_sl].chan[channel].current_address >> 8) as u32
                }
            }

            // Current count registers (Bochs dma.cc)
            0x01 | 0x03 | 0x05 | 0x07 | 0xC2 | 0xC6 | 0xCA | 0xCE => {
                let channel =
                    ((address >> (1 + ma_sl as u16)) & 0x03) as usize;
                if !self.s[ma_sl].flip_flop {
                    self.s[ma_sl].flip_flop = true;
                    (self.s[ma_sl].chan[channel].current_count & 0xFF) as u32
                } else {
                    self.s[ma_sl].flip_flop = false;
                    (self.s[ma_sl].chan[channel].current_count >> 8) as u32
                }
            }

            // Status register (Bochs dma.cc)
            0x08 | 0xD0 => {
                let retval = self.s[ma_sl].status_reg;
                self.s[ma_sl].status_reg &= 0xF0; // Clear TC bits on read
                retval as u32
            }

            // Temporary register (Bochs dma.cc)
            0x0D | 0xDA => {
                tracing::trace!(
                    "DMA-{}: read of temporary register always returns 0",
                    ma_sl + 1
                );
                0
            }

            // DMA1 page registers (Bochs dma.cc)
            0x0081 | 0x0082 | 0x0083 | 0x0087 => {
                let channel = CHANNEL_INDEX[(address - 0x81) as usize] as usize;
                self.s[0].chan[channel].page_reg as u32
            }

            // DMA2 page registers (Bochs dma.cc)
            0x0089 | 0x008A | 0x008B | 0x008F => {
                let channel = CHANNEL_INDEX[(address - 0x89) as usize] as usize;
                self.s[1].chan[channel].page_reg as u32
            }

            // Extra page registers (Bochs dma.cc)
            0x0080 | 0x0084 | 0x0085 | 0x0086 | 0x0088 | 0x008C | 0x008D | 0x008E => {
                tracing::trace!(
                    "DMA: read extra page register {:#06x} (unused)",
                    address
                );
                self.ext_page_reg[(address & 0x0F) as usize] as u32
            }

            // Read all mask bits (undocumented, Bochs dma.cc)
            0x0F | 0xDE => {
                let retval = (self.s[ma_sl].mask[0] as u8)
                    | ((self.s[ma_sl].mask[1] as u8) << 1)
                    | ((self.s[ma_sl].mask[2] as u8) << 2)
                    | ((self.s[ma_sl].mask[3] as u8) << 3);
                (0xF0 | retval) as u32
            }

            _ => {
                tracing::trace!("DMA: read unsupported address={:#06x}", address);
                0
            }
        }
    }

    // -----------------------------------------------------------------------
    // I/O port write handler (Bochs dma.cc)
    // -----------------------------------------------------------------------

    /// Write to DMA I/O port (Bochs dma.cc)
    pub fn write(&mut self, address: u16, value: u32, io_len: u8) {
        // Handle word write to mode register (Bochs dma.cc)
        if io_len > 1 {
            if io_len == 2 && address == 0x0B {
                self.write(address, value & 0xFF, 1);
                self.write(address + 1, value >> 8, 1);
                return;
            }
            tracing::trace!(
                "DMA: io write to address {:#010x}, len={}",
                address,
                io_len
            );
            return;
        }

        let value = value as u8;
        let ma_sl: usize = if address >= 0xC0 { 1 } else { 0 };

        match address {
            // Address registers (Bochs dma.cc)
            0x00 | 0x02 | 0x04 | 0x06 | 0xC0 | 0xC4 | 0xC8 | 0xCC => {
                let channel =
                    ((address >> (1 + ma_sl as u16)) & 0x03) as usize;
                if !self.s[ma_sl].flip_flop {
                    // 1st byte
                    self.s[ma_sl].chan[channel].base_address = value as u16;
                    self.s[ma_sl].chan[channel].current_address = value as u16;
                } else {
                    // 2nd byte
                    self.s[ma_sl].chan[channel].base_address |= (value as u16) << 8;
                    self.s[ma_sl].chan[channel].current_address |= (value as u16) << 8;
                }
                self.s[ma_sl].flip_flop = !self.s[ma_sl].flip_flop;
            }

            // Count registers (Bochs dma.cc)
            0x01 | 0x03 | 0x05 | 0x07 | 0xC2 | 0xC6 | 0xCA | 0xCE => {
                let channel =
                    ((address >> (1 + ma_sl as u16)) & 0x03) as usize;
                if !self.s[ma_sl].flip_flop {
                    // 1st byte
                    self.s[ma_sl].chan[channel].base_count = value as u16;
                    self.s[ma_sl].chan[channel].current_count = value as u16;
                } else {
                    // 2nd byte
                    self.s[ma_sl].chan[channel].base_count |= (value as u16) << 8;
                    self.s[ma_sl].chan[channel].current_count |= (value as u16) << 8;
                }
                self.s[ma_sl].flip_flop = !self.s[ma_sl].flip_flop;
            }

            // Command register (Bochs dma.cc)
            0x08 | 0xD0 => {
                if (value & 0xFB) != 0x00 {
                    tracing::trace!(
                        "DMA: write to command register: value {:#04x} not supported",
                        value
                    );
                }
                self.s[ma_sl].command_reg = value;
                self.s[ma_sl].ctrl_disabled = (value >> 2) & 0x01 != 0;
                self.control_hrq(ma_sl);
            }

            // Request register (Bochs dma.cc)
            0x09 | 0xD2 => {
                let channel = (value & 0x03) as usize;
                if value & 0x04 != 0 {
                    // Set request bit in status reg
                    self.s[ma_sl].status_reg |= 1 << (channel + 4);
                    tracing::trace!(
                        "DMA-{}: set request bit for channel {}",
                        ma_sl + 1,
                        channel
                    );
                } else {
                    // Clear request bit in status reg
                    self.s[ma_sl].status_reg &= !(1 << (channel + 4));
                    tracing::trace!(
                        "DMA-{}: cleared request bit for channel {}",
                        ma_sl + 1,
                        channel
                    );
                }
                self.control_hrq(ma_sl);
            }

            // Single mask register (Bochs dma.cc)
            0x0A | 0xD4 => {
                let channel = (value & 0x03) as usize;
                self.s[ma_sl].mask[channel] = (value & 0x04) != 0;
                tracing::trace!(
                    "DMA-{}: set_mask_bit={}, channel={}, mask now={:#04x}",
                    ma_sl + 1,
                    (value & 0x04) != 0,
                    channel,
                    self.s[ma_sl].mask[channel] as u8
                );
                self.control_hrq(ma_sl);
            }

            // Mode register (Bochs dma.cc)
            0x0B | 0xD6 => {
                let channel = (value & 0x03) as usize;
                self.s[ma_sl].chan[channel].mode.mode_type = (value >> 6) & 0x03;
                self.s[ma_sl].chan[channel].mode.address_decrement = (value >> 5) & 0x01 != 0;
                self.s[ma_sl].chan[channel].mode.autoinit_enable = (value >> 4) & 0x01 != 0;
                self.s[ma_sl].chan[channel].mode.transfer_type = (value >> 2) & 0x03;
                tracing::trace!(
                    "DMA-{}: mode register[{}] = {:#04x}",
                    ma_sl + 1,
                    channel,
                    value
                );
            }

            // Clear byte flip/flop (Bochs dma.cc)
            0x0C | 0xD8 => {
                tracing::trace!("DMA-{}: clear flip/flop", ma_sl + 1);
                self.s[ma_sl].flip_flop = false;
            }

            // Master clear (Bochs dma.cc)
            0x0D | 0xDA => {
                tracing::trace!("DMA-{}: master clear", ma_sl + 1);
                self.reset_controller(ma_sl);
            }

            // Clear mask register (Bochs dma.cc)
            0x0E | 0xDC => {
                tracing::trace!("DMA-{}: clear mask register", ma_sl + 1);
                self.s[ma_sl].mask = [false, false, false, false];
                self.control_hrq(ma_sl);
            }

            // Write all mask bits (Bochs dma.cc)
            0x0F | 0xDE => {
                tracing::trace!("DMA-{}: write all mask bits", ma_sl + 1);
                let mut v = value;
                self.s[ma_sl].mask[0] = v & 0x01 != 0;
                v >>= 1;
                self.s[ma_sl].mask[1] = v & 0x01 != 0;
                v >>= 1;
                self.s[ma_sl].mask[2] = v & 0x01 != 0;
                v >>= 1;
                self.s[ma_sl].mask[3] = v & 0x01 != 0;
                self.control_hrq(ma_sl);
            }

            // DMA1 page registers (Bochs dma.cc)
            0x81 | 0x82 | 0x83 | 0x87 => {
                let channel = CHANNEL_INDEX[(address - 0x81) as usize] as usize;
                self.s[0].chan[channel].page_reg = value;
                tracing::trace!("DMA-1: page register {} = {:#04x}", channel, value);
            }

            // DMA2 page registers (Bochs dma.cc)
            0x89 | 0x8A | 0x8B | 0x8F => {
                let channel = CHANNEL_INDEX[(address - 0x89) as usize] as usize;
                self.s[1].chan[channel].page_reg = value;
                tracing::trace!("DMA-2: page register {} = {:#04x}", channel + 4, value);
            }

            // Extra page registers (Bochs dma.cc)
            0x0080 | 0x0084 | 0x0085 | 0x0086 | 0x0088 | 0x008C | 0x008D | 0x008E => {
                tracing::trace!("DMA: write extra page register {:#06x} (unused)", address);
                self.ext_page_reg[(address & 0x0F) as usize] = value;
            }

            _ => {
                tracing::trace!(
                    "DMA: write ignored: {:#06x} = {:#04x}",
                    address,
                    value
                );
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dma_creation() {
        let dma = BxDmaC::new();
        assert_eq!(dma.s[0].mask, [true, true, true, true]);
        assert_eq!(dma.s[1].mask, [true, true, true, true]);
    }

    #[test]
    fn test_dma_init_cascade() {
        let mut dma = BxDmaC::new();
        dma.init();
        // Channel 4 (s[1].chan[0]) should be marked as used (cascade)
        assert!(dma.s[1].chan[0].used);
    }

    #[test]
    fn test_dma_reset_controller() {
        let mut dma = BxDmaC::new();
        dma.init();
        dma.s[0].mask = [false, false, false, false];
        dma.s[0].status_reg = 0xFF;
        dma.reset_controller(0);
        assert_eq!(dma.s[0].mask, [true, true, true, true]);
        assert_eq!(dma.s[0].status_reg, 0);
        assert!(!dma.s[0].ctrl_disabled);
    }

    #[test]
    fn test_dma_address_count_write_read() {
        let mut dma = BxDmaC::new();
        dma.init();

        // Write address for DMA1 channel 0 (port 0x00): low byte then high byte
        dma.write(0x00, 0x34, 1); // low byte
        dma.write(0x00, 0x12, 1); // high byte
        assert_eq!(dma.s[0].chan[0].base_address, 0x1234);
        assert_eq!(dma.s[0].chan[0].current_address, 0x1234);

        // Clear flip-flop, then read back
        dma.write(0x0C, 0, 1); // clear flip-flop
        let lo = dma.read(0x00, 1);
        let hi = dma.read(0x00, 1);
        assert_eq!(lo, 0x34);
        assert_eq!(hi, 0x12);
    }

    #[test]
    fn test_dma_mode_register() {
        let mut dma = BxDmaC::new();
        dma.init();

        // Write mode for channel 2: single mode, read, autoinit, increment
        // Channel = 2 (bits 0-1)
        // Transfer type = 2 (read, bits 2-3)
        // Autoinit = 1 (bit 4)
        // Decrement = 0 (bit 5)
        // Mode type = 1 (single, bits 6-7)
        let mode_val = 0x02 | (0x02 << 2) | (0x01 << 4) | (0x01 << 6); // 0x5A
        dma.write(0x0B, mode_val as u32, 1);

        assert_eq!(dma.s[0].chan[2].mode.mode_type, 1); // single
        assert_eq!(dma.s[0].chan[2].mode.transfer_type, 2); // read
        assert!(dma.s[0].chan[2].mode.autoinit_enable);
        assert!(!dma.s[0].chan[2].mode.address_decrement);
    }

    #[test]
    fn test_dma_status_register_read_clears_tc() {
        let mut dma = BxDmaC::new();
        dma.init();

        // Set some TC bits in status register
        dma.s[0].status_reg = 0xF5; // TC bits = 0x05, request bits = 0xF0
        let status = dma.read(0x08, 1);
        assert_eq!(status, 0xF5);
        // Lower 4 bits (TC) should be cleared after read
        assert_eq!(dma.s[0].status_reg, 0xF0);
    }

    #[test]
    fn test_dma_page_registers() {
        let mut dma = BxDmaC::new();
        dma.init();

        // Write page register for DMA1 channel 2 (port 0x81)
        dma.write(0x81, 0x56, 1);
        assert_eq!(dma.s[0].chan[2].page_reg, 0x56);
        assert_eq!(dma.read(0x81, 1), 0x56);

        // Write page register for DMA1 channel 0 (port 0x87)
        dma.write(0x87, 0xAB, 1);
        assert_eq!(dma.s[0].chan[0].page_reg, 0xAB);
        assert_eq!(dma.read(0x87, 1), 0xAB);
    }

    #[test]
    fn test_dma_mask_register() {
        let mut dma = BxDmaC::new();
        dma.init();

        // Clear all masks
        dma.write(0x0E, 0, 1);
        assert_eq!(dma.s[0].mask, [false, false, false, false]);

        // Set mask for channel 1
        dma.write(0x0A, 0x05, 1); // channel 1, set mask (bit 2 set)
        assert!(dma.s[0].mask[1]);
        assert!(!dma.s[0].mask[0]);

        // Write all mask bits
        dma.write(0x0F, 0x0A, 1); // mask channels 1 and 3
        assert!(!dma.s[0].mask[0]);
        assert!(dma.s[0].mask[1]);
        assert!(!dma.s[0].mask[2]);
        assert!(dma.s[0].mask[3]);

        // Read all mask bits (undocumented)
        let mask_val = dma.read(0x0F, 1);
        assert_eq!(mask_val & 0x0F, 0x0A);
        assert_eq!(mask_val & 0xF0, 0xF0); // upper nibble always 0xF0
    }

    #[test]
    fn test_dma_extra_page_registers() {
        let mut dma = BxDmaC::new();
        dma.init();

        dma.write(0x80, 0x42, 1);
        assert_eq!(dma.ext_page_reg[0], 0x42);
        assert_eq!(dma.read(0x80, 1), 0x42);

        dma.write(0x84, 0x99, 1);
        assert_eq!(dma.ext_page_reg[4], 0x99);
        assert_eq!(dma.read(0x84, 1), 0x99);
    }

    #[test]
    fn test_dma_get_tc() {
        let dma = BxDmaC::new();
        assert!(!dma.get_tc());
    }

    #[test]
    fn test_dma_register_channel() {
        let mut dma = BxDmaC::new();
        dma.init();

        fn dummy_read(_data: &[u8], _maxlen: u16) -> u16 {
            0
        }
        fn dummy_write(_data: &mut [u8], _maxlen: u16) -> u16 {
            0
        }

        assert!(dma.register_dma8_channel(2, dummy_read, dummy_write, "floppy"));
        assert!(dma.s[0].chan[2].used);

        // Trying to register again should fail
        assert!(!dma.register_dma8_channel(2, dummy_read, dummy_write, "floppy2"));

        // Unregister
        assert!(dma.unregister_dma_channel(2));
        assert!(!dma.s[0].chan[2].used);
    }
}
