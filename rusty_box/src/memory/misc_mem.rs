use alloc::vec::Vec;
use core::ffi::c_void;

use crate::{
    config::BxPhyAddress,
    cpu::{rusty_box::MemoryAccessType, BxCpuC, BxCpuIdTrait},
    memory::{
        memory_rusty_box::{bios_map_last128k, MemoryAreaT, BIOSROMSZ, BIOS_MASK, EXROM_MASK},
        BxMemC, BxMemoryStubC,
    },
};

use super::Result;

pub(super) const FLASH_READ_ARRAY: u8 = 0xff;
pub(super) const FLASH_INT_ID: u8 = 0x90;
pub(super) const FLASH_READ_STATUS: u8 = 0x70;
pub(super) const FLASH_CLR_STATUS: u8 = 0x50;
pub(super) const FLASH_ERASE_SETUP: u8 = 0x20;
pub(super) const FLASH_ERASE_SUSP: u8 = 0xb0;
pub(super) const FLASH_PROG_SETUP: u8 = 0x40;
pub(super) const FLASH_ERASE: u8 = 0xd0;

const BX_PHY_ADDRESS_WIDTH: u64 = 40;
const BX_MEM_HANDLERS: usize = ((1u64 << BX_PHY_ADDRESS_WIDTH) >> 20) as usize;

impl BxMemC<'_> {
    pub fn new(mem_stub: BxMemoryStubC, pci_enabled: bool) -> Self {
        let mut memory_handlers = Vec::with_capacity(BX_MEM_HANDLERS);
        for _ in 0..BX_MEM_HANDLERS {
            memory_handlers.push(None);
        }

        let memory_type: [[bool; 2]; 13] = [[false, false]; 13];

        Self {
            inherited_memory_stub: mem_stub,
            smram_available: false,
            smram_enable: false,
            smram_restricted: false,
            memory_handlers,

            pci_enabled,
            bios_write_enabled: false,
            bios_rom_addr: 0xffff0000,
            flash_type: 0,
            flash_status: 0x80,
            flash_wsm_state: FLASH_READ_ARRAY,
            flash_modified: false,
            rom_present: [false; 65],
            memory_type,

            bios_rom_access: 0, // idk tbh

            // A20 enabled by default (full 64-bit addressing)
            a20_mask: 0xFFFF_FFFF_FFFF_FFFFu64,
        }
    }
}

