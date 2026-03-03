use alloc::{boxed::Box, format, vec::Vec};
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
            bios_write_enabled: true, // Enable BIOS ROM writes (for flash ROM and early stack)
            bios_rom_addr: 0xffff0000,
            flash_type: 0,
            flash_status: 0x80,
            flash_wsm_state: FLASH_READ_ARRAY,
            flash_modified: false,
            rom_present: [false; 65],
            memory_type,

            bios_rom_access: 0, // idk tbh

            // A20 starts DISABLED at boot (synced from PC system during init)
            a20_mask: 0xFFFF_FFFF_FFEF_FFFFu64,
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
        let a20_addr: BxPhyAddress = self.a20_addr(addr);

        // Match Bochs: 0xE0000-0xFFFFF is ALWAYS BIOS ROM, plus addresses >= bios_rom_addr
        // This is critical for rombios32 which is linked to run at 0xE0000!
        // From cpp_orig/bochs/memory/misc_mem.cc:674 and memory-bochs.h:40
        let is_bios =
            (a20_addr >= 0xE0000 && a20_addr < 0x100000) || a20_addr >= self.bios_rom_addr.into();

        #[cfg(feature = "bx_phy_address_long")]
        let is_bios = if a20_addr > 0xffffffffu64 {
            false
        } else {
            is_bios
        };

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
            } else if cfg!(feature = "bx_support_pci")
                && self.pci_enabled
                && (a20_addr >= 0x000c0000 && a20_addr < 0x00100000)
            {
                // PCI path for C0000-FFFFF: check memory_type to decide ROM vs ShadowRAM.
                // Bochs: misc_mem.cc:714-729 — this check MUST come before the unconditional
                // E0000 ROM return, because PAM registers can redirect reads to shadow DRAM.
                let mut area: usize = ((a20_addr as u32 >> 14) & 0x0f).try_into()?;
                if area > MemoryAreaT::F0000 as _ {
                    area = MemoryAreaT::F0000 as _;
                }
                if self.memory_type[area][0] {
                    // Read from ShadowRAM (PAM enabled DRAM reads)
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                } else {
                    // Read from ROM
                    let rom_offset = if (a20_addr & 0xfffe0000) == 0x000e0000 {
                        // Last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                        bios_map_last128k(a20_addr.try_into()?)
                    } else {
                        // Expansion ROM (0xC0000-0xDFFFF)
                        ((a20_addr & EXROM_MASK as BxPhyAddress) + BIOSROMSZ as BxPhyAddress)
                            .try_into()?
                    };
                    Ok(Some(&mut self.inherited_memory_stub.rom()[rom_offset..]))
                }
            } else if (a20_addr < self.inherited_memory_stub.len.try_into()?) && !is_bios {
                // Regular RAM or non-PCI ROM
                if a20_addr < 0x000c0000 || a20_addr >= 0x00100000 {
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                }
                // must be in C0000 - FFFFF range (non-PCI path)
                // Bochs: misc_mem.cc:731-744
                else if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                    let mapped = bios_map_last128k(a20_addr.try_into()?);
                    Ok(Some(&mut self.inherited_memory_stub.rom()[mapped..]))
                } else {
                    // non-last-128K ROM (C0000-DFFFF)
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
            } else if a20_addr >= 0xFEE00000 && a20_addr < 0xFEF00000 {
                // APIC MMIO at 0xFEE00000-0xFEEFFFFF: veto direct access.
                // LAPIC register reads have side effects and must go through
                // the CPU's mem_read_dword → lapic.read() intercept path.
                Ok(None)
            } else if is_bios {
                // High BIOS ROM access (>= bios_rom_addr, e.g. 0xFFFF0000+)
                let rom_offset = bios_map_last128k(a20_addr.try_into()?);
                Ok(Some(&mut self.inherited_memory_stub.rom()[rom_offset..]))
            } else {
                // Out of bounds - return bogus memory (matches Bochs)
                Ok(Some(
                    &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff).try_into()?..],
                ))
            }
        } else {
            // op == {BX_WRITE, BX_RW}
            if a20_addr >= 0xFEE00000 && a20_addr < 0xFEF00000 {
                // APIC MMIO at 0xFEE00000-0xFEEFFFFF: veto direct access.
                // LAPIC register writes have side effects (EOI, ICR, timer, etc.)
                // and must go through the CPU's mem_write_dword → lapic.write() intercept.
                return Ok(None);
            }
            if (a20_addr >= self.inherited_memory_stub.len.try_into()?) || is_bios {
                // Error, requested addr is out of bounds or writing to BIOS ROM
                // From cpp_orig/bochs/memory/misc_mem.cc:763-764
                Ok(None)
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
                rom_address,
                offset,
                size,
                self.bios_rom_addr
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
                    let check_bytes =
                        &rom[check_offset..check_offset + 16.min(rom.len() - check_offset)];
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
                    let check_bytes =
                        &rom[check_offset..check_offset + 16.min(rom.len() - check_offset)];
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
            rom_type,
            rom_address,
            size,
            offset
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
    /// Based on BX_MEM_C::writePhysicalPage in memory.cc:39-175
    pub fn write_physical_page<I: BxCpuIdTrait>(
        &mut self,
        cpus: &[&BxCpuC<I>],
        page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<()> {
        use crate::memory::memory_rusty_box::{bios_map_last128k, MemoryAreaT, BIOSROMSZ};

        let mut a20_addr = self.a20_addr(addr);

        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(super::MemoryError::WritePhysicalPage { addr, len }.into());
        }

        #[cfg(feature = "bx_support_monitor_mwait")]
        Self::is_monitor(cpus, a20_addr, len.try_into()?);

        // Match Bochs: 0xE0000-0xFFFFF is ALWAYS BIOS ROM, plus addresses >= bios_rom_addr
        // This is critical for rombios32 which is linked to run at 0xE0000!
        let is_bios =
            (a20_addr >= 0xE0000 && a20_addr < 0x100000) || a20_addr >= self.bios_rom_addr.into();
        #[cfg(feature = "bx_phy_address_long")]
        let is_bios = if a20_addr > 0xffffffffu64 {
            false
        } else {
            is_bios
        };

        let cpu_opt = cpus.first();

        // Check SMRAM first (before memory handlers)
        if cpu_opt.is_some()
            && (a20_addr >= 0x000a0000 && a20_addr < 0x000c0000)
            && self.smram_available
        {
            if let Some(cpu) = cpu_opt {
                if self.smram_enable || (cpu.smm_mode() && !self.smram_restricted) {
                    // Write to SMRAM - delegate to stub for regular memory write
                    return self.inherited_memory_stub.write_physical_page(
                        cpus,
                        page_write_stamp_table,
                        addr,
                        len,
                        data,
                        self.a20_mask,
                    );
                }
            }
        }

        // Check memory handlers
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    if handler.write_handler as usize != 0
                        && handler.begin <= a20_addr
                        && handler.end >= a20_addr
                    {
                        if (handler.write_handler)(
                            a20_addr,
                            len as u32,
                            data.as_mut_ptr() as *mut c_void,
                            handler.param,
                        ) {
                            return Ok(()); // Handler processed the write
                        }
                    }
                    current_handler = handler.next.as_ref().map(|b| b.as_ref());
                }
            }
        }

        // mem_write: (from memory.cc:85)

        // All memory access fits in single 4K page.
        // Note: Bochs does NOT check is_bios here — addresses in E0000-FFFFF
        // (where is_bios=true) must enter this block to reach the PCI shadow RAM
        // write path. High BIOS addresses (>= bios_rom_addr like 0xFFFF0000) are
        // above RAM len so the `a20_addr < len` check naturally excludes them.
        if a20_addr < self.inherited_memory_stub.len.try_into()? {
            // All of data is within limits of physical memory
            if a20_addr < 0x000a0000 || a20_addr >= 0x00100000 {
                // Log writes to very low RAM (first 4KB) - these might be IVT/BDA initialization
                if a20_addr < 0x1000 {
                    let data_preview = if len <= 8 {
                        format!("{:02x?}", &data[0..len])
                    } else {
                        format!("{:02x?}...", &data[0..8])
                    };
                    tracing::trace!(
                        "💾 LOW_RAM_WRITE: addr={:#x}, len={}, data={}",
                        a20_addr,
                        len,
                        data_preview
                    );
                }
                // Regular RAM - delegate to stub
                return self.inherited_memory_stub.write_physical_page(
                    cpus,
                    page_write_stamp_table,
                    addr,
                    len,
                    data,
                    self.a20_mask,
                );
            }

            // Address must be in range 0x000A0000..0x000FFFFF
            page_write_stamp_table.dec_write_stamp(a20_addr);

            for i in 0..len {
                // SMMRAM (0xA0000-0xBFFFF)
                if a20_addr < 0x000c0000 {
                    // Devices are not allowed to access SMMRAM under VGA memory
                    if cpu_opt.is_some() {
                        let vector = self.get_vector(cpus, a20_addr)?;
                        if let Some(byte) = vector.get_mut(0) {
                            *byte = data[i];
                        }
                    }
                    a20_addr += 1;
                    continue;
                }

                // Adapter ROM (0xC0000..0xDFFFF) and ROM BIOS memory (0xE0000..0xFFFFF)
                #[cfg(feature = "bx_support_pci")]
                if self.pci_enabled && ((a20_addr & 0xfffc0000) == 0x000c0000) {
                    let area = ((a20_addr >> 14) & 0x0f) as usize;
                    let area = area.min(MemoryAreaT::F0000 as usize);

                    if self.memory_type[area][1] {
                        // Writes to ShadowRAM
                        tracing::debug!(
                            "Writing to ShadowRAM: address {:#x}, data {:02x}",
                            a20_addr,
                            data[i]
                        );
                        let vector = self.get_vector(cpus, a20_addr)?;
                        if let Some(byte) = vector.get_mut(0) {
                            *byte = data[i];
                        }
                    } else if (area >= MemoryAreaT::E0000 as usize) && self.bios_write_enabled {
                        // Volatile BIOS write support (flash ROM path)
                        let rom_offset = bios_map_last128k(a20_addr as usize);
                        if rom_offset < BIOSROMSZ {
                            let rom = self.inherited_memory_stub.rom();
                            if let Some(byte) = rom.get_mut(rom_offset) {
                                *byte = data[i];
                            }
                        }
                    } else {
                        // Writes to ROM, Inhibit
                        tracing::debug!(
                            "Write to ROM ignored: address {:#x}, data {:02x}",
                            a20_addr,
                            data[i]
                        );
                    }
                }

                #[cfg(not(feature = "bx_support_pci"))]
                {
                    // Without PCI support, ignore writes to ROM
                    tracing::debug!(
                        "Write to ROM ignored (no PCI): address {:#x}, data {:02x}",
                        a20_addr,
                        data[i]
                    );
                }

                a20_addr += 1;
            }

            return Ok(());
        } else if self.bios_write_enabled && is_bios {
            // Volatile BIOS write support (from memory.cc:151-170)
            for i in 0..len {
                let rom_offset = bios_map_last128k(a20_addr as usize);
                if rom_offset < BIOSROMSZ {
                    let rom = self.inherited_memory_stub.rom();
                    if let Some(byte) = rom.get_mut(rom_offset) {
                        *byte = data[i];
                    }
                }
                a20_addr += 1;
            }
            return Ok(());
        } else {
            // Access outside limits of physical memory, ignore (from memory.cc:172-174)
            tracing::trace!(
                "Write outside the limits of physical memory ({:#x}) (ignore)",
                a20_addr
            );
            return Ok(());
        }
    }

    /// Read physical page with memory handler support
    /// Based on BX_MEM_C::readPhysicalPage in memory.cc:177-334
    pub fn read_physical_page<I: BxCpuIdTrait>(
        &mut self,
        cpus: &[&BxCpuC<I>],
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<()> {
        use crate::memory::memory_rusty_box::{
            bios_map_last128k, MemoryAreaT, BIOSROMSZ, EXROM_MASK,
        };

        let mut a20_addr = self.a20_addr(addr);

        // Note: accesses should always be contained within a single page
        if (addr >> 12) != ((addr + len as u64 - 1) >> 12) {
            return Err(super::MemoryError::ReadPhysicalPage { addr, len }.into());
        }

        // Match Bochs: 0xE0000-0xFFFFF is ALWAYS BIOS ROM, plus addresses >= bios_rom_addr
        // This is critical for rombios32 which is linked to run at 0xE0000!
        let is_bios =
            (a20_addr >= 0xE0000 && a20_addr < 0x100000) || a20_addr >= self.bios_rom_addr.into();
        #[cfg(feature = "bx_phy_address_long")]
        let is_bios = if a20_addr > 0xffffffffu64 {
            false
        } else {
            is_bios
        };

        let cpu_opt = cpus.first();

        // Check SMRAM first (before memory handlers)
        if cpu_opt.is_some()
            && (a20_addr >= 0x000a0000 && a20_addr < 0x000c0000)
            && self.smram_available
        {
            if let Some(cpu) = cpu_opt {
                if self.smram_enable || (cpu.smm_mode() && !self.smram_restricted) {
                    // Read from SMRAM - delegate to stub for regular memory read
                    return self.inherited_memory_stub.read_physical_page(
                        cpus,
                        addr,
                        len,
                        data,
                        self.a20_mask,
                    );
                }
            }
        }

        // Check memory handlers
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    if handler.read_handler as usize != 0
                        && handler.begin <= a20_addr
                        && handler.end >= a20_addr
                    {
                        if (handler.read_handler)(
                            a20_addr,
                            len as u32,
                            data.as_mut_ptr() as *mut c_void,
                            handler.param,
                        ) {
                            #[cfg(feature = "bx_support_pci")]
                            if self.pci_enabled && ((a20_addr & 0xfffc0000) == 0x000c0000) {
                                let area = ((a20_addr >> 14) & 0x0f) as usize;
                                let area = area.min(MemoryAreaT::F0000 as usize);
                                if !self.memory_type[area][0] {
                                    // Read from ROM, not shadow RAM - continue to ROM read below
                                } else {
                                    return Ok(()); // Handler processed the read from shadow RAM
                                }
                            } else {
                                return Ok(()); // Handler processed the read
                            }
                        }
                    }
                    current_handler = handler.next.as_ref().map(|b| b.as_ref());
                }
            }
        }

        // mem_read:
        // Note: Bochs does NOT check is_bios here — addresses in E0000-FFFFF
        // must enter this block to reach the PCI shadow RAM read path.
        if a20_addr < self.inherited_memory_stub.len.try_into()? {
            // All of data is within limits of physical memory
            if a20_addr < 0x000a0000 || a20_addr >= 0x00100000 {
                // Regular RAM - delegate to stub
                return self.inherited_memory_stub.read_physical_page(
                    cpus,
                    addr,
                    len,
                    data,
                    self.a20_mask,
                );
            }

            // Address must be in range 0x000A0000..0x000FFFFF
            for i in 0..len {
                // SMMRAM (0xA0000-0xBFFFF)
                if a20_addr < 0x000c0000 {
                    // Devices are not allowed to access SMMRAM under VGA memory
                    if cpu_opt.is_some() {
                        let vector = self.get_vector(cpus, a20_addr)?;
                        if let Some(byte) = vector.get(0) {
                            data[i] = *byte;
                        }
                    }
                    a20_addr += 1;
                    continue;
                }

                // ROM area (0xC0000..0xFFFFF)
                #[cfg(feature = "bx_support_pci")]
                if self.pci_enabled && ((a20_addr & 0xfffc0000) == 0x000c0000) {
                    let area = ((a20_addr >> 14) & 0x0f) as usize;
                    let area = area.min(MemoryAreaT::F0000 as usize);

                    if !self.memory_type[area][0] {
                        // Read from ROM
                        if (a20_addr & 0xfffe0000) == 0x000e0000 {
                            // Last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                            let rom_offset = bios_map_last128k(a20_addr as usize);
                            if rom_offset < BIOSROMSZ {
                                let rom = self.inherited_memory_stub.rom();
                                if let Some(byte) = rom.get(rom_offset) {
                                    data[i] = *byte;
                                }
                            }
                        } else {
                            // Expansion ROM (0xC0000-0xDFFFF)
                            let rom_offset =
                                ((a20_addr & EXROM_MASK as u64) + BIOSROMSZ as u64) as usize;
                            let rom = self.inherited_memory_stub.rom();
                            if let Some(byte) = rom.get(rom_offset) {
                                data[i] = *byte;
                            }
                        }
                    } else {
                        // Read from ShadowRAM
                        let vector = self.get_vector(cpus, a20_addr)?;
                        if let Some(byte) = vector.get(0) {
                            data[i] = *byte;
                        }
                    }
                }

                #[cfg(not(feature = "bx_support_pci"))]
                {
                    // Without PCI support, read from ROM
                    if (a20_addr & 0xfffc0000) != 0x000c0000 {
                        let vector = self.get_vector(cpus, a20_addr)?;
                        if let Some(byte) = vector.get(0) {
                            data[i] = *byte;
                        }
                    } else if (a20_addr & 0xfffe0000) == 0x000e0000 {
                        // Last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                        let rom_offset = bios_map_last128k(a20_addr as usize);
                        if rom_offset < BIOSROMSZ {
                            let rom = self.inherited_memory_stub.rom();
                            if let Some(byte) = rom.get(rom_offset) {
                                data[i] = *byte;
                            }
                        }
                    } else {
                        // Expansion ROM (0xC0000-0xDFFFF)
                        let rom_offset =
                            ((a20_addr & EXROM_MASK as u64) + BIOSROMSZ as u64) as usize;
                        let rom = self.inherited_memory_stub.rom();
                        if let Some(byte) = rom.get(rom_offset) {
                            data[i] = *byte;
                        }
                    }
                }

                a20_addr += 1;
            }

            return Ok(());
        } else {
            // Access outside limits of physical memory

            #[cfg(feature = "bx_phy_address_long")]
            if a20_addr > 0xffffffffu64 {
                data.fill(0xFF);
                return Ok(());
            }

            if is_bios {
                // Read from BIOS ROM
                for i in 0..len {
                    let rom_offset = bios_map_last128k(a20_addr as usize);
                    if rom_offset < BIOSROMSZ {
                        let rom = self.inherited_memory_stub.rom();
                        if let Some(byte) = rom.get(rom_offset) {
                            data[i] = *byte;
                        } else {
                            data[i] = 0xFF;
                        }
                    } else {
                        data[i] = 0xFF;
                    }
                    a20_addr += 1;
                }
            } else {
                // Bogus memory
                data.fill(0xFF);
            }

            return Ok(());
        }
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
