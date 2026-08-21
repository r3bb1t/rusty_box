#![allow(private_interfaces, dead_code)]
#![allow(non_snake_case)]

use super::PhysAccess;
use crate::{
    config::BxPhyAddress,
    cpu::rusty_box::MemoryAccessType,
    memory::{
        memory_rusty_box::{
            bios_map_last128k, bx_guest_ram_span, MemoryAreaT, BIOSROMSZ, BIOS_MASK, EXROM_MASK,
        },
        BxMemC, BxMemoryStubC, CpuMemoryPolicy,
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

impl BxMemC {
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
            mmio: super::mmio_map::MmioMap::new(),

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
        }
    }
}

impl BxMemC {
    /// Return a resident, block-bounded allocation range for an already
    /// A20-adjusted GPA after checked PCI-hole/high-RAM translation.
    fn resident_ram_range(
        &mut self,
        addr: BxPhyAddress,
    ) -> Result<core::ops::Range<usize>> {
        let span = bx_guest_ram_span(addr, 1, self.inherited_memory_stub.guest_len())
            .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
        self.inherited_memory_stub
            .resident_slot_range(span.start)
    }

    /// The sole CPU-facing direct host mapping, as bytes. The caller supplies
    /// a by-value CPU memory policy; a caller that caches what it gets back
    /// must also record the residency epoch it was answered at, since nothing
    /// here holds a guest block in place.
    pub(crate) fn get_host_mem_addr(
        &mut self,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        policy: CpuMemoryPolicy,
    ) -> Result<Option<&mut [u8]>> {
        let Some(range) = self.host_mem_range(addr, rw, policy)? else {
            return Ok(None);
        };
        Ok(Some(&mut self.inherited_memory_stub.actual_vector_mut()[range]))
    }

    /// The same mapping decision, reported as a range *within the memory
    /// allocation* instead of as borrowed bytes.
    ///
    /// This is the primitive: every arm below already knows the offset before
    /// it would build a slice, and an offset is what a caller can cache. It
    /// also lets a caller ask where a page lives without holding a borrow of
    /// the whole memory system for as long as it keeps the answer.
    ///
    /// Guest RAM, the ROM image and the bogus page all live in that one
    /// allocation, so a single range type spans every arm — which is why the
    /// instruction side can be offset-based at all.
    pub(crate) fn host_mem_range(
        &mut self,
        addr: BxPhyAddress,
        rw: MemoryAccessType,
        policy: CpuMemoryPolicy,
    ) -> Result<Option<core::ops::Range<usize>>> {
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
            return Ok(Some(self.resident_ram_range(a20_addr)?));
        }

        if write && policy.monitor_hit() {
            return Ok(None);
        }

        // Registered MMIO regions always win over direct RAM.
        if self.mmio.covers(a20_addr) {
            return Ok(None);
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
                    return Ok(Some(self.resident_ram_range(a20_addr)?));
                }
                let rom_offset = if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    bios_map_last128k(a20_addr as usize)
                } else {
                    ((a20_addr & EXROM_MASK as BxPhyAddress) + BIOSROMSZ as BxPhyAddress)
                        as usize
                };
                return Ok(Some(self.inherited_memory_stub.rom_range(rom_offset)));
            }
            if bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.guest_len()).is_some() && !is_bios
            {
                if !(0x000c0000..0x00100000).contains(&a20_addr) {
                    return Ok(Some(self.resident_ram_range(a20_addr)?));
                }
                if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    let mapped = bios_map_last128k(a20_addr as usize);
                    return Ok(Some(self.inherited_memory_stub.rom_range(mapped)));
                }
                let rom_offset =
                    ((a20_addr & EXROM_MASK as BxPhyAddress) + BIOSROMSZ as BxPhyAddress)
                        as usize;
                return Ok(Some(self.inherited_memory_stub.rom_range(rom_offset)));
            }
            if a20_addr > u64::from(u32::MAX) {
                return Ok(Some(
                    self.inherited_memory_stub.bogus_range((a20_addr & 0xfff) as usize),
                ));
            }
            if (0xFEE00000..0xFEF00000).contains(&a20_addr) {
                return Ok(None);
            }
            if is_bios {
                let rom_offset = bios_map_last128k(a20_addr as usize);
                return Ok(Some(self.inherited_memory_stub.rom_range(rom_offset)));
            }
            return Ok(Some(
                self.inherited_memory_stub.bogus_range((a20_addr & 0xfff) as usize),
            ));
        }

        if !direct_host_write_allowed(a20_addr, self.inherited_memory_stub.guest_len(), is_bios) {
            return Ok(None);
        }
        Ok(Some(self.resident_ram_range(a20_addr)?))
    }
}