impl<'c> BxMemC<'c> {
    pub(crate) fn get_host_mem_addr<I: BxCpuIdTrait>(
        &mut self,
        // cpu_option: Option<&'c BxCpuC<I>>,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        cpus: &[&BxCpuC<I>],
    ) -> Result<Option<&mut [u8]>> {
        // Debug logging for stack address range
        if addr >= 0xfffffb80 && addr <= 0xfffffc00 {
            tracing::error!("🔎 get_host_mem_addr ENTRY: addr={:#x}, rw={:?}", addr, rw);
        }
        // Only log on trace level to avoid spam - debug was too verbose
        // tracing::trace!("get_host_mem_addr addr: {addr:?} ({:#x}) rw: {rw:?}", addr);
        let a20_addr: BxPhyAddress = self.a20_addr(addr);
        // tracing::trace!("after A20 masking: {a20_addr:?} ({:#x})", a20_addr);

        // Match original Bochs: is_bios = (a20addr >= bios_rom_addr)
        // From cpp_orig/bochs/memory/misc_mem.cc:5
        let mut is_bios = a20_addr >= self.bios_rom_addr.into();

        #[cfg(feature = "bx_phy_address_long")]
        if a20_addr > 0xffffffffu64 {
            is_bios = false;
        }

        let write: bool = (rw as u32 & 1) != 0;

        // allow direct access to SMRAM memory space for code and veto data
        if let Some(cpu) = cpus.first() {
            // reading from SMRAM memory space
            if (a20_addr >= 0x000a0000 && a20_addr < 0x000c0000) && (self.smram_available) {
                if self.smram_enable || cpu.smm_mode() {
                    return Ok(Some(self.get_vector(cpus, a20_addr)?));
                }
            }
        }

        #[cfg(feature = "bx_support_monitor_mwait")]
        if write && Self::is_monitor(cpus, a20_addr & !(0xfff as BxPhyAddress), 0xfff) {
            // Vetoed! Write monitored page !
            return Ok(None);
        }

        // Check memory handlers BEFORE vetoing VGA memory
        // Based on BX_MEM_C::readPhysicalPage/writePhysicalPage in memory.cc
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                // Traverse the handler linked list
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    // Check if the address is within this handler's range
                    if handler.begin <= a20_addr && handler.end >= a20_addr {
                        // Try direct access handler first (for get_host_mem_addr)
                        if let Some(da_handler) = handler.da_handler {
                            return Ok(Some(da_handler(
                                &handler.param,
                                a20_addr,
                                rw,
                                &handler.param,
                            )));
                        }
                        // If no direct access handler, the read/write handlers will be called
                        // from readPhysicalPage/writePhysicalPage methods
                        // For get_host_mem_addr, return None to veto (handler will process it)
                        if !write && (handler.read_handler as usize) != 0 {
                            return Ok(None); // Vetoed - handler will process via readPhysicalPage
                        }
                        if write && (handler.write_handler as usize) != 0 {
                            return Ok(None); // Vetoed - handler will process via writePhysicalPage
                        }
                    }
                    // Move to next handler in the list
                    current_handler = handler.next.as_ref().map(|b| b.as_ref());
                }
            }
        }

        if !write {
            if a20_addr >= 0x000a0000 && a20_addr < 0x000c0000 {
                // VGA memory area - vetoed (no handler registered)
                Ok(None)
            } else if (a20_addr & 0xfffe0000) == 0x000e0000 {
                // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF - handle first
                // Matching C++ line 737-739 and 721: return (Bit8u *) &BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
                // The C++ code uses BIOS_MAP_LAST128K(a20addr) directly where a20addr is pAddrFetchPage (page-aligned).
                // When CPU accesses bytes, it uses eipFetchPtr[pageOffset], which correctly accesses the actual byte.
                // BIOS_MAP_LAST128K(0xFF000) + 0x55A = BIOS_MAP_LAST128K(0xFF55A), so this works correctly.
                let mapped = bios_map_last128k(a20_addr.try_into()?);
                let final_offset = mapped;
                let rom = self.inherited_memory_stub.rom();
                // Log access to 0xFFFF0 (reset vector) for debugging
                if a20_addr == 0xFF000 {
                    let bios_load_offset = (self.bios_rom_addr as usize) & (BIOSROMSZ - 1);
                    let offset_from_bios = final_offset - bios_load_offset;
                    if offset_from_bios < rom.len() - bios_load_offset {
                        let check_offset = bios_load_offset + offset_from_bios;
                        if check_offset + 0xFF0 < rom.len() {
                            let reset_vector_bytes = &rom[check_offset + 0xFF0..check_offset + 0xFF0 + 16];
                            tracing::info!(
                                "get_host_mem_addr: a20_addr={:#x}, mapped={:#x}, final_offset={:#x}, offset_from_bios={:#x}, reset_vector_bytes={:02x?}",
                                a20_addr, mapped, final_offset, offset_from_bios, reset_vector_bytes
                            );
                        }
                    }
                }
                Ok(Some(
                    &mut self.inherited_memory_stub.rom()[final_offset..],
                ))
            } else if cfg!(feature = "bx_support_pci")
                && self.pci_enabled
                && (a20_addr >= 0x000c0000 && a20_addr < 0x00100000)
            {
                let mut area: usize = ((a20_addr as u32 >> 14) & 0x0f).try_into()?;
                if area > MemoryAreaT::F0000 as _ {
                    area = MemoryAreaT::F0000 as _;
                }
                if self.memory_type[area][0] == false {
                    // Read from ROM
                    let to_return = &mut self.inherited_memory_stub.rom()[((a20_addr
                        & EXROM_MASK as BxPhyAddress)
                        + BIOSROMSZ as BxPhyAddress)
                        .try_into()?..];
                    Ok(Some(to_return))
                } else {
                    // Read from ShadowRAM
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                }
            } else if (a20_addr < self.inherited_memory_stub.len.try_into()?) && !is_bios {
                if a20_addr < 0x000c0000 || a20_addr >= 0x00100000 {
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                }
                // must be in C0000 - FFFFF range
                // Matching C++ line 737-743: check for 0xE0000-0xFFFFF range first
                else if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                    // Matching C++ line 739: return (Bit8u *) &BX_MEM_THIS rom[BIOS_MAP_LAST128K(a20addr)];
                    let mapped = bios_map_last128k(a20_addr.try_into()?);
                    let final_offset = mapped;
                    Ok(Some(
                        &mut self.inherited_memory_stub.rom()[final_offset..],
                    ))
                } else {
                    // non-last-128K ROM (C0000-DFFFF)
                    // Matching C++ line 742: return((Bit8u *) &BX_MEM_THIS rom[(a20addr & EXROM_MASK) + BIOSROMSZ]);
                    Ok(Some(
                        &mut self.inherited_memory_stub.rom()[((a20_addr
                            & EXROM_MASK as BxPhyAddress)
                            + BIOSROMSZ as BxPhyAddress)
                            .try_into()?..],
                    ))
                }
            } else if cfg!(feature = "bx_phy_address_long") && a20_addr > 0xffffffffu64 {
                // Error, requested addr is out of bounds.
                Ok(Some(
                    &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff).try_into()?..],
                ))
            } else if is_bios {
                // BIOS ROM access
                Ok(Some(
                    &mut self.inherited_memory_stub.rom()
                        [(a20_addr & BIOS_MASK as BxPhyAddress).try_into()?..],
                ))
            } else if a20_addr >= self.inherited_memory_stub.len.try_into()? {
                // Out of bounds but not BIOS - use bogus buffer for consistency with writes
                // This handles stack at high addresses like 0xfffffb84 with limited RAM
                if a20_addr >= 0xfffffb80 && a20_addr <= 0xfffffc00 {
                    let bogus_off = (a20_addr & 0xfff) as usize;
                    tracing::error!("📖 get_host_mem_addr READ: addr={:#x}, bogus_offset={:#x}, returning bogus buffer",
                        a20_addr, bogus_off);
                }
                Ok(Some(
                    &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff).try_into()?..],
                ))
            } else {
                // Should not reach here, but return bogus as fallback
                Ok(Some(
                    &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff).try_into()?..],
                ))
            }
        } else {
            // op == {BX_WRITE, BX_RW}
            // Match original Bochs: if ((a20addr >= len) || is_bios) return NULL
            // From cpp_orig/bochs/memory/misc_mem.cc:95-96
            if (a20_addr >= self.inherited_memory_stub.len.try_into()?) || is_bios {
                // Writes beyond RAM or to BIOS ROM are vetoed
                Ok(None) // Error, requested addr is out of bounds.
            } else if a20_addr >= 0x000a0000 && a20_addr < 0x000c0000 {
                Ok(None) // Vetoed!  Mem mapped IO (VGA)
            } else if cfg!(feature = "bx_support_pci")
                && (self.pci_enabled && (a20_addr >= 0x000c0000 && a20_addr < 0x00100000))
            {
                // Veto direct writes to this area. Otherwise, there is a chance
                // for Guest2HostTLB and memory consistency problems, for example
                // when some 16K block marked as write-only using PAM registers.
                Ok(None)
            } else {
                if a20_addr < 0x000c0000 || a20_addr >= 0x00100000 {
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                } else {
                    Ok(None) // Vetoed!  ROMs
                }
            }
        }
    }
}

