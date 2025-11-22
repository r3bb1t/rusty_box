use alloc::vec::Vec;

use crate::{
    config::BxPhyAddress,
    cpu::{rusty_box::MemoryAccessType, BxCpuC, BxCpuIdTrait},
    memory::{
        memory_rusty_box::{bios_map_last128k, MemoryAreaT, BIOSROMSZ, BIOS_MASK, EXROM_MASK},
        BxMemC, BxMemoryStubC,
    },
};

pub(super) const FLASH_READ_ARRAY: u8 = 0xff;
pub(super) const FLASH_INT_ID: u8 = 0x90;
pub(super) const FLASH_READ_STATUS: u8 = 0x70;
pub(super) const FLASH_CLR_STATUS: u8 = 0x50;
pub(super) const FLASH_ERASE_SETUP: u8 = 0x20;
pub(super) const FLASH_ERASE_SUSP: u8 = 0xb0;
pub(super) const FLASH_PROG_SETUP: u8 = 0x40;
pub(super) const FLASH_ERASE: u8 = 0xd0;

use super::Result;

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
            rom_present: [false; _],
            memory_type,

            bios_rom_access: 0, // idk tbh
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
        let a20_addr: BxPhyAddress = crate::pc_system::a20_addr(addr);

        let mut is_bios = a20_addr > self.bios_rom_addr.into();

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

        // Access the memory handler at the specified index
        let mut memory_handler = self.memory_handlers[(a20_addr >> 20) as usize]
            .as_ref()
            .unwrap() // FIXME: don't unwrap
            .next
            .as_ref();

        while let Some(handler) = memory_handler {
            // Check if the address is within the range
            if handler.begin <= a20_addr && handler.end >= a20_addr {
                // Call the direct access handler if it exists
                if let Some(da_handler) = handler.da_handler {
                    return Ok(Some(da_handler(
                        &handler.param,
                        a20_addr,
                        rw,
                        &handler.param,
                    )));
                } else {
                    return Ok(None); // Vetoed! No handler available
                }
            }
            // Move to the next handler
            memory_handler = handler.next.as_ref();
        }
        //let mut memory_handler = self.memory_handlers[a20_addr as usize >> 20];
        //
        //loop {
        //    if memory_handler.begin <= a20_addr && memory_handler.end >= a20_addr {
        //        if let Some(da_handler) = memory_handler.da_handler {
        //            let to_return = da_handler(&mut (), a20_addr, rw, &memory_handler.param);
        //            return Ok(Some(to_return));
        //        } else {
        //            return Ok(None); // Vetoed! memory handler for i/o apic, vram, mmio and PCI PnP
        //        }
        //    }
        //    if let Some(new_memory_handler) = *memory_handler.next {
        //        memory_handler = new_memory_handler;
        //    } else {
        //        break;
        //    }
        //}

        if !write {
            if a20_addr >= 0x000a0000 && a20_addr < 0x000c0000 {
                Ok(None)
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
                    if (a20_addr & 0xfffe0000) == 0x000e0000 {
                        // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                        let to_return = &mut self.inherited_memory_stub.rom()
                            [bios_map_last128k(a20_addr.try_into()?)..];
                        Ok(Some(to_return))
                    } else {
                        let to_return = &mut self.inherited_memory_stub.rom()[((a20_addr
                            & EXROM_MASK as BxPhyAddress)
                            + BIOSROMSZ as BxPhyAddress)
                            .try_into()?..];
                        Ok(Some(to_return))
                    }
                } else {
                    // Read from ShadowRAM
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                }
            } else if (a20_addr < self.inherited_memory_stub.len.try_into()?) && !is_bios {
                if a20_addr < 0x000c0000 || a20_addr >= 0x00100000 {
                    Ok(Some(self.get_vector(cpus, a20_addr)?))
                }
                // must be in C0000 - FFFFF range
                else if (a20_addr & 0xfffe0000) == 0x000e0000 {
                    // last 128K of BIOS ROM mapped to 0xE0000-0xFFFFF
                    Ok(Some(
                        &mut self.inherited_memory_stub.rom()
                            [bios_map_last128k(a20_addr.try_into()?)..],
                    ))
                } else {
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
                Ok(Some(
                    &mut self.inherited_memory_stub.rom()
                        [(a20_addr & BIOS_MASK as BxPhyAddress).try_into()?..],
                ))
            } else {
                // Error, requested addr is out of bounds.
                Ok(Some(
                    &mut self.inherited_memory_stub.bogus()[(a20_addr & 0xfff).try_into()?..],
                ))
            }
        } else {
            // op == {BX_WRITE, BX_RW}
            if (a20_addr >= self.inherited_memory_stub.len.try_into()?) || is_bios {
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
