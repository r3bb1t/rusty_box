#![allow(private_interfaces, dead_code)]
#![allow(non_snake_case)]

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::{
    config::{BxPhyAddress, MAX_HANDLER_OVERFLOW},
    cpu::rusty_box::MemoryAccessType,
    memory::{
        memory_rusty_box::{
            bios_map_last128k, bx_guest_ram_span, MemoryAreaT, BIOSROMSZ, BIOS_MASK, EXROM_MASK,
        },
        BxMemC, BxMemoryStubC, CpuMemoryPolicy, CpuTlbPin,
    },
};

use super::{MemoryError, Result};

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

#[inline]
fn direct_host_write_allowed(
    a20_addr: BxPhyAddress,
    ram_len: usize,
    is_bios: bool,
) -> bool {
    !(0xFEE00000..0xFEF00000).contains(&a20_addr)
        && bx_guest_ram_span(a20_addr, 1, ram_len).is_some()
        && !is_bios
        && !(0x000a0000..0x000c0000).contains(&a20_addr)
        && !(0x000c0000..0x00100000).contains(&a20_addr)
}

impl BxMemC<'_> {
    #[cfg(feature = "alloc")]
    pub fn new(mem_stub: alloc::boxed::Box<BxMemoryStubC>, pci_enabled: bool) -> Self {
        Self::new_inner(*mem_stub, pci_enabled)
    }

    pub fn new_from_stub(mem_stub: BxMemoryStubC, pci_enabled: bool) -> Self {
        Self::new_inner(mem_stub, pci_enabled)
    }

    fn new_inner(mem_stub: BxMemoryStubC, pci_enabled: bool) -> Self {
        let memory_type: [[bool; 2]; 13] = [[false, false]; 13];

        Self {
            inherited_memory_stub: mem_stub,
            smram_available: false,
            smram_enable: false,
            smram_restricted: false,
            #[cfg(feature = "alloc")]
            memory_handlers: {
                let mut v = Vec::with_capacity(BX_MEM_HANDLERS);
                v.resize_with(BX_MEM_HANDLERS, || None);
                v
            },
            #[cfg(not(feature = "alloc"))]
            memory_handlers: [const { None }; 4096],
            handler_overflow: [const { None }; MAX_HANDLER_OVERFLOW],
            handler_overflow_count: 0,

            pci_enabled,
            // Bochs defaults bios_write_enabled to false (misc_mem.cc
            // init_memory); the PIIX3 bridge is the only thing that flips it,
            // via DEV_mem_set_bios_write() when XBCS register 0x4E bit 2 is
            // written (pci2isa.cc case 0x4e), now wired end-to-end through
            // BxPiix3::pci_write -> DeviceManager::bios_write_needs_update ->
            // apply_bios_write_to_memory (devices.rs machine boundary).
            bios_write_enabled: false,
            bios_rom_addr: 0xffff0000,
            flash_type: 0,
            flash_status: 0x80,
            flash_wsm_state: FLASH_READ_ARRAY,
            flash_modified: false,
            rom_present: [false; 65],
            memory_type,

            bios_rom_access: 0,

            // A20 starts DISABLED at boot (synced from PC system during init)
            a20_mask: 0xFFFF_FFFF_FFEF_FFFFu64,
            hpet_access_clock: core::cell::Cell::new((0, 0)),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'c> BxMemC<'c> {
    /// Return a resident, block-bounded host span for an already A20-adjusted
    /// GPA after checked PCI-hole/high-RAM translation.
    fn resident_ram_span<'m>(
        &'m mut self,
        pins: &[CpuTlbPin],
        addr: BxPhyAddress,
    ) -> Result<&'m mut [u8]> {
        let span = bx_guest_ram_span(addr, 1, self.inherited_memory_stub.len)
            .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
        self.inherited_memory_stub.get_vector_offset(span.start, pins)
    }

    /// The sole CPU-facing direct host mapping. Its complete stable pin set
    /// guards eviction; the caller supplies a by-value CPU memory policy.
    pub(crate) fn get_host_mem_addr_pinned(
        &mut self,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        pins: &[CpuTlbPin],
        policy: CpuMemoryPolicy,
    ) -> Result<Option<&mut [u8]>> {
        let a20_addr = self.a20_addr(addr);
        let is_bios = if a20_addr > u64::from(u32::MAX) {
            false
        } else {
            (0xE0000..0x100000).contains(&a20_addr)
                || a20_addr >= BxPhyAddress::from(self.bios_rom_addr)
        };
        let write = (rw as u32 & 1) != 0;

        // Bochs misc_mem.cc getHostMemAddr: "allow direct access to SMRAM
        // memory space for code and veto data". The direct span is handed out
        // for INSTRUCTION FETCH only — data reads/writes deliberately fall
        // through to the VGA memory handler's veto below so they take the slow
        // read/writePhysicalPage path, which applies the stricter
        // `smram_enable || (smm_mode && !smram_restricted)` routing. Note the
        // condition here is the LOOSER one (no `!smram_restricted`), matching
        // Bochs. The `cpu != NULL` guard is structural here: every caller of
        // this function is a CPU path (the DMA paths never take it).
        if rw as u32 == MemoryAccessType::Execute as u32
            && (0x000a0000..0x000c0000).contains(&a20_addr)
            && self.smram_available
            && (self.smram_enable || policy.smm_mode())
        {
            return Ok(Some(self.resident_ram_span(pins, a20_addr)?));
        }

        if write && policy.monitor_hit() {
            return Ok(None);
        }

        // Registered handlers always win over direct RAM.
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler = Some(handler_struct);
                while let Some(handler) = current_handler {
                    if handler.begin <= a20_addr && handler.end >= a20_addr {
                        return Ok(None);
                    }
                    current_handler = handler
                        .next
                        .and_then(|idx| self.handler_overflow[idx as usize].as_ref());
                }
            }
        }

        if !write {
            if (0x000a0000..0x000c0000).contains(&a20_addr) {
                return Ok(None);
            }
            if self.pci_enabled && (0x000c0000..0x00100000).contains(&a20_addr) {
                let mut area = ((a20_addr as u32 >> 14) & 0x0f) as usize;
                if area > MemoryAreaT::F0000 as usize {
                    area = MemoryAreaT::F0000 as usize;
                }
                if self.memory_type[area][0] {
                    return Ok(Some(self.resident_ram_span(pins, a20_addr)?));
                }
                let rom_offset = if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    bios_map_last128k(a20_addr as usize)
                } else {
                    ((a20_addr & EXROM_MASK as BxPhyAddress) + BIOSROMSZ as BxPhyAddress)
                        as usize
                };
                return Ok(Some(&mut self.inherited_memory_stub.rom()[rom_offset..]));
            }
            if bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.len).is_some() && !is_bios
            {
                if !(0x000c0000..0x00100000).contains(&a20_addr) {
                    return Ok(Some(self.resident_ram_span(pins, a20_addr)?));
                }
                if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    let mapped = bios_map_last128k(a20_addr as usize);
                    return Ok(Some(&mut self.inherited_memory_stub.rom()[mapped..]));
                }
                let rom_offset =
                    ((a20_addr & EXROM_MASK as BxPhyAddress) + BIOSROMSZ as BxPhyAddress)
                        as usize;
                return Ok(Some(&mut self.inherited_memory_stub.rom()[rom_offset..]));
            }
            if a20_addr > u64::from(u32::MAX) {
                return Ok(Some(
                    &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff) as usize..],
                ));
            }
            if (0xFEE00000..0xFEF00000).contains(&a20_addr) {
                return Ok(None);
            }
            if is_bios {
                let rom_offset = bios_map_last128k(a20_addr as usize);
                return Ok(Some(&mut self.inherited_memory_stub.rom()[rom_offset..]));
            }
            return Ok(Some(
                &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff) as usize..],
            ));
        }

        if !direct_host_write_allowed(a20_addr, self.inherited_memory_stub.len, is_bios) {
            return Ok(None);
        }
        Ok(Some(self.resident_ram_span(pins, a20_addr)?))
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
            tracing::debug!(
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
                    tracing::debug!(
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
                    tracing::debug!(
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
                    tracing::debug!(
                        "BIOS bytes at offset {:#x} (corresponds to 0xFFFF0, reset vector): {:02x?}",
                        check_offset,
                        check_bytes
                    );
                    // The reset vector should be: EA 5B E0 00 F0 (ljmp 0xf000:0xe05b)
                    if check_bytes.len() >= 5 {
                        let expected = [0xEA, 0x5B, 0xE0, 0x00, 0xF0];
                        let matches = check_bytes[0..5] == expected;
                        if matches {
                            tracing::debug!("Reset vector at 0xFFFF0 is correct!");
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
        if !size.is_multiple_of(512) {
            return Err(MemoryError::RomSizeNotMultipleOf512.into());
        }
        if !rom_address.is_multiple_of(2048) {
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
        tracing::debug!(
            "ROM loaded: type={}, address={:#x}, size={:#x}, offset={:#x}",
            rom_type,
            rom_address,
            size,
            offset
        );

        // Log first 16 bytes of ROM
        let display_size = 16.min(size);
        tracing::debug!(
            "ROM first 16 bytes at offset {:#x}: {:02X?}",
            offset,
            &rom[offset..offset + display_size]
        );

        // For option ROMs (type > 0), check signature and entry point
        if rom_type > 0 && size >= 4 {
            let signature = u16::from_le_bytes([rom[offset], rom[offset + 1]]);
            if signature == 0xAA55 {
                tracing::debug!("✓ Option ROM signature valid (55 AA)");

                // ROM entry point is at offset +3
                let init_size_blocks = rom[offset + 2];
                let init_offset = init_size_blocks as usize * 512;
                tracing::debug!(
                    "  ROM init size: {} blocks ({} bytes)",
                    init_size_blocks,
                    init_offset
                );

                // Calculate entry point address
                let entry_point = rom_address + 3;
                tracing::debug!("  ROM entry point: {:#x}", entry_point);
            } else {
                tracing::warn!(
                    "⚠ Invalid option ROM signature: {:#04x} (expected 0xAA55)",
                    signature
                );
            }
        }

        // For system BIOS (type 0), verify reset vector
        if rom_type == 0 && offset + 0x1FFF0 + 5 <= rom.len() {
            let reset_vec = &rom[offset + 0x1FFF0..offset + 0x1FFF0 + 5];
            if reset_vec[0] == 0xEA {
                let target_offset = u16::from_le_bytes([reset_vec[1], reset_vec[2]]);
                let target_segment = u16::from_le_bytes([reset_vec[3], reset_vec[4]]);
                tracing::debug!(
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
    pub fn load_RAM(
        &mut self,
        pins: &[CpuTlbPin],
        ram_data: &[u8],
        ram_address: BxPhyAddress,
    ) -> Result<()> {
        if ram_data.is_empty() {
            return Err(MemoryError::RamImageOutOfRange.into());
        }
        let copied = self.write_ram(pins, ram_address, ram_data)?;
        if copied != ram_data.len() {
            return Err(MemoryError::RamImageOutOfRange.into());
        }
        tracing::debug!("ram at {:#05x}/{} (RAM image)", ram_address, ram_data.len());
        Ok(())
    }


    /// Write physical page with memory handler support
    /// Based on BX_MEM_C::writePhysicalPage in memory.cc
    pub(crate) fn write_physical_page(
        &mut self,
        pins: &[CpuTlbPin],
        policy: CpuMemoryPolicy,
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


        // Match Bochs: 0xE0000-0xFFFFF is ALWAYS BIOS ROM, plus addresses >= bios_rom_addr
        // This is critical for rombios32 which is linked to run at 0xE0000!
        let is_bios =
            (0xE0000..0x100000).contains(&a20_addr) || a20_addr >= self.bios_rom_addr.into();
        let is_bios = if a20_addr > 0xffffffffu64 {
            false
        } else {
            is_bios
        };

        // Check SMRAM first (before memory handlers).
        let smram_hit = (0x000a0000..0x000c0000).contains(&a20_addr)
            && self.smram_available
            && (self.smram_enable || (policy.smm_mode() && !self.smram_restricted));
        if smram_hit {
            // Write to SMRAM - delegate to stub for regular memory write
            return self.inherited_memory_stub.write_physical_page(
                pins,
                addr,
                len,
                data,
                self.a20_mask,
            );
        }

        // Check memory handlers
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    if handler.begin <= a20_addr && handler.end >= a20_addr {
                        // Bochs: memory_handler->write_handler(a20addr, 1, buf, param)
                        if let Some(vga) = handler.device_id.vga_mut() {
                            vga.mem_write(a20_addr, len as u32, &data[..len]);
                            return Ok(());
                        } else if let Some(ioapic) = handler.device_id.ioapic_mut() {
                            ioapic.mem_write(a20_addr, len as u32, &data[..len]);
                            return Ok(());
                        } else if let Some(hpet) = handler.device_id.hpet_mut() {
                            let (ticks, ips) = self.hpet_access_clock.get();
                            hpet.set_now(ticks, ips);
                            hpet.mem_write(a20_addr, len as u32, &data[..len]);
                            return Ok(());
                        }
                    }
                    current_handler = handler
                        .next
                        .and_then(|idx| self.handler_overflow[idx as usize].as_ref());
                }
            }
        }

        // mem_write: (from memory.cc)

        // All memory access fits in single 4K page.
        // Note: Bochs does NOT check is_bios here — addresses in E0000-FFFFF
        // (where is_bios=true) must enter this block to reach the PCI shadow RAM
        // write path. High BIOS addresses (>= bios_rom_addr like 0xFFFF0000) are
        // above RAM len so the `a20_addr < len` check naturally excludes them.
        if bx_guest_ram_span(a20_addr, len, self.inherited_memory_stub.len).is_some() {
            // All of data is within limits of physical memory
            if !(0x000a0000..0x00100000).contains(&a20_addr) {
                // Log writes to very low RAM (first 4KB) - these might be IVT/BDA initialization
                // Regular RAM - delegate to stub
                return self.inherited_memory_stub.write_physical_page(
                    pins,
                    addr,
                    len,
                    data,
                    self.a20_mask,
                );
            }

            // Address must be in range 0x000A0000..0x000FFFFF
            self.inherited_memory_stub.smc_dec_write_stamp_page(a20_addr);

            for &data_byte in &data[..len] {
                // SMMRAM (0xA0000-0xBFFFF)
                if a20_addr < 0x000c0000 {
                    // Devices are not allowed to access SMMRAM under VGA memory.
                    let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.len)
                        .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                    let vector = self.inherited_memory_stub.get_vector_offset(span.start, pins)?;
                    if let Some(byte) = vector.get_mut(0) {
                        *byte = data_byte;
                    }
                    a20_addr += 1;
                    continue;
                }

                // Adapter ROM (0xC0000..0xDFFFF) and ROM BIOS memory (0xE0000..0xFFFFF)
                if self.pci_enabled && ((a20_addr & 0xfffc0000) == 0x000c0000) {
                    let area = ((a20_addr >> 14) & 0x0f) as usize;
                    let area = area.min(MemoryAreaT::F0000 as usize);

                    if self.memory_type[area][1] {
                        // Writes to ShadowRAM
                        tracing::trace!(
                            "Writing to ShadowRAM: address {:#x}, data {:02x}",
                            a20_addr,
                            data_byte
                        );
                        let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.len)
                            .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                        let vector = self.inherited_memory_stub.get_vector_offset(span.start, pins)?;
                        if let Some(byte) = vector.get_mut(0) {
                            *byte = data_byte;
                        }
                    } else if (area >= MemoryAreaT::E0000 as usize) && self.bios_write_enabled {
                        // Volatile BIOS write support (flash ROM path)
                        let rom_offset = bios_map_last128k(a20_addr as usize);
                        if rom_offset < BIOSROMSZ {
                            let rom = self.inherited_memory_stub.rom();
                            if let Some(byte) = rom.get_mut(rom_offset) {
                                *byte = data_byte;
                            }
                        }
                    } else {
                        // Writes to ROM, Inhibit
                        tracing::trace!(
                            "Write to ROM ignored: address {:#x}, data {:02x}",
                            a20_addr,
                            data_byte
                        );
                    }
                }

                a20_addr += 1;
            }

            Ok(())
        } else if self.bios_write_enabled && is_bios {
            // Volatile BIOS write support (from memory.cc)
            for &data_byte in &data[..len] {
                let rom_offset = bios_map_last128k(a20_addr as usize);
                if rom_offset < BIOSROMSZ {
                    let rom = self.inherited_memory_stub.rom();
                    if let Some(byte) = rom.get_mut(rom_offset) {
                        *byte = data_byte;
                    }
                }
                a20_addr += 1;
            }
            Ok(())
        } else {
            // Access outside limits of physical memory, ignore (from memory.cc)
            Ok(())
        }
    }

    /// Read physical page with memory handler support
    /// Based on BX_MEM_C::readPhysicalPage in memory.cc
    pub(crate) fn read_physical_page(
        &mut self,
        pins: &[CpuTlbPin],
        policy: CpuMemoryPolicy,
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
            (0xE0000..0x100000).contains(&a20_addr) || a20_addr >= self.bios_rom_addr.into();
        let is_bios = if a20_addr > 0xffffffffu64 {
            false
        } else {
            is_bios
        };

        // Check SMRAM first (before memory handlers).
        if (0x000a0000..0x000c0000).contains(&a20_addr)
            && self.smram_available
            && (self.smram_enable || (policy.smm_mode() && !self.smram_restricted))
        {
            // Read from SMRAM - delegate to stub for regular memory read
            return self.inherited_memory_stub.read_physical_page(
                pins,
                addr,
                len,
                data,
                self.a20_mask,
            );
        }

        // Check memory handlers
        let page_idx = (a20_addr >> 20) as usize;
        if page_idx < self.memory_handlers.len() {
            if let Some(handler_struct) = &self.memory_handlers[page_idx] {
                let mut current_handler: Option<&super::MemoryHandlerStruct> = Some(handler_struct);

                while let Some(handler) = current_handler {
                    if handler.begin <= a20_addr && handler.end >= a20_addr {
                        // Bochs: memory_handler->read_handler(a20addr, 1, buf, param)
                        if let Some(vga) = handler.device_id.vga_mut() {
                            vga.mem_read(a20_addr, len as u32, data);
                            return Ok(());
                        } else if let Some(hpet) = handler.device_id.hpet_mut() {
                            let (ticks, ips) = self.hpet_access_clock.get();
                            hpet.set_now(ticks, ips);
                            hpet.mem_read(a20_addr, len as u32, data);
                            return Ok(());
                        } else if let Some(ioapic) = handler.device_id.ioapic_mut() {
                            ioapic.mem_read(a20_addr, len as u32, data);
                            return Ok(());
                        }
                    }
                    current_handler = handler
                        .next
                        .and_then(|idx| self.handler_overflow[idx as usize].as_ref());
                }
            }
        }

        // mem_read:
        // Note: Bochs does NOT check is_bios here — addresses in E0000-FFFFF
        // must enter this block to reach the PCI shadow RAM read path.
        if bx_guest_ram_span(a20_addr, len, self.inherited_memory_stub.len).is_some() {
            // All of data is within limits of physical memory
            if !(0x000a0000..0x00100000).contains(&a20_addr) {
                // Regular RAM - delegate to stub
                return self.inherited_memory_stub.read_physical_page(
                    pins,
                    addr,
                    len,
                    data,
                    self.a20_mask,
                );
            }

            // Address must be in range 0x000A0000..0x000FFFFF
            for data_byte in &mut data[..len] {
                // SMMRAM (0xA0000-0xBFFFF)
                if a20_addr < 0x000c0000 {
                    // Devices are not allowed to access SMMRAM under VGA memory.
                    let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.len)
                        .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                    let vector = self.inherited_memory_stub.get_vector_offset(span.start, pins)?;
                    if let Some(byte) = vector.first() {
                        *data_byte = *byte;
                    }
                    a20_addr += 1;
                    continue;
                }

                // ROM area (0xC0000..0xFFFFF)
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
                                    *data_byte = *byte;
                                }
                            }
                        } else {
                            // Expansion ROM (0xC0000-0xDFFFF)
                            let rom_offset =
                                ((a20_addr & EXROM_MASK as u64) + BIOSROMSZ as u64) as usize;
                            let rom = self.inherited_memory_stub.rom();
                            if let Some(byte) = rom.get(rom_offset) {
                                *data_byte = *byte;
                            }
                        }
                    } else {
                        // Read from ShadowRAM
                        let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.len)
                            .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                        let vector = self.inherited_memory_stub.get_vector_offset(span.start, pins)?;
                        if let Some(byte) = vector.first() {
                            *data_byte = *byte;
                        }
                    }
                }

                a20_addr += 1;
            }

            Ok(())
        } else {
            // Access outside limits of physical memory

            if a20_addr > 0xffffffffu64 {
                data.fill(0xFF);
                return Ok(());
            }

            if is_bios {
                // Read from BIOS ROM
                for data_byte in &mut data[..len] {
                    let rom_offset = bios_map_last128k(a20_addr as usize);
                    if rom_offset < BIOSROMSZ {
                        let rom = self.inherited_memory_stub.rom();
                        if let Some(byte) = rom.get(rom_offset) {
                            *data_byte = *byte;
                        } else {
                            *data_byte = 0xFF;
                        }
                    } else {
                        *data_byte = 0xFF;
                    }
                    a20_addr += 1;
                }
            } else {
                // Bogus memory
                data.fill(0xFF);
            }

            Ok(())
        }
    }

    /// Register a memory-mapped I/O handler for a specific address range.
    ///
    /// Based on BX_MEM_C::registerMemoryHandlers in misc_mem.cc
    ///
    /// # Arguments
    /// * `device_id` - Identifies the device and carries a pointer to its instance
    /// * `begin_addr` - Start address of the range
    /// * `end_addr` - End address of the range (inclusive)
    pub fn register_memory_handlers(
        &mut self,
        device_id: super::MemoryDeviceId,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> Result<()> {
        use crate::memory::error::MemoryError;

        if end_addr < begin_addr {
            return Err(MemoryError::InvalidAddressRange.into());
        }

        tracing::debug!(
            "Register memory access handlers: {:#x} - {:#x}",
            begin_addr,
            end_addr
        );

        // Register handlers for each 1MB page in the range
        let start_page = (begin_addr >> 20) as usize;
        let end_page = (end_addr >> 20) as usize;

        // Ensure handlers array/vec is large enough
        let required_len = end_page + 1;
        #[cfg(feature = "alloc")]
        if required_len > self.memory_handlers.len() {
            let current_len = self.memory_handlers.len();
            self.memory_handlers.reserve(required_len - current_len);
            for _ in current_len..required_len {
                self.memory_handlers.push(None);
            }
        }
        #[cfg(not(feature = "alloc"))]
        assert!(
            required_len <= self.memory_handlers.len(),
            "memory handler page index {} exceeds no-alloc limit {}",
            required_len,
            self.memory_handlers.len()
        );

        for page_idx in start_page..=end_page {
            self.register_page(page_idx, device_id, begin_addr, end_addr)?;
        }

        Ok(())
    }

    /// Return the 64KB-subrange bitmap occupied by a handler on one 1MB page.
    #[inline]
    fn handler_page_bitmap(
        page_idx: usize,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> u16 {
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
        bitmap
    }

    /// Register one 1 MB page's slice of a handler range. Factored out of
    /// `register_memory_handlers` so `unregister_memory_handlers` can rebuild a
    /// page's handler chain from the surviving handlers.
    fn register_page(
        &mut self,
        page_idx: usize,
        device_id: super::MemoryDeviceId,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> Result<()> {
        use crate::memory::error::MemoryError;

        let mut bitmap = Self::handler_page_bitmap(page_idx, begin_addr, end_addr);

        // Check for overlapping handlers
        if let Some(existing) = &self.memory_handlers[page_idx] {
            if (bitmap & existing.bitmap) != 0 {
                tracing::error!("Register failed: overlapping memory handlers!");
                return Err(MemoryError::OverlappingHandlers.into());
            }
            bitmap |= existing.bitmap;
        }

        // If this page already has a handler, move it to the overflow pool
        let next_idx = if let Some(existing) = self.memory_handlers[page_idx].take() {
            let idx = self.alloc_overflow_slot();
            self.handler_overflow[idx] = Some(existing);
            Some(idx as u16)
        } else {
            None
        };

        self.memory_handlers[page_idx] = Some(super::MemoryHandlerStruct {
            next: next_idx,
            begin: begin_addr,
            end: end_addr,
            bitmap,
            device_id,
        });
        Ok(())
    }

    /// Allocate an overflow-pool slot, reusing a freed (`None`) slot before
    /// extending the high-water mark. Without this, repeated register/unregister
    /// cycles (PCI BAR relocation) would leak the fixed 16-entry pool.
    fn alloc_overflow_slot(&mut self) -> usize {
        for idx in 0..self.handler_overflow_count {
            if self.handler_overflow[idx].is_none() {
                return idx;
            }
        }
        assert!(
            self.handler_overflow_count < MAX_HANDLER_OVERFLOW,
            "handler overflow pool exhausted"
        );
        let idx = self.handler_overflow_count;
        self.handler_overflow_count += 1;
        idx
    }

    /// Atomically replace one device handler range with another.
    ///
    /// The complete final state is preflighted before the old range is removed,
    /// so overlap or capacity failure leaves every mapping unchanged. `None`
    /// supports initial registration and removal.
    pub(crate) fn relocate_memory_handlers(
        &mut self,
        device_id: super::MemoryDeviceId,
        old_range: Option<(BxPhyAddress, BxPhyAddress)>,
        new_range: Option<(BxPhyAddress, BxPhyAddress)>,
    ) -> Result<()> {
        use crate::memory::error::MemoryError;

        for (begin_addr, end_addr) in old_range.into_iter().chain(new_range) {
            if end_addr < begin_addr {
                return Err(MemoryError::InvalidAddressRange.into());
            }
        }

        let new_pages = new_range.map(|(begin_addr, end_addr)| {
            ((begin_addr >> 20) as usize, (end_addr >> 20) as usize)
        });
        if let Some((_, end_page)) = new_pages {
            if end_page >= self.memory_handlers.len() {
                return Err(
                    MemoryError::Internal("memory handler range exceeds handler table capacity")
                        .into(),
                );
            }
        }

        let mut projected_overflow = self.handler_overflow[..self.handler_overflow_count]
            .iter()
            .filter(|slot| slot.is_some())
            .count() as isize;
        let old_pages = old_range.map(|(begin_addr, end_addr)| {
            ((begin_addr >> 20) as usize, (end_addr >> 20) as usize)
        });

        if let Some((start_page, end_page)) = old_pages {
            if start_page < self.memory_handlers.len() {
                for page_idx in start_page..=end_page.min(self.memory_handlers.len() - 1) {
                    let page_new_range = new_range.filter(|_| {
                        new_pages.is_some_and(|(new_start, new_end)| {
                            (new_start..=new_end).contains(&page_idx)
                        })
                    });
                    projected_overflow += self.preflight_relocation_page(
                        page_idx,
                        device_id,
                        old_range,
                        page_new_range,
                    )?;
                }
            }
        }

        if let Some((start_page, end_page)) = new_pages {
            for page_idx in start_page..=end_page {
                if old_pages.is_some_and(|(old_start, old_end)| {
                    (old_start..=old_end).contains(&page_idx)
                }) {
                    continue;
                }
                projected_overflow +=
                    self.preflight_relocation_page(page_idx, device_id, old_range, new_range)?;
            }
        }

        if projected_overflow > MAX_HANDLER_OVERFLOW as isize {
            return Err(MemoryError::Internal("memory handler overflow pool exhausted").into());
        }

        if let Some((begin_addr, end_addr)) = old_range {
            self.unregister_memory_handlers(device_id, begin_addr, end_addr)
                .expect("preflighted handler relocation must unregister");
        }
        if let Some((begin_addr, end_addr)) = new_range {
            self.register_memory_handlers(device_id, begin_addr, end_addr)
                .expect("preflighted handler relocation must register");
        }
        Ok(())
    }

    /// Validate a relocation page and return its projected overflow-slot delta.
    fn preflight_relocation_page(
        &self,
        page_idx: usize,
        device_id: super::MemoryDeviceId,
        old_range: Option<(BxPhyAddress, BxPhyAddress)>,
        new_range: Option<(BxPhyAddress, BxPhyAddress)>,
    ) -> Result<isize> {
        use crate::memory::error::MemoryError;

        let new_bitmap = new_range
            .map(|(begin_addr, end_addr)| Self::handler_page_bitmap(page_idx, begin_addr, end_addr));
        let mut current_count = 0usize;
        let mut survivor_count = 0usize;
        let mut current = self.memory_handlers[page_idx].as_ref();
        while let Some(handler) = current {
            current_count += 1;
            let is_old = old_range.is_some_and(|(begin_addr, end_addr)| {
                handler.begin == begin_addr
                    && handler.end == end_addr
                    && handler.device_id.same_device(&device_id)
            });
            if !is_old {
                survivor_count += 1;
                if let Some(bitmap) = new_bitmap {
                    if bitmap
                        & Self::handler_page_bitmap(page_idx, handler.begin, handler.end)
                        != 0
                    {
                        return Err(MemoryError::OverlappingHandlers.into());
                    }
                }
            }
            current = handler
                .next
                .and_then(|idx| self.handler_overflow[idx as usize].as_ref());
        }

        let final_count = survivor_count + usize::from(new_range.is_some());
        if final_count > MAX_HANDLER_OVERFLOW + 1 {
            return Err(MemoryError::Internal("memory handler page capacity exhausted").into());
        }
        Ok(final_count.saturating_sub(1) as isize
            - current_count.saturating_sub(1) as isize)
    }

    /// Remove the memory handler covering exactly `[begin_addr, end_addr]` for
    /// `device_id`, restoring any other handlers that shared its pages. The
    /// inverse of [`register_memory_handlers`]; required for PCI BAR relocation
    /// (e.g. moving the VGA LFB to a BIOS-assigned base). Pages with no matching
    /// handler are left untouched.
    pub fn unregister_memory_handlers(
        &mut self,
        device_id: super::MemoryDeviceId,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> Result<()> {
        use crate::memory::error::MemoryError;

        if end_addr < begin_addr {
            return Err(MemoryError::InvalidAddressRange.into());
        }

        let start_page = (begin_addr >> 20) as usize;
        let end_page = (end_addr >> 20) as usize;
        // A page holds at most one handler per non-overlapping 64 KB sub-range.
        const MAX_PAGE_HANDLERS: usize = MAX_HANDLER_OVERFLOW + 1;

        for page_idx in start_page..=end_page {
            if page_idx >= self.memory_handlers.len() {
                break;
            }

            // Detach the whole chain for this page, freeing its overflow slots.
            let mut survivors: [Option<(super::MemoryDeviceId, BxPhyAddress, BxPhyAddress)>;
                MAX_PAGE_HANDLERS] = [None; MAX_PAGE_HANDLERS];
            let mut nsurv = 0usize;
            let mut cur = self.memory_handlers[page_idx].take();
            while let Some(handler) = cur {
                let next = handler
                    .next
                    .and_then(|idx| self.handler_overflow[idx as usize].take());
                let is_target = handler.begin == begin_addr
                    && handler.end == end_addr
                    && handler.device_id.same_device(&device_id);
                if !is_target {
                    survivors[nsurv] = Some((handler.device_id, handler.begin, handler.end));
                    nsurv += 1;
                }
                cur = next;
            }

            // Rebuild from the survivors, earliest-registered first, so the chain
            // order and the union bitmap are reconstructed exactly.
            for i in (0..nsurv).rev() {
                let (did, begin, end) = survivors[i].expect("survivor slot populated");
                self.register_page(page_idx, did, begin, end)?;
            }
        }

        Ok(())
    }

    // ========================================================================
    // Flash ROM state machine (Bochs misc_mem.cc)
    // ========================================================================

    /// Flash ROM read — returns value based on current flash state machine state.
    ///
    /// `addr` is a ROM array offset (already mapped via `bios_map_last128k` or
    /// `& BIOS_MASK` by the caller), matching Bochs misc_mem.cc.
    ///
    /// Not yet wired into the read path — stub for future integration when
    /// `flash_type > 0` is configured.
    pub(crate) fn flash_read(&mut self, addr: u32) -> u8 {
        match self.flash_wsm_state {
            FLASH_READ_ARRAY => {
                // Normal read — return ROM data (Bochs misc_mem.cc)
                let rom = self.inherited_memory_stub.rom();
                rom.get(addr as usize).copied().unwrap_or(0xFF)
            }
            FLASH_INT_ID => {
                // Manufacturer/device ID (Bochs misc_mem.cc)
                if (addr & 1) != 0 {
                    if self.flash_type == 2 {
                        0x7c
                    } else {
                        0x94
                    }
                } else {
                    0x89 // Intel manufacturer ID
                }
            }
            _ => {
                // FLASH_READ_STATUS and all other states return flash_status
                // (Bochs misc_mem.cc)
                if self.flash_wsm_state == FLASH_ERASE {
                    self.flash_status |= 0x80;
                }
                self.flash_status
            }
        }
    }

    /// Flash ROM write — processes command bytes for the flash state machine.
    ///
    /// `addr` is a ROM array offset (already mapped by the caller), matching
    /// Bochs misc_mem.cc.
    ///
    /// Not yet wired into the write path — stub for future integration when
    /// `flash_type > 0` is configured.
    pub(crate) fn flash_write(&mut self, addr: u32, data: u8) {
        let flash_addr = if self.flash_type == 2 {
            addr & 0x3ffff
        } else {
            addr & 0x1ffff
        };

        if self.flash_wsm_state == FLASH_PROG_SETUP {
            // Actual byte program — AND data into ROM (Bochs misc_mem.cc)
            let rom = self.inherited_memory_stub.rom();
            if let Some(byte) = rom.get_mut(addr as usize) {
                *byte &= data;
            }
            self.flash_wsm_state = FLASH_READ_STATUS;
            self.flash_modified = true;
        } else {
            // Command byte processing (Bochs misc_mem.cc)
            match data {
                FLASH_INT_ID | FLASH_READ_ARRAY | FLASH_ERASE_SETUP | FLASH_ERASE_SUSP
                | FLASH_PROG_SETUP => {
                    self.flash_wsm_state = data;
                }
                FLASH_READ_STATUS => {
                    if self.flash_wsm_state != FLASH_ERASE {
                        self.flash_wsm_state = data;
                    }
                }
                FLASH_CLR_STATUS => {
                    // Clear status register error bits (Bochs misc_mem.cc)
                    self.flash_status &= !0x38;
                    self.flash_wsm_state = FLASH_READ_ARRAY;
                }
                FLASH_ERASE => {
                    // Erase confirm / erase resume (Bochs misc_mem.cc)
                    if self.flash_wsm_state == FLASH_ERASE_SETUP {
                        self.flash_status &= !0xc0;
                        self.flash_wsm_state = FLASH_ERASE;
                        // Block erase — fill block with 0xFF
                        let rom = self.inherited_memory_stub.rom();
                        if self.flash_type == 1 && (flash_addr == 0x1c000 || flash_addr == 0x1d000)
                        {
                            for i in 0..0x1000u32 {
                                if let Some(byte) = rom.get_mut((addr + i) as usize) {
                                    *byte = 0xff;
                                }
                            }
                            self.flash_modified = true;
                        } else if self.flash_type == 2
                            && (flash_addr == 0x38000 || flash_addr == 0x3a000)
                        {
                            for i in 0..0x2000u32 {
                                if let Some(byte) = rom.get_mut((addr + i) as usize) {
                                    *byte = 0xff;
                                }
                            }
                            self.flash_modified = true;
                        }
                    } else if self.flash_wsm_state == FLASH_ERASE_SUSP {
                        // Erase resume (Bochs misc_mem.cc)
                        self.flash_status &= !0x40;
                        self.flash_wsm_state = FLASH_ERASE;
                    } else {
                        tracing::trace!("flash_write(): unexpected ERASE CONFIRM / ERASE RESUME");
                    }
                }
                _ => {
                    tracing::trace!("flash_write(): unsupported code {:#04x}", data);
                }
            }
        }
    }
}

#[cfg(test)]
mod handler_tests {

/// Emulator construction needs a bigger stack than the default 2 MiB test
/// thread: `Emulator` is ~4 MiB and the debug build materialises a few
/// copies while boxing it. 64 MiB is ample; the previous 256 MiB made
/// enough concurrent reservations to intermittently exhaust the process
/// and fail unrelated tests with STATUS_STACK_OVERFLOW.
const TEST_STACK_SIZE: usize = 64 * 1024 * 1024;
    use super::*;
    use crate::memory::MemoryDeviceId;

    fn test_mem() -> BxMemC<'static> {
        let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
        BxMemC::new(stub, false)
    }

    #[test]
    fn direct_write_accepts_translated_high_gpa() {
        // GPA 4 GiB maps down across the 1 GiB PCI hole to RAM offset 3 GiB.
        // This is the direct-host eligibility proof used by the CPU write path;
        // no multi-gigabyte host allocation is needed to test the translation.
        assert!(direct_host_write_allowed(0x1_0000_0000, 0xC000_0001, false));
    }

    // Fake device pointers — the handler table only stores/compares them here;
    // these tests never dispatch through them, so they are never dereferenced.
    fn vga_a() -> MemoryDeviceId {
        MemoryDeviceId::Vga(core::ptr::null_mut())
    }
    fn vga_b() -> MemoryDeviceId {
        MemoryDeviceId::Vga(4usize as *mut crate::iodev::vga::BxVgaC)
    }

    fn handler_range_at(mem: &BxMemC<'_>, addr: u64) -> Option<(u64, u64)> {
        let page = (addr >> 20) as usize;
        if page >= mem.memory_handlers.len() {
            return None;
        }
        let mut cur = mem.memory_handlers[page].as_ref();
        while let Some(h) = cur {
            if h.begin <= addr && h.end >= addr {
                return Some((h.begin, h.end));
            }
            cur = h.next.and_then(|i| mem.handler_overflow[i as usize].as_ref());
        }
        None
    }

    fn handler_device_snapshot(device_id: MemoryDeviceId) -> (u8, usize) {
        match device_id {
            MemoryDeviceId::Vga(pointer) => (0, pointer as usize),
            MemoryDeviceId::IoApic(pointer) => (1, pointer as usize),
            MemoryDeviceId::None => (2, 0),
            MemoryDeviceId::Hpet(pointer) => (3, pointer as usize),
        }
    }

    fn handler_page_snapshot(
        mem: &BxMemC<'_>,
        page: usize,
    ) -> (
        [Option<(Option<u16>, u64, u64, u16, (u8, usize))>; MAX_HANDLER_OVERFLOW + 1],
        usize,
    ) {
        let mut snapshot = [None; MAX_HANDLER_OVERFLOW + 1];
        let mut count = 0;
        let mut current = mem.memory_handlers[page].as_ref();
        while let Some(handler) = current {
            snapshot[count] = Some((
                handler.next,
                handler.begin,
                handler.end,
                handler.bitmap,
                handler_device_snapshot(handler.device_id),
            ));
            count += 1;
            current = handler
                .next
                .and_then(|idx| mem.handler_overflow[idx as usize].as_ref());
        }
        (snapshot, mem.handler_overflow_count)
    }

    #[test]
    fn handler_relocation_is_atomic_on_overlap() {
        let mut mem = test_mem();
        let old = (0xE000_0000u64, 0xE01F_FFFFu64);
        let new = (0xD000_0000u64, 0xD01F_FFFFu64);
        let conflicting = (0xD010_0000u64, 0xD010_FFFFu64);
        mem.register_memory_handlers(vga_a(), old.0, old.1).unwrap();
        mem.register_memory_handlers(vga_b(), conflicting.0, conflicting.1)
            .unwrap();

        let old_page = (old.0 >> 20) as usize;
        let first_new_page = (new.0 >> 20) as usize;
        let conflicting_page = (conflicting.0 >> 20) as usize;
        let before = (
            handler_page_snapshot(&mem, old_page),
            handler_page_snapshot(&mem, first_new_page),
            handler_page_snapshot(&mem, conflicting_page),
        );

        assert!(
            mem.relocate_memory_handlers(vga_a(), Some(old), Some(new))
                .is_err()
        );
        assert_eq!(
            (
                handler_page_snapshot(&mem, old_page),
                handler_page_snapshot(&mem, first_new_page),
                handler_page_snapshot(&mem, conflicting_page),
            ),
            before
        );
    }

    #[test]
    fn relocate_memory_handlers_moves_and_removes_handler() {
        let mut mem = test_mem();
        let initial = (0xE000_0000u64, 0xE00F_FFFFu64);
        let moved = (0xD000_0000u64, 0xD00F_FFFFu64);

        mem.relocate_memory_handlers(vga_a(), None, Some(initial))
            .unwrap();
        assert_eq!(handler_range_at(&mem, initial.0), Some(initial));
        mem.relocate_memory_handlers(vga_a(), Some(initial), Some(moved))
            .unwrap();
        assert_eq!(handler_range_at(&mem, initial.0), None);
        assert_eq!(handler_range_at(&mem, moved.0), Some(moved));
        mem.relocate_memory_handlers(vga_a(), Some(moved), None)
            .unwrap();
        assert_eq!(handler_range_at(&mem, moved.0), None);
    }

    #[test]
    fn relocate_memory_handlers_is_atomic_on_overflow_capacity() {
        let mut mem = test_mem();
        let old = (0xC000_0000u64, 0xC000_FFFFu64);
        let new = (0xD001_0000u64, 0xD001_FFFFu64);
        let blocker = (0xD000_0000u64, 0xD000_FFFFu64);
        mem.register_memory_handlers(vga_a(), old.0, old.1).unwrap();

        for sub_page in 0..16u64 {
            let begin = 0xA000_0000 + (sub_page << 16);
            mem.register_memory_handlers(vga_b(), begin, begin + 0xFFFF)
                .unwrap();
        }
        mem.register_memory_handlers(vga_b(), 0xB000_0000, 0xB000_FFFF)
            .unwrap();
        mem.register_memory_handlers(vga_b(), 0xB001_0000, 0xB001_FFFF)
            .unwrap();
        mem.register_memory_handlers(vga_b(), blocker.0, blocker.1)
            .unwrap();
        assert_eq!(mem.handler_overflow_count, MAX_HANDLER_OVERFLOW);

        let before = (
            handler_page_snapshot(&mem, (old.0 >> 20) as usize),
            handler_page_snapshot(&mem, (new.0 >> 20) as usize),
        );
        assert!(
            mem.relocate_memory_handlers(vga_a(), Some(old), Some(new))
                .is_err()
        );
        assert_eq!(
            (
                handler_page_snapshot(&mem, (old.0 >> 20) as usize),
                handler_page_snapshot(&mem, (new.0 >> 20) as usize),
            ),
            before
        );
    }

    #[test]
    fn unregister_removes_sole_handler_and_frees_the_range() {
        let mut mem = test_mem();
        let begin = 0xE000_0000u64;
        let end = begin + (16 << 20) - 1; // 16 MB LFB, 16 pages

        mem.register_memory_handlers(vga_a(), begin, end).unwrap();
        assert_eq!(handler_range_at(&mem, begin + 0x1234), Some((begin, end)));

        mem.unregister_memory_handlers(vga_a(), begin, end).unwrap();
        for p in (begin >> 20)..=(end >> 20) {
            assert!(
                mem.memory_handlers[p as usize].is_none(),
                "page {p:#x} not cleared"
            );
        }
        // Bitmap was cleared, so the same range can be registered again.
        mem.register_memory_handlers(vga_a(), begin, end).unwrap();
        assert_eq!(handler_range_at(&mem, begin), Some((begin, end)));
    }

    #[test]
    fn unregister_preserves_other_handler_on_shared_page() {
        let mut mem = test_mem();
        let a = (0xA0000u64, 0xAFFFFu64); // page 0, 64 KB sub-range 0xA
        let b = (0xB0000u64, 0xBFFFFu64); // page 0, 64 KB sub-range 0xB

        mem.register_memory_handlers(vga_a(), a.0, a.1).unwrap();
        mem.register_memory_handlers(vga_b(), b.0, b.1).unwrap();

        mem.unregister_memory_handlers(vga_a(), a.0, a.1).unwrap();

        assert_eq!(handler_range_at(&mem, 0xB_8000), Some(b), "B must survive");
        assert_eq!(handler_range_at(&mem, 0xA_8000), None, "A must be gone");

        // A's sub-range is free again.
        mem.register_memory_handlers(vga_a(), a.0, a.1).unwrap();
        assert_eq!(handler_range_at(&mem, 0xA_8000), Some(a));
        assert_eq!(handler_range_at(&mem, 0xB_8000), Some(b));
    }

    #[test]
    fn unregister_matches_device_identity() {
        let mut mem = test_mem();
        let r = (0xC0000u64, 0xCFFFFu64);
        mem.register_memory_handlers(vga_a(), r.0, r.1).unwrap();

        // Wrong device id must not remove the handler.
        mem.unregister_memory_handlers(vga_b(), r.0, r.1).unwrap();
        assert_eq!(handler_range_at(&mem, 0xC_8000), Some(r));

        mem.unregister_memory_handlers(vga_a(), r.0, r.1).unwrap();
        assert_eq!(handler_range_at(&mem, 0xC_8000), None);
    }

    #[test]
    fn repeated_register_unregister_does_not_leak_overflow_pool() {
        let mut mem = test_mem();
        let a = (0xA0000u64, 0xAFFFFu64);
        let b = (0xB0000u64, 0xBFFFFu64);
        mem.register_memory_handlers(vga_a(), a.0, a.1).unwrap();

        // Far more cycles than the 16-entry pool could hold if it leaked.
        for _ in 0..200 {
            mem.register_memory_handlers(vga_b(), b.0, b.1).unwrap();
            mem.unregister_memory_handlers(vga_b(), b.0, b.1).unwrap();
        }

        assert!(mem.handler_overflow_count <= MAX_HANDLER_OVERFLOW);
        assert_eq!(handler_range_at(&mem, 0xA_8000), Some(a));
    }

    // ─── Finding #8: enable_smram/disable_smram actually switch routing ──────

    #[test]
    fn direct_mapping_uses_by_value_monitor_policy() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);

        assert!(
            mem.get_host_mem_addr_pinned(
                0x2000,
                MemoryAccessType::RW,
                &[],
                CpuMemoryPolicy::new(false, true),
            )
            .unwrap()
            .is_none()
        );
        assert!(
            mem.get_host_mem_addr_pinned(
                0x2000,
                MemoryAccessType::RW,
                &[],
                CpuMemoryPolicy::new(false, false),
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn enable_smram_bypasses_vga_handler_disable_restores_it() {
        use crate::cpu::builder::BxCpuBuilder;
        use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;
        use crate::iodev::vga::BxVgaC;

        // BxICache contains ~19MB fixed arrays; the debug-mode struct literal
        // built by BxCpuBuilder::build() overflows the small default test
        // stack (2MB on win32), so this must run on a big-stack thread —
        // same pattern as cpu/tests_jumps.rs.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(move || {
                let mut mem = test_mem();

                // Register a VGA memory handler over the SMRAM window (0xA0000-0xBFFFF),
                // matching real hardware where VGA legacy memory owns that range when
                // SMRAM shadowing is closed.
                let mut vga = BxVgaC::new();
                let vga_id = MemoryDeviceId::Vga(&mut vga as *mut BxVgaC);
                mem.register_memory_handlers(vga_id, 0xA0000, 0xBFFFF).unwrap();

                let cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
                let pins = [CpuTlbPin::new(&*cpu)];

                // SMRAM open (DOPEN, unrestricted): the write must land in RAM,
                // bypassing the VGA handler entirely — write_physical_page checks
                // smram_available/smram_enable BEFORE the memory-handler table.
                mem.enable_smram(true, false);
                let mut data = [0x42u8];
                mem.write_physical_page(
                    &pins,
                    CpuMemoryPolicy::default(),
                    0xA1000,
                    1,
                    &mut data,
                )
                .unwrap();
                let mut ram_byte = [0];
                assert_eq!(mem.read_ram(&pins, 0xA1000, &mut ram_byte).unwrap(), 1);
                assert_eq!(ram_byte, [0x42], "SMRAM open must route the write to RAM");

                // disable_smram() must restore prior (VGA-handler) routing: the same
                // address now goes to the VGA handler, not RAM, so the RAM byte
                // written above stays untouched by the second write.
                mem.disable_smram();
                let mut data2 = [0x99u8];
                mem.write_physical_page(
                    &pins,
                    CpuMemoryPolicy::default(),
                    0xA1000,
                    1,
                    &mut data2,
                )
                .unwrap();
                assert_eq!(mem.read_ram(&pins, 0xA1000, &mut ram_byte).unwrap(), 1);
                assert_eq!(
                    ram_byte,
                    [0x42],
                    "SMRAM disabled must route the write to the VGA handler, not RAM"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    // ─── Finding #35b: bios_write_enabled gates BIOS-ROM-region writes ───────
    //
    // Bochs misc_mem.cc BX_MEM_C::init_memory() defaults bios_write_enabled
    // to false; PIIX3 XBCS bit 2 (pci2isa.cc case 0x4e) is the only thing
    // that ever flips it via DEV_mem_set_bios_write(). memory.cc gates the
    // top-of-address-space BIOS mirror write path on it directly:
    //   } else if (BX_MEM_THIS bios_write_enabled && is_bios) { ... }

    #[test]
    fn bios_write_enabled_gates_high_mirror_rom_writes() {
        use crate::cpu::builder::BxCpuBuilder;
        use crate::cpu::cpudb::amd::amd_ryzen::AmdRyzen;

        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(move || {
                let mut mem = test_mem();
                mem.set_a20_mask(0xFFFF_FFFF_FFFF_FFFF); // A20 enabled: no address wraparound

                let cpu = BxCpuBuilder::<AmdRyzen>::new().build().unwrap();
                let pins = [CpuTlbPin::new(&*cpu)];

                // High BIOS mirror: any address >= bios_rom_addr (default
                // 0xffff0000), far above the 1MB guest RAM this test_mem()
                // uses, so write_physical_page takes the `is_bios` branch.
                let addr: u64 = 0xFFFF_0000;
                let rom_offset = bios_map_last128k(addr as usize);

                // Default (matches Bochs init_memory: bios_write_enabled =
                // false): the write must be dropped, not land in ROM.
                assert!(!mem.bios_write_enabled());
                let mut data = [0xAAu8];
                mem.write_physical_page(
                    &pins,
                    CpuMemoryPolicy::default(),
                    addr,
                    1,
                    &mut data,
                )
                .unwrap();
                assert_ne!(
                    mem.inherited_memory_stub.rom()[rom_offset],
                    0xAA,
                    "write must be dropped while BIOS write is disabled (Bochs default)"
                );

                // XBCS bit 2 set (pci2isa.cc DEV_mem_set_bios_write(true)):
                // the same write must now land.
                mem.set_bios_write_enabled(true);
                let mut data2 = [0xAAu8];
                mem.write_physical_page(
                    &pins,
                    CpuMemoryPolicy::default(),
                    addr,
                    1,
                    &mut data2,
                )
                .unwrap();
                assert_eq!(
                    mem.inherited_memory_stub.rom()[rom_offset],
                    0xAA,
                    "write must succeed once BIOS write is enabled"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