impl BxMemC<'_> {
    pub fn load_ROM(
        &mut self,
        rom_data: &[u8],
        rom_address: BxPhyAddress,
        rom_type: u8,
    ) -> Result<()> {
        use crate::memory::error::MemoryError;
        let size = rom_data.len();
        if size == 0 {
            return Err(MemoryError::RomTooLarge(0).into());
        }
        if rom_type == 0 {
            // system BIOS
            // Matching C++ line 365: offset = romaddress & BIOS_MASK;
            let offset = (rom_address as usize) & (BIOSROMSZ - 1);
            let rom = self.inherited_memory_stub.rom();
            if offset + size > rom.len() {
                return Err(MemoryError::RomTooLarge(rom.len()).into());
            }
            rom[offset..offset + size].copy_from_slice(rom_data);
            self.bios_rom_addr = rom_address as u32;
            for i in 64..65 {
                self.rom_present[i] = true;
            }
            tracing::info!(
                "BIOS loaded: rom_address={:#x}, offset={:#x}, size={}, bios_rom_addr={:#x}",
                rom_address, offset, size, self.bios_rom_addr
            );
            // Verify first few bytes are not all zeros
            if size > 16 {
                let first_bytes = &rom[offset..offset + 16];
                let all_zeros = first_bytes.iter().all(|&b| b == 0);
                if all_zeros {
                    tracing::error!(
                        "BIOS first 16 bytes at offset {:#x} are ALL ZEROS! BIOS may not be loaded correctly.",
                        offset
                    );
                } else {
                    tracing::info!(
                        "BIOS first 16 bytes at offset {:#x}: {:02x?}",
                        offset,
                        first_bytes
                    );
                }
            }
            // Also verify bytes at a few key locations
            // Check bytes at 0xFF55A (offset 0x155A from BIOS start)
            if size > 0x155A {
                let check_offset = offset + 0x155A;
                if check_offset < rom.len() {
                    let check_bytes = &rom[check_offset..check_offset + 16.min(rom.len() - check_offset)];
                    tracing::info!(
                        "BIOS bytes at offset {:#x} (corresponds to 0xFF55A): {:02x?}",
                        check_offset,
                        check_bytes
                    );
                }
            }
            // Check bytes at 0xFFFF0 (last 16 bytes of BIOS) - this is where the reset vector should be
            if size > 0x1FFF0 {
                let check_offset = offset + 0x1FFF0;
                if check_offset < rom.len() {
                    let check_bytes = &rom[check_offset..check_offset + 16.min(rom.len() - check_offset)];
                    tracing::info!(
                        "BIOS bytes at offset {:#x} (corresponds to 0xFFFF0, reset vector): {:02x?}",
                        check_offset,
                        check_bytes
                    );
                    // The reset vector should be: EA 5B E0 00 F0 (ljmp 0xf000:0xe05b)
                    if check_bytes.len() >= 5 {
                        let expected = [0xEA, 0x5B, 0xE0, 0x00, 0xF0];
                        let matches = check_bytes[0..5] == expected;
                        if matches {
                            tracing::info!("Reset vector at 0xFFFF0 is correct!");
                        } else {
                            tracing::warn!(
                                "Reset vector at 0xFFFF0 mismatch! Expected {:02x?}, got {:02x?}",
                                expected,
                                &check_bytes[0..5]
                            );
                        }
                    }
                }
            }
            return Ok(());
        }
        // vga/option roms
        if (size % 512) != 0 {
            return Err(MemoryError::RomSizeNotMultipleOf512.into());
        }
        if (rom_address % 2048) != 0 {
            return Err(MemoryError::RomNot2kAligned.into());
        }
        if rom_address < 0xc0000 {
            return Err(MemoryError::RomAddressOutOfRange.into());
        }
        let offset = if rom_address < 0xe0000 {
            ((rom_address & EXROM_MASK as BxPhyAddress) + BIOSROMSZ as BxPhyAddress) as usize
        } else {
            (rom_address & BIOS_MASK as BxPhyAddress) as usize
        };
        let rom = self.inherited_memory_stub.rom();
        if offset + size > rom.len() {
            return Err(MemoryError::RomTooLarge(rom.len()).into());
        }
        rom[offset..offset + size].copy_from_slice(rom_data);

        // === ROM Content Verification Logging ===
        tracing::info!(
            "ROM loaded: type={}, address={:#x}, size={:#x}, offset={:#x}",
            rom_type, rom_address, size, offset
        );

        // Log first 16 bytes of ROM
        let display_size = 16.min(size);
        tracing::info!(
            "ROM first 16 bytes at offset {:#x}: {:02X?}",
            offset,
            &rom[offset..offset + display_size]
        );

        // For option ROMs (type > 0), check signature and entry point
        if rom_type > 0 {
            if size >= 4 {
                let signature = u16::from_le_bytes([rom[offset], rom[offset + 1]]);
                if signature == 0xAA55 {
                    tracing::info!("✓ Option ROM signature valid (55 AA)");

                    // ROM entry point is at offset +3
                    let init_size_blocks = rom[offset + 2];
                    let init_offset = init_size_blocks as usize * 512;
                    tracing::info!(
                        "  ROM init size: {} blocks ({} bytes)",
                        init_size_blocks,
                        init_offset
                    );

                    // Calculate entry point address
                    let entry_point = rom_address + 3;
                    tracing::info!("  ROM entry point: {:#x}", entry_point);
                } else {
                    tracing::warn!(
                        "⚠ Invalid option ROM signature: {:#04x} (expected 0xAA55)",
                        signature
                    );
                }
            }
        }

        // For system BIOS (type 0), verify reset vector
        if rom_type == 0 && offset + 0x1FFF0 + 5 <= rom.len() {
            let reset_vec = &rom[offset + 0x1FFF0..offset + 0x1FFF0 + 5];
            if reset_vec[0] == 0xEA {
                let target_offset = u16::from_le_bytes([reset_vec[1], reset_vec[2]]);
                let target_segment = u16::from_le_bytes([reset_vec[3], reset_vec[4]]);
                tracing::info!(
                    "✓ BIOS reset vector: JMP FAR {:04X}:{:04X}",
                    target_segment,
                    target_offset
                );
            }
        }

        Ok(())
    }

    /// Load optional RAM image into memory
    ///
    /// Based on BX_MEM_C::load_RAM() in misc_mem.cc
    /// This loads a RAM image directly into the memory vector at the specified address.
    /// Unlike ROMs, RAM images are loaded into regular memory space (not ROM space).
    ///
    /// # Arguments
    /// * `ram_data` - Raw RAM image data
    /// * `ram_address` - Physical address where to load the RAM image
    pub fn load_RAM(&mut self, ram_data: &[u8], ram_address: BxPhyAddress) -> Result<()> {
        use crate::memory::error::MemoryError;

        let size = ram_data.len();
        if size == 0 {
            return Err(MemoryError::RomTooLarge(0).into());
        }

        // RAM images are loaded directly into memory at the specified address
        // We need to write to the memory vector using get_vector
        let a20_addr = self.a20_addr(ram_address);

        // For simplicity, we'll write directly to the memory stub
        // In the original Bochs, it calls get_vector() which returns a pointer to memory
        // We need to access the memory vector and write at the offset
        let mem_stub = &mut self.inherited_memory_stub;
        let vector = mem_stub.vector();

        let offset = a20_addr as usize;
        if offset + size > vector.len() {
            return Err(MemoryError::RomTooLarge(vector.len()).into());
        }

        vector[offset..offset + size].copy_from_slice(ram_data);

        tracing::info!("ram at {:#05x}/{} ({})", ram_address, size, "RAM image");

        Ok(())
    }

    /// Write physical page with memory handler support
    /// Based on BX_MEM_C::writePhysicalPage in memory.cc
    pub fn write_physical_page<I: BxCpuIdTrait>(
        &mut self,
        cpus: &[&BxCpuC<I>],
        page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<()> {
        let a20_addr = self.a20_addr(addr);

        // Check memory handlers first (before vetoing VGA memory)
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    if handler.begin <= a20_addr && handler.end >= a20_addr {
                        // Call write handler if it exists
                        let write_handler = handler.write_handler;
                        if (write_handler as usize) != 0 {
                            // Call handler once for the entire length (matching original behavior)
                            // The handler will process all bytes internally
                            if write_handler(
                                a20_addr,
                                len as u32,
                                data.as_mut_ptr() as *mut c_void,
                                handler.param,
                            ) {
                                return Ok(()); // Handler processed the write
                            }
                        }
                    }
                    current_handler = handler.next.as_ref().map(|b| b.as_ref());
                }
            }
        }

        // No handler processed it, delegate to stub implementation
        self.inherited_memory_stub.write_physical_page(
            cpus,
            page_write_stamp_table,
            addr,
            len,
            data,
            self.a20_mask,
        )
    }

    /// Read physical page with memory handler support
    /// Based on BX_MEM_C::readPhysicalPage in memory.cc
    pub fn read_physical_page<I: BxCpuIdTrait>(
        &mut self,
        cpus: &[&BxCpuC<I>],
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<()> {
        let a20_addr = self.a20_addr(addr);

        // Check memory handlers first (before vetoing VGA memory)
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    if handler.begin <= a20_addr && handler.end >= a20_addr {
                        // Call read handler if it exists
                        let read_handler = handler.read_handler;
                        if (read_handler as usize) != 0 {
                            // Call handler once for the entire length (matching original behavior)
                            // The handler will process all bytes internally
                            if read_handler(
                                a20_addr,
                                len as u32,
                                data.as_mut_ptr() as *mut c_void,
                                handler.param,
                            ) {
                                return Ok(()); // Handler processed the read
                            }
                        }
                    }
                    current_handler = handler.next.as_ref().map(|b| b.as_ref());
                }
            }
        }

        // No handler processed it, delegate to stub implementation
        self.inherited_memory_stub
            .read_physical_page(cpus, addr, len, data, self.a20_mask)
    }

    /// Register memory handlers for a specific address range
    ///
    /// Based on BX_MEM_C::registerMemoryHandlers in misc_mem.cc
    ///
    /// # Arguments
    /// * `param` - Pointer to device instance (e.g., VGA controller)
    /// * `read_handler` - Function to handle memory reads
    /// * `write_handler` - Function to handle memory writes (can be null)
    /// * `begin_addr` - Start address of the range
    /// * `end_addr` - End address of the range (inclusive)
    pub fn register_memory_handlers(
        &mut self,
        param: *const core::ffi::c_void,
        read_handler: super::MemoryHandlerT,
        write_handler: super::MemoryHandlerT,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> Result<()> {
        use crate::memory::error::MemoryError;

        if end_addr < begin_addr {
            return Err(MemoryError::InvalidAddressRange.into());
        }

        if read_handler as usize == 0 {
            return Err(MemoryError::InvalidHandler.into());
        }

        tracing::info!(
            "Register memory access handlers: {:#x} - {:#x}",
            begin_addr,
            end_addr
        );

        // Register handlers for each 1MB page in the range
        let start_page = (begin_addr >> 20) as usize;
        let end_page = (end_addr >> 20) as usize;

        // Ensure handlers vector is large enough
        let required_len = end_page + 1;
        if required_len > self.memory_handlers.len() {
            // Extend the handlers vector if needed
            let current_len = self.memory_handlers.len();
            self.memory_handlers.reserve(required_len - current_len);
            for _ in current_len..required_len {
                self.memory_handlers.push(None);
            }
        }

        for page_idx in start_page..=end_page {
            // Calculate bitmap for 64KB sub-ranges within this page
            let mut bitmap = 0xFFFFu16;
            let page_base = (page_idx as BxPhyAddress) << 20;

            if begin_addr > page_base {
                let sub_page = ((begin_addr >> 16) & 0xF) as u16;
                bitmap &= 0xFFFFu16 << sub_page;
            }

            if end_addr < page_base + 0x100000 {
                let sub_page = ((end_addr >> 16) & 0xF) as u16;
                bitmap &= 0xFFFFu16 >> (0x0F - sub_page);
            }

            // Check for overlapping handlers
            if let Some(existing) = &self.memory_handlers[page_idx] {
                if (bitmap & existing.bitmap) != 0 {
                    tracing::error!("Register failed: overlapping memory handlers!");
                    return Err(MemoryError::OverlappingHandlers.into());
                }
                bitmap |= existing.bitmap;
            }

            // Create new handler struct
            // Store handler on each page that the range covers
            let handler = super::MemoryHandlerStruct {
                next: self.memory_handlers[page_idx].take().map(Box::new),
                param, // Pointer can be copied
                begin: begin_addr,
                end: end_addr,
                bitmap,
                read_handler,
                write_handler,
                da_handler: None,
            };
            self.memory_handlers[page_idx] = Some(handler);
        }

        Ok(())
    }
}