impl BxMemC {
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
        ram_data: &[u8],
        ram_address: BxPhyAddress,
    ) -> Result<()> {
        if ram_data.is_empty() {
            return Err(MemoryError::RamImageOutOfRange.into());
        }
        let copied = self.write_ram(ram_address, ram_data)?;
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
        policy: CpuMemoryPolicy,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<PhysAccess> {
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
        // Bochs memory.cc wraps the SMRAM window in `if (cpu != NULL)`, so a
        // device access (DMA, a device writing memory) never reaches SMRAM
        // through it and falls through to the handler/VGA routing below.
        let smram_hit = policy.is_cpu_context()
            && (0x000a0000..0x000c0000).contains(&a20_addr)
            && self.smram_available
            && (self.smram_enable || (policy.smm_mode() && !self.smram_restricted));
        if smram_hit {
            // Write to SMRAM - delegate to stub for regular memory write
            self.inherited_memory_stub.write_physical_page(addr,
                len,
                data,
                self.a20_mask,
            )?;
            return Ok(PhysAccess::Done);
        }

        // Bochs calls the device's write_handler here. This reports the owner
        // instead; the caller runs it, because only the caller holds devices.
        if let Some(token) = self.mmio.lookup(a20_addr) {
            return Ok(PhysAccess::Mmio(token));
        }

        // mem_write: (from memory.cc)

        // All memory access fits in single 4K page.
        // Note: Bochs does NOT check is_bios here — addresses in E0000-FFFFF
        // (where is_bios=true) must enter this block to reach the PCI shadow RAM
        // write path. High BIOS addresses (>= bios_rom_addr like 0xFFFF0000) are
        // above RAM len so the `a20_addr < len` check naturally excludes them.
        if bx_guest_ram_span(a20_addr, len, self.inherited_memory_stub.guest_len()).is_some() {
            // All of data is within limits of physical memory
            if !(0x000a0000..0x00100000).contains(&a20_addr) {
                // Log writes to very low RAM (first 4KB) - these might be IVT/BDA initialization
                // Regular RAM - delegate to stub
                self.inherited_memory_stub.write_physical_page(addr,
                    len,
                    data,
                    self.a20_mask,
                )?;
                return Ok(PhysAccess::Done);
            }

            // Address must be in range 0x000A0000..0x000FFFFF
            self.inherited_memory_stub.smc_dec_write_stamp_page(a20_addr);

            for &data_byte in &data[..len] {
                // SMMRAM (0xA0000-0xBFFFF)
                if a20_addr < 0x000c0000 {
                    // Devices are not allowed to access SMMRAM under VGA memory.
                    let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.guest_len())
                        .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                    let vector = self.inherited_memory_stub.get_vector_offset(span.start)?;
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
                        let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.guest_len())
                            .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                        let vector = self.inherited_memory_stub.get_vector_offset(span.start)?;
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

            Ok(PhysAccess::Done)
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
            Ok(PhysAccess::Done)
        } else {
            // Access outside limits of physical memory, ignore (from memory.cc)
            Ok(PhysAccess::Done)
        }
    }

    /// Read physical page with memory handler support
    /// Based on BX_MEM_C::readPhysicalPage in memory.cc
    pub(crate) fn read_physical_page(
        &mut self,
        policy: CpuMemoryPolicy,
        addr: BxPhyAddress,
        len: usize,
        data: &mut [u8],
    ) -> Result<PhysAccess> {
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

        // Check SMRAM first (before memory handlers). Gated on CPU context —
        // Bochs memory.cc reaches the SMRAM window only when `cpu != NULL`.
        if policy.is_cpu_context()
            && (0x000a0000..0x000c0000).contains(&a20_addr)
            && self.smram_available
            && (self.smram_enable || (policy.smm_mode() && !self.smram_restricted))
        {
            // Read from SMRAM - delegate to stub for regular memory read
            self.inherited_memory_stub.read_physical_page(addr,
                len,
                data,
                self.a20_mask,
            )?;
            return Ok(PhysAccess::Done);
        }

        // See `write_physical_page`: the owner is reported, not invoked.
        if let Some(token) = self.mmio.lookup(a20_addr) {
            return Ok(PhysAccess::Mmio(token));
        }

        // mem_read:
        // Note: Bochs does NOT check is_bios here — addresses in E0000-FFFFF
        // must enter this block to reach the PCI shadow RAM read path.
        if bx_guest_ram_span(a20_addr, len, self.inherited_memory_stub.guest_len()).is_some() {
            // All of data is within limits of physical memory
            if !(0x000a0000..0x00100000).contains(&a20_addr) {
                // Regular RAM - delegate to stub
                self.inherited_memory_stub.read_physical_page(addr,
                    len,
                    data,
                    self.a20_mask,
                )?;
                return Ok(PhysAccess::Done);
            }

            // Address must be in range 0x000A0000..0x000FFFFF
            for data_byte in &mut data[..len] {
                // SMMRAM (0xA0000-0xBFFFF)
                if a20_addr < 0x000c0000 {
                    // Devices are not allowed to access SMMRAM under VGA memory.
                    let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.guest_len())
                        .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                    let vector = self.inherited_memory_stub.get_vector_offset(span.start)?;
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
                        let span = bx_guest_ram_span(a20_addr, 1, self.inherited_memory_stub.guest_len())
                            .ok_or(MemoryError::Internal("physical address is not guest RAM"))?;
                        let vector = self.inherited_memory_stub.get_vector_offset(span.start)?;
                        if let Some(byte) = vector.first() {
                            *data_byte = *byte;
                        }
                    }
                }

                a20_addr += 1;
            }

            Ok(PhysAccess::Done)
        } else {
            // Access outside limits of physical memory

            if a20_addr > 0xffffffffu64 {
                data.fill(0xFF);
                return Ok(PhysAccess::Done);
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

            Ok(PhysAccess::Done)
        }
    }

    /// Map an address range to the device that owns it.
    ///
    /// Bochs misc_mem.cc `registerMemoryHandlers`, minus the handler pointers:
    /// the range is recorded against `token`, and an access that lands in it is
    /// reported to the caller rather than dispatched here.
    pub fn register_memory_handlers(
        &mut self,
        token: super::mmio_map::MmioToken,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> Result<()> {
        self.mmio.map(token, begin_addr, end_addr)?;
        Ok(())
    }

    /// Move `token`'s mapping from `old_range` to `new_range`, atomically.
    ///
    /// Either side may be absent, covering first registration and removal. A
    /// rejected move leaves every mapping unchanged, which is what lets a PCI
    /// BAR write be reported to the device only when it actually took effect.
    pub(crate) fn relocate_memory_handlers(
        &mut self,
        token: super::mmio_map::MmioToken,
        old_range: Option<(BxPhyAddress, BxPhyAddress)>,
        new_range: Option<(BxPhyAddress, BxPhyAddress)>,
    ) -> Result<()> {
        self.mmio.relocate(token, old_range, new_range)?;
        Ok(())
    }

    /// Remove `token`'s mapping of exactly `[begin_addr, end_addr]`.
    ///
    /// Bochs `unregisterMemoryHandlers` matches the owner *and* the exact
    /// range, so one device cannot drop another's mapping by naming its
    /// addresses; an unmatched range is not an error.
    pub fn unregister_memory_handlers(
        &mut self,
        token: super::mmio_map::MmioToken,
        begin_addr: BxPhyAddress,
        end_addr: BxPhyAddress,
    ) -> Result<()> {
        self.mmio.unmap(token, begin_addr, end_addr)?;
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

    fn test_mem() -> BxMemC {
        let stub = BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap();
        BxMemC::new(stub, false)
    }

    /// Instruction fetch decides whether it may install an ITLB entry by
    /// asking whether the returned span covers a whole page. The ROM and bogus
    /// arms answer with a span that runs to the end of the allocation, not to
    /// the end of their nominal region — so the page holding the reset vector,
    /// which sits at the very top of the ROM image, still reports a full page.
    ///
    /// Bounding those spans at their region instead would cost nothing
    /// visible: the guest boots either way, just without a cached mapping for
    /// the top of the ROM. This is the only thing that would notice.
    #[test]
    fn rom_and_bogus_spans_run_to_the_end_of_the_allocation() {
        let mut mem = test_mem();
        // A20 defaults to masked, which pulls the reset vector below
        // `bios_rom_addr` and routes it to the bogus page instead of the ROM.
        // The BIOS enables A20 long before anything fetches from the top of
        // the image, so this is the configuration that matters here.
        mem.set_a20_mask(u64::MAX);
        let (_, allocation_len) = mem.allocation_span();

        for (what, addr) in [("reset vector", 0xFFFF_FFF0u64), ("bogus page", 0x1_0000_0000)] {
            let range = mem
                .host_mem_range(
                    addr,
                    MemoryAccessType::Execute,
                    CpuMemoryPolicy::default(),
                )
                .unwrap()
                .unwrap_or_else(|| panic!("{what} must have a direct mapping"));
            assert!(
                range.end <= allocation_len,
                "{what} span must stay inside the allocation"
            );
            assert!(
                range.len() >= 4096,
                "{what} span is {} bytes, so fetch would refuse to cache the page",
                range.len()
            );
        }
    }

    #[test]
    fn direct_write_accepts_translated_high_gpa() {
        // GPA 4 GiB maps down across the 1 GiB PCI hole to RAM offset 3 GiB.
        // This is the direct-host eligibility proof used by the CPU write path;
        // no multi-gigabyte host allocation is needed to test the translation.
        assert!(direct_host_write_allowed(0x1_0000_0000, 0xC000_0001, false));
    }

    // Two owners. The map stores tokens, so these are values, not pointers —
    // the identity a mapping is matched on can no longer dangle.
    const OWNER_A: super::super::mmio_map::MmioToken = super::super::mmio_map::MmioToken(1);
    const OWNER_B: super::super::mmio_map::MmioToken = super::super::mmio_map::MmioToken(2);

    /// Whether a mapped region shadows direct RAM at `addr`.
    ///
    /// This is the property the routing actually depends on: a mapped range
    /// must make `get_host_mem_addr` decline, so the CPU falls off its
    /// direct path and into the reporting one.
    fn direct_ram_available(mem: &mut BxMemC, addr: u64) -> bool {
        matches!(
            mem.get_host_mem_addr(
                addr,
                MemoryAccessType::Read,
                CpuMemoryPolicy::default(),
            ),
            Ok(Some(_))
        )
    }

    /// A mapped region must hide direct RAM, and unmapping must give it back.
    /// Bochs decides this the same way — a registered handler wins over the
    /// direct path — and getting it wrong is invisible until a device's
    /// registers start reading as stale RAM.
    #[test]
    fn a_mapped_region_shadows_direct_ram_until_it_is_unmapped() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);
        assert!(direct_ram_available(&mut mem, 0x2000));

        mem.register_memory_handlers(OWNER_A, 0x2000, 0x2FFF).unwrap();
        assert!(
            !direct_ram_available(&mut mem, 0x2000),
            "a mapped region must not be served from RAM"
        );

        mem.unregister_memory_handlers(OWNER_A, 0x2000, 0x2FFF)
            .unwrap();
        assert!(
            direct_ram_available(&mut mem, 0x2000),
            "unmapping must restore the direct path"
        );
    }

    /// A physical access inside a mapped region reports its owner instead of
    /// touching RAM, and one outside it does not.
    #[test]
    fn a_physical_access_reports_the_owning_token() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);
        mem.register_memory_handlers(OWNER_B, 0x4000, 0x4FFF).unwrap();

        let mut data = [0u8; 4];
        assert_eq!(
            mem.read_physical_page(CpuMemoryPolicy::default(), 0x4010, 4, &mut data)
                .unwrap(),
            PhysAccess::Mmio(OWNER_B)
        );
        assert_eq!(
            mem.write_physical_page(CpuMemoryPolicy::default(), 0x4010, 4, &mut data)
                .unwrap(),
            PhysAccess::Mmio(OWNER_B)
        );
        assert_eq!(
            mem.read_physical_page(CpuMemoryPolicy::default(), 0x5010, 4, &mut data)
                .unwrap(),
            PhysAccess::Done,
            "an address outside every region is ordinary memory"
        );
    }

    /// A rejected relocation must leave the old mapping serving, because the
    /// device has already been told its BAR moved.
    #[test]
    fn a_rejected_relocation_leaves_the_old_region_mapped() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);
        let old = (0xA000_0000u64, 0xA000_FFFFu64);
        let blocker = (0xB000_0000u64, 0xB000_FFFFu64);
        mem.register_memory_handlers(OWNER_A, old.0, old.1).unwrap();
        mem.register_memory_handlers(OWNER_B, blocker.0, blocker.1)
            .unwrap();

        assert!(mem
            .relocate_memory_handlers(OWNER_A, Some(old), Some(blocker))
            .is_err());

        let mut data = [0u8; 4];
        assert_eq!(
            mem.read_physical_page(CpuMemoryPolicy::default(), old.0, 4, &mut data)
                .unwrap(),
            PhysAccess::Mmio(OWNER_A)
        );
        assert_eq!(
            mem.read_physical_page(CpuMemoryPolicy::default(), blocker.0, 4, &mut data)
                .unwrap(),
            PhysAccess::Mmio(OWNER_B)
        );
    }

    /// Relocation covers first registration and removal, which is how a PCI
    /// BAR that has never been programmed and one being torn down are handled.
    #[test]
    fn relocation_registers_moves_and_removes() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);
        let initial = (0xA000_0000u64, 0xA000_FFFFu64);
        let moved = (0xA010_0000u64, 0xA010_FFFFu64);
        let mut data = [0u8; 4];
        let read = |mem: &mut BxMemC, addr: u64, data: &mut [u8; 4]| {
            mem.read_physical_page(CpuMemoryPolicy::default(), addr, 4, data)
                .unwrap()
        };

        mem.relocate_memory_handlers(OWNER_A, None, Some(initial))
            .unwrap();
        assert_eq!(read(&mut mem, initial.0, &mut data), PhysAccess::Mmio(OWNER_A));

        mem.relocate_memory_handlers(OWNER_A, Some(initial), Some(moved))
            .unwrap();
        assert_eq!(read(&mut mem, initial.0, &mut data), PhysAccess::Done);
        assert_eq!(read(&mut mem, moved.0, &mut data), PhysAccess::Mmio(OWNER_A));

        mem.relocate_memory_handlers(OWNER_A, Some(moved), None)
            .unwrap();
        assert_eq!(read(&mut mem, moved.0, &mut data), PhysAccess::Done);
    }

    /// Register/unregister cycles must not leak region slots. The old chain
    /// leaked its fixed overflow pool without a free-list; a table that fills
    /// up silently would start refusing a relocating BAR.
    #[test]
    fn repeated_register_unregister_does_not_leak_region_slots() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);
        let kept = (0xA_0000u64, 0xA_FFFFu64);
        let churned = (0xB_0000u64, 0xB_FFFFu64);
        mem.register_memory_handlers(OWNER_A, kept.0, kept.1).unwrap();

        for _ in 0..200 {
            mem.register_memory_handlers(OWNER_B, churned.0, churned.1)
                .unwrap();
            mem.unregister_memory_handlers(OWNER_B, churned.0, churned.1)
                .unwrap();
        }

        assert_eq!(mem.mmio.len(), 1, "only the kept region may remain");
    }

    // ─── Finding #8: enable_smram/disable_smram actually switch routing ──────

    #[test]
    fn direct_mapping_uses_by_value_monitor_policy() {
        let mut mem = test_mem();
        mem.set_a20_mask(u64::MAX);

        assert!(
            mem.get_host_mem_addr(
                0x2000,
                MemoryAccessType::RW,
                CpuMemoryPolicy::new(false, true),
            )
            .unwrap()
            .is_none()
        );
        assert!(
            mem.get_host_mem_addr(
                0x2000,
                MemoryAccessType::RW,
                CpuMemoryPolicy::new(false, false),
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn enable_smram_bypasses_vga_handler_disable_restores_it() {
        // BxICache contains ~19MB fixed arrays; the debug-mode struct literal
        // built by BxCpuBuilder::build() overflows the small default test
        // stack (2MB on win32), so this must run on a big-stack thread —
        // same pattern as cpu/tests_jumps.rs.
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(move || {
                let mut mem = test_mem();

                // Map the VGA aperture over the SMRAM window (0xA0000-0xBFFFF),
                // matching real hardware where VGA legacy memory owns that range
                // when SMRAM shadowing is closed. Only the owner's identity
                // matters here, so the token stands in for the device.
                mem.register_memory_handlers(OWNER_A, 0xA0000, 0xBFFFF)
                    .unwrap();

                // SMRAM open (DOPEN, unrestricted): the write must land in RAM,
                // bypassing the mapped region entirely — write_physical_page
                // checks smram_available/smram_enable BEFORE the region map.
                mem.enable_smram(true, false);
                let mut data = [0x42u8];
                assert_eq!(
                    mem.write_physical_page(CpuMemoryPolicy::default(),
                        0xA1000,
                        1,
                        &mut data,
                    )
                    .unwrap(),
                    PhysAccess::Done,
                    "SMRAM open must complete in memory, reaching no device"
                );
                let mut ram_byte = [0];
                assert_eq!(mem.read_ram(0xA1000, &mut ram_byte).unwrap(), 1);
                assert_eq!(ram_byte, [0x42], "SMRAM open must route the write to RAM");

                // disable_smram() must restore the prior routing: the same
                // address is now the mapped region's, so the access is reported
                // to its owner and RAM is left as the first write set it.
                mem.disable_smram();
                let mut data2 = [0x99u8];
                assert_eq!(
                    mem.write_physical_page(CpuMemoryPolicy::default(),
                        0xA1000,
                        1,
                        &mut data2,
                    )
                    .unwrap(),
                    PhysAccess::Mmio(OWNER_A),
                    "SMRAM disabled must route the write to the mapped device"
                );
                assert_eq!(mem.read_ram(0xA1000, &mut ram_byte).unwrap(), 1);
                assert_eq!(
                    ram_byte,
                    [0x42],
                    "a reported MMIO write must not also touch RAM"
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
        std::thread::Builder::new()
            .stack_size(TEST_STACK_SIZE)
            .spawn(move || {
                let mut mem = test_mem();
                mem.set_a20_mask(0xFFFF_FFFF_FFFF_FFFF); // A20 enabled: no address wraparound

                // High BIOS mirror: any address >= bios_rom_addr (default
                // 0xffff0000), far above the 1MB guest RAM this test_mem()
                // uses, so write_physical_page takes the `is_bios` branch.
                let addr: u64 = 0xFFFF_0000;
                let rom_offset = bios_map_last128k(addr as usize);

                // Default (matches Bochs init_memory: bios_write_enabled =
                // false): the write must be dropped, not land in ROM.
                assert!(!mem.bios_write_enabled());
                let mut data = [0xAAu8];
                assert_eq!(
                    mem.write_physical_page(CpuMemoryPolicy::default(),
                        addr,
                        1,
                        &mut data,
                    )
                    .unwrap(),
                    PhysAccess::Done,
                    "the BIOS mirror is memory's own, not a device's"
                );
                assert_ne!(
                    mem.inherited_memory_stub.rom()[rom_offset],
                    0xAA,
                    "write must be dropped while BIOS write is disabled (Bochs default)"
                );

                // XBCS bit 2 set (pci2isa.cc DEV_mem_set_bios_write(true)):
                // the same write must now land.
                mem.set_bios_write_enabled(true);
                let mut data2 = [0xAAu8];
                assert_eq!(
                    mem.write_physical_page(CpuMemoryPolicy::default(),
                        addr,
                        1,
                        &mut data2,
                    )
                    .unwrap(),
                    PhysAccess::Done,
                    "the BIOS mirror is memory's own, not a device's"
                );
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
