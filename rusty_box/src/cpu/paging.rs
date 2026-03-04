//! Paging support
//!
//! Based on Bochs cpu/paging.cc
//! Implements page table walking and address translation

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, Result};
use crate::{
    config::{BxAddress, BxPhyAddress},
    cpu::{
        rusty_box::MemoryAccessType,
        tlb::{BxHostpageaddr, TLBEntry},
    },
    memory::BxMemC,
};

// Page fault error code bits
const ERROR_NOT_PRESENT: u32 = 0x00;
const ERROR_PROTECTION: u32 = 0x01;
const ERROR_WRITE_ACCESS: u32 = 0x02;
const ERROR_USER_ACCESS: u32 = 0x04;
const ERROR_RESERVED: u32 = 0x08;
const ERROR_CODE_ACCESS: u32 = 0x10;

// Combined access bits
const BX_COMBINED_ACCESS_WRITE: u32 = 0x2;
const BX_COMBINED_ACCESS_USER: u32 = 0x4;

// DTLB access permission bits (matching Bochs tlb.h)
const TLB_SYS_READ_OK: u32 = 0x01;
const TLB_USER_READ_OK: u32 = 0x02;
const TLB_SYS_WRITE_OK: u32 = 0x04;
const TLB_USER_WRITE_OK: u32 = 0x08;

// Paging level constants (matching Bochs paging.cc:542-548)
const BX_LEVEL_PTE: usize = 0;
const BX_LEVEL_PDE: usize = 1;
const BX_LEVEL_PDPTE: usize = 2;
const BX_LEVEL_PML4: usize = 3;
const BX_LEVEL_PML5: usize = 4;

// CR3 paging mask — legacy uses bits 31:12, PAE/long mode uses bits 51:12
const BX_CR3_PAGING_MASK: u64 = 0xFFFFF000;
const BX_CR3_PAGING_MASK_PAE: u64 = 0x000F_FFFF_FFFF_F000;

// NX bit in 64-bit page table entries
const PAGE_DIRECTORY_NX_BIT: u64 = 0x8000_0000_0000_0000;

// Reserved bits in 4MB PSE PDE entries (Bochs paging.cc PAGING_PDE4M_RESERVED_BITS).
// For BX_PHY_ADDRESS_WIDTH=40: ((1 << (41-40))-1) << (13+40-32) = 1 << 21 = 0x200000
const PAGING_PDE4M_RESERVED_BITS: u32 = 0x200000;

// Reserved bits for PAE paging (BX_PHY_ADDRESS_WIDTH=40)
// BX_PAGING_PHY_ADDRESS_RESERVED_BITS = bits 51:40 = 0x000F_FF00_0000_0000
const PAGING_PAE_PHY_RESERVED_BITS: u64 = 0x000F_FF00_0000_0000;

// In legacy PAE mode, bits [62:52] are reserved (bit 63 is NX)
// PAGING_LEGACY_PAE_RESERVED_BITS = PHY_RESERVED | 0x7FF0_0000_0000_0000
const PAGING_LEGACY_PAE_RESERVED_BITS: u64 = 0x7FFF_FF00_0000_0000;

// PAE PDE 2MB page reserved bits: bits 20:13 must be zero
// PAGING_PAE_PDE2M_RESERVED_BITS = PHY_RESERVED | 0x001F_E000
const PAGING_PAE_PDE2M_RESERVED_BITS: u64 = 0x000F_FF00_001F_E000;

// Legacy PAE PDPTE reserved bits: PHY_RESERVED | bits 63:52, 8:5, 2:1
// = 0x000F_FF00_0000_0000 | 0xFFF0_0000_0000_01E6 = 0xFFFF_FF00_0000_01E6
const PAGING_PAE_PDPTE_RESERVED_BITS: u64 = 0xFFFF_FF00_0000_01E6;

// Long mode PDPTE 1GB page reserved bits: bits 29:13 + PHY_RESERVED
const PAGING_PAE_PDPTE1G_RESERVED_BITS: u64 = 0x000F_FF00_3FFF_E000;

// Privilege check array (for CR0.WP=0 and CR0.WP=1)
// Index format: |wp|us|us|rw|rw| where:
//   wp: CR0.WP value
//   us: U/S of current access
//   us,rw: combined U/S and R/W from page tables
//   rw: R/W of current access
// Value: 1 = allowed, 0 = not allowed
const PRIV_CHECK: [u8; 32] = [
    // CR0.WP=0
    1, 1, 1, 1, 1, 1, 1, 1, // sys access
    0, 0, 0, 0, 1, 0, 1, 1, // user access
    // CR0.WP=1
    1, 0, 1, 1, 1, 0, 1, 1, // sys access
    0, 0, 0, 0, 1, 0, 1, 1, // user access
];

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    #[cfg(feature = "bx_large_ram_file")]
    pub(crate) fn check_addr_in_tlb_buffers(&self, addr: usize, end: usize) -> bool {
        let addr_ptr = addr;
        let end_ptr = end;

        // Check VMCS host pointer if VMX is active
        #[cfg(feature = "bx_support_vmx")]
        {
            // TODO: Implement vmcshostptr when VMX is fully implemented
            // if self.in_vmx_guest && self.vmcshostptr != 0 {
            //     let vmcshostptr = self.vmcshostptr as usize;
            //     if vmcshostptr >= addr_ptr && vmcshostptr < end_ptr {
            //         return true;
            //     }
            // }
        }

        // Check VMCB host pointer if SVM is active
        #[cfg(feature = "bx_support_svm")]
        {
            if self.in_svm_guest && self.vmcbhostptr != 0 {
                let vmcbhostptr = self.vmcbhostptr as usize;
                if vmcbhostptr >= addr_ptr && vmcbhostptr < end_ptr {
                    return true;
                }
            }
        }

        // Check DTLB entries
        if self.dtlb.check_addr_in_tlb_buffers(addr_ptr, end_ptr) {
            return true;
        }

        // Check ITLB entries
        if self.itlb.check_addr_in_tlb_buffers(addr_ptr, end_ptr) {
            return true;
        }

        false
    }

    /// Read physical dword (bypasses paging, used for page table walking)
    /// Note: We need to pass a slice of CPU references, but we only have &mut self
    /// So we create a temporary immutable reference (safe because we're only reading)
    fn read_physical_dword(&mut self, paddr: BxPhyAddress, mem: &mut BxMemC) -> Result<u32> {
        // Read directly from memory, bypassing paging
        // We need to pass &[&BxCpuC] but we have &mut self
        // Create a temporary immutable reference - safe because we're only reading
        let mut data = [0u8; 4];
        let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
        let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
        // read_physical_page returns crate::memory::Result which is Result<T, crate::error::Error>
        // We need to convert it to Result<T, CpuError>
        match mem.read_physical_page(&[cpu_ref], paddr, 4, &mut data) {
            Ok(()) => {}
            Err(crate::error::Error::Memory(e)) => return Err(super::CpuError::Memory(e)),
            Err(_) => {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ))
            }
        }
        Ok(u32::from_le_bytes(data))
    }

    /// Write physical dword (bypasses paging, used for updating page table entries)
    fn write_physical_dword(
        &mut self,
        paddr: BxPhyAddress,
        value: u32,
        mem: &mut BxMemC,
        page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
    ) -> Result<()> {
        let mut data = value.to_le_bytes();
        // We need to pass &[&BxCpuC] but we have &mut self
        // Create a temporary immutable reference - safe because write_physical_page doesn't mutate CPU
        let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
        let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
        // write_physical_page returns crate::memory::Result which is Result<T, crate::error::Error>
        // We need to convert it to Result<T, CpuError>
        let result =
            mem.write_physical_page(&[cpu_ref], page_write_stamp_table, paddr, 4, &mut data);
        match result {
            Ok(()) => Ok(()),
            Err(crate::error::Error::Memory(e)) => Err(super::CpuError::Memory(e)),
            Err(_) => Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            )), // Fallback
        }
    }

    /// Translate linear address using legacy 32-bit paging (4KB pages)
    /// Based on BX_CPU_C::translate_linear_legacy in paging.cc:1153
    fn translate_linear_legacy(
        &mut self,
        laddr: BxAddress,
        user: bool,
        rw: MemoryAccessType,
        mem: &mut BxMemC,
        page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
    ) -> Result<BxPhyAddress> {
        // Get page directory base from CR3
        let cr3 = self.cr3;
        let mut ppf = (cr3 & BX_CR3_PAGING_MASK) as u32;

        let mut combined_access = BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER;
        let mut entry_addr = [0u64; 2];
        let mut entry = [0u32; 2];

        // Walk page directory (PDE)
        let pde_index = ((laddr >> 22) & 0x3FF) as u32;
        entry_addr[BX_LEVEL_PDE] = ppf as u64 + (pde_index * 4) as u64;

        entry[BX_LEVEL_PDE] = self.read_physical_dword(entry_addr[BX_LEVEL_PDE], mem)?;

        // Check present bit
        if (entry[BX_LEVEL_PDE] & 0x1) == 0 {
            tracing::debug!("PDE not present: PDE={:#010x}", entry[BX_LEVEL_PDE]);
            // Set CR2 and raise page fault exception
            // Note: We can't modify self here, so we'll return an error that the caller will convert
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        // Extract page frame from PDE
        ppf = entry[BX_LEVEL_PDE] & 0xFFFFF000;

        // Check for 4MB page (PSE bit in PDE, only if CR4.PSE enabled)
        if (entry[BX_LEVEL_PDE] & 0x80) != 0 && self.cr4.pse() {
            // Bochs paging.cc: check reserved bits in PSE PDE
            if (entry[BX_LEVEL_PDE] & PAGING_PDE4M_RESERVED_BITS) != 0 {
                tracing::debug!(
                    "PSE PDE4M: reserved bit is set: PDE={:#010x}",
                    entry[BX_LEVEL_PDE]
                );
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }
            // 4MB page — permission check using combined access from PDE only
            let combined =
                entry[BX_LEVEL_PDE] & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);
            let is_write = matches!(rw, MemoryAccessType::Write);
            let priv_index =
                ((self.cr0.wp() as u32) << 4) | ((user as u32) << 3) | combined | (is_write as u32);
            if PRIV_CHECK[priv_index as usize] == 0 {
                tracing::debug!(
                    "4MB page protection violation: laddr={:#x}, priv_index={}",
                    laddr,
                    priv_index
                );
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }
            // Set Accessed + Dirty bits on PDE (PDE is the leaf for 4MB pages)
            let needed = 0x20 | if is_write { 0x40 } else { 0 };
            if entry[BX_LEVEL_PDE] & needed != needed {
                entry[BX_LEVEL_PDE] |= needed;
                self.write_physical_dword(
                    entry_addr[BX_LEVEL_PDE],
                    entry[BX_LEVEL_PDE],
                    mem,
                    page_write_stamp_table,
                )?;
            }
            let ppf_4m = (entry[BX_LEVEL_PDE] & 0xFFC00000) as u64;
            let offset = laddr & 0x3FFFFF;
            return Ok(ppf_4m | offset);
        }

        // Walk page table (PTE)
        let pte_index = ((laddr >> 12) & 0x3FF) as u32;
        entry_addr[BX_LEVEL_PTE] = ppf as u64 + (pte_index * 4) as u64;

        entry[BX_LEVEL_PTE] = self.read_physical_dword(entry_addr[BX_LEVEL_PTE], mem)?;

        // Check present bit
        if (entry[BX_LEVEL_PTE] & 0x1) == 0 {
            tracing::debug!("PTE not present: PTE={:#010x}", entry[BX_LEVEL_PTE]);
            // Set CR2 and raise page fault exception
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        // Combine access bits from PDE and PTE
        combined_access &= entry[BX_LEVEL_PDE]; // U/S and R/W from PDE
        combined_access &= entry[BX_LEVEL_PTE]; // U/S and R/W from PTE

        // Check permissions
        let is_write = matches!(rw, MemoryAccessType::Write);
        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | (combined_access & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER))
            | (is_write as u32);

        if PRIV_CHECK[priv_index as usize] == 0 {
            tracing::debug!(
                "Page protection violation: laddr={:#x}, priv_index={}",
                laddr,
                priv_index
            );
            // Set CR2 and raise page fault exception
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }

        // Update accessed/dirty bits
        self.update_access_dirty(
            &entry_addr,
            &mut entry,
            BX_LEVEL_PTE,
            is_write,
            mem,
            page_write_stamp_table,
        )?;

        // Extract page frame from PTE
        ppf = entry[BX_LEVEL_PTE] & 0xFFFFF000;
        let offset = (laddr & 0xFFF) as u32;

        Ok((ppf as u64) | (offset as u64))
    }

    /// Update accessed and dirty bits in page table entries
    fn update_access_dirty(
        &mut self,
        entry_addr: &[u64; 2],
        entry: &mut [u32; 2],
        leaf: usize,
        write: bool,
        mem: &mut BxMemC,
        page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
    ) -> Result<()> {
        // Update PDE accessed bit if needed (when accessing PTE)
        if leaf == BX_LEVEL_PTE {
            if (entry[BX_LEVEL_PDE] & 0x20) == 0 {
                entry[BX_LEVEL_PDE] |= 0x20;
                self.write_physical_dword(
                    entry_addr[BX_LEVEL_PDE],
                    entry[BX_LEVEL_PDE],
                    mem,
                    page_write_stamp_table,
                )?;
            }
        }

        // Update PTE accessed/dirty bits
        let set_dirty = write && (entry[leaf] & 0x40) == 0;
        if (entry[leaf] & 0x20) == 0 || set_dirty {
            entry[leaf] |= 0x20; // Set accessed bit
            if set_dirty {
                entry[leaf] |= 0x40; // Set dirty bit
            }
            self.write_physical_dword(entry_addr[leaf], entry[leaf], mem, page_write_stamp_table)?;
        }

        Ok(())
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Translate a linear address to a physical address
    /// Based on BX_CPU_C::translate_linear in paging.cc:1261
    /// Returns Ok(paddr) on success, or Err with page fault info that caller should handle
    pub(super) fn translate_linear(
        &mut self,
        _tlb_entry: &TLBEntry,
        laddr: BxAddress,
        user: bool,
        rw: MemoryAccessType,
        a20_mask: BxPhyAddress,
        mem: &mut BxMemC,
        page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
    ) -> Result<BxPhyAddress> {
        // Mask to 32 bits if not in long mode
        let laddr = if self.long_mode() {
            laddr
        } else {
            laddr & 0xFFFFFFFF
        };

        // If paging is disabled, linear address = physical address (with A20 mask)
        if !self.cr0.pg() {
            let paddr = laddr & a20_mask;
            return Ok(paddr);
        }

        // Paging is enabled — dispatch to the appropriate paging mode.
        // Bochs paging.cc:1324-1334
        let result = if self.long_mode() {
            self.translate_linear_long_mode_slow(laddr, user, rw, mem, page_write_stamp_table)
        } else if self.cr4.pae() {
            self.translate_linear_pae_slow(laddr, user, rw, mem, page_write_stamp_table)
        } else {
            self.translate_linear_legacy(laddr, user, rw, mem, page_write_stamp_table)
        };

        match result {
            Ok(paddr) => {
                // Apply A20 mask
                let paddr = paddr & a20_mask;
                tracing::trace!("translate_linear: laddr={:#x} -> paddr={:#x}", laddr, paddr);
                Ok(paddr)
            }
            Err(e) => {
                // Handle page fault - set CR2 and raise exception
                // Based on BX_CPU_C::page_fault in paging.cc:503
                self.cr2 = laddr;
                let is_write = matches!(rw, MemoryAccessType::Write);
                let is_execute = matches!(rw, MemoryAccessType::Execute);
                let mut error_code = match &e {
                    super::CpuError::Memory(
                        crate::memory::MemoryError::PageProtectionViolation,
                    ) => ERROR_PROTECTION | ((user as u32) << 2) | ((is_write as u32) << 1),
                    _ => ERROR_NOT_PRESENT | ((user as u32) << 2) | ((is_write as u32) << 1),
                };
                // Set I/D bit for execute access when PAE+NXE is enabled
                if is_execute && self.cr4.pae() && self.efer.nxe() {
                    error_code |= ERROR_CODE_ACCESS;
                }

                // Raise page fault exception (based on page_fault function in paging.cc:539)
                if let Err(exc_err) = self.exception(super::cpu::Exception::Pf, error_code as u16) {
                    return Err(exc_err);
                }
                Err(e)
            }
        }
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// PAE paging translation (slow path, used by translate_linear for prefetch).
    /// Based on Bochs translate_linear_PAE in paging.cc:1044.
    fn translate_linear_pae_slow(
        &mut self,
        laddr: BxAddress,
        user: bool,
        rw: MemoryAccessType,
        mem: &mut BxMemC,
        _page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
    ) -> Result<BxPhyAddress> {
        let mut combined_access = BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER;
        let mut nx_page = false;

        let mut reserved = PAGING_LEGACY_PAE_RESERVED_BITS;
        if !self.efer.nxe() {
            reserved |= PAGE_DIRECTORY_NX_BIT;
        }

        // ---- PDPTE from cache ----
        let pdpte_index = ((laddr >> 30) & 0x3) as usize;
        let pdpte = self.pdptrcache.entry[pdpte_index];
        if pdpte & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        let mut ppf = pdpte & 0x000F_FFFF_FFFF_F000;

        let mut entry_addr = [0u64; 2];
        let mut entry = [0u64; 2];

        // ---- PDE ----
        entry_addr[BX_LEVEL_PDE] = ppf + (((laddr >> 21) & 0x1FF) << 3);
        let pde_bytes = {
            let mut buf = [0u8; 8];
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            match mem.read_physical_page(&[cpu_ref], entry_addr[BX_LEVEL_PDE], 8, &mut buf) {
                Ok(()) => u64::from_le_bytes(buf),
                Err(_) => {
                    return Err(super::CpuError::Memory(
                        crate::memory::MemoryError::PageNotPresent,
                    ))
                }
            }
        };
        entry[BX_LEVEL_PDE] = pde_bytes;

        if entry[BX_LEVEL_PDE] & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        if entry[BX_LEVEL_PDE] & reserved != 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }
        if entry[BX_LEVEL_PDE] & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        ppf = entry[BX_LEVEL_PDE] & 0x000F_FFFF_FFFF_F000;

        // ---- 2MB large page ----
        if entry[BX_LEVEL_PDE] & 0x80 != 0 {
            if entry[BX_LEVEL_PDE] & PAGING_PAE_PDE2M_RESERVED_BITS != 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }
            ppf = entry[BX_LEVEL_PDE] & 0x000F_FFFF_FFE0_0000;

            combined_access &=
                (entry[BX_LEVEL_PDE] as u32) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);
            let is_write = matches!(rw, MemoryAccessType::Write);
            let is_execute = matches!(rw, MemoryAccessType::Execute);
            let priv_index = ((self.cr0.wp() as u32) << 4)
                | ((user as u32) << 3)
                | combined_access
                | (is_write as u32);
            if PRIV_CHECK[priv_index as usize] == 0 || (nx_page && is_execute) {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }

            // A/D bits
            let needed = 0x20u64 | if is_write { 0x40 } else { 0 };
            if entry[BX_LEVEL_PDE] & needed != needed {
                entry[BX_LEVEL_PDE] |= needed;
                let data = entry[BX_LEVEL_PDE].to_le_bytes();
                let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
                let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
                let _ = mem.write_physical_page(
                    &[cpu_ref],
                    _page_write_stamp_table,
                    entry_addr[BX_LEVEL_PDE],
                    8,
                    &mut data.clone(),
                );
            }
            return Ok(ppf | (laddr & 0x1FFFFF));
        }

        combined_access &= entry[BX_LEVEL_PDE] as u32;

        // ---- PTE ----
        entry_addr[BX_LEVEL_PTE] = ppf + (((laddr >> 12) & 0x1FF) << 3);
        let pte_bytes = {
            let mut buf = [0u8; 8];
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            match mem.read_physical_page(&[cpu_ref], entry_addr[BX_LEVEL_PTE], 8, &mut buf) {
                Ok(()) => u64::from_le_bytes(buf),
                Err(_) => {
                    return Err(super::CpuError::Memory(
                        crate::memory::MemoryError::PageNotPresent,
                    ))
                }
            }
        };
        entry[BX_LEVEL_PTE] = pte_bytes;

        if entry[BX_LEVEL_PTE] & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        if entry[BX_LEVEL_PTE] & reserved != 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }
        if entry[BX_LEVEL_PTE] & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        combined_access &=
            (entry[BX_LEVEL_PTE] as u32) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);
        let is_write = matches!(rw, MemoryAccessType::Write);
        let is_execute = matches!(rw, MemoryAccessType::Execute);
        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | combined_access
            | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 || (nx_page && is_execute) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }

        // A/D bits — PDE gets A, PTE gets A+D
        if entry[BX_LEVEL_PDE] & 0x20 == 0 {
            entry[BX_LEVEL_PDE] |= 0x20;
            let data = entry[BX_LEVEL_PDE].to_le_bytes();
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let _ = mem.write_physical_page(
                &[cpu_ref],
                _page_write_stamp_table,
                entry_addr[BX_LEVEL_PDE],
                8,
                &mut data.clone(),
            );
        }
        let pte_needed = 0x20u64 | if is_write { 0x40 } else { 0 };
        if entry[BX_LEVEL_PTE] & pte_needed != pte_needed {
            entry[BX_LEVEL_PTE] |= pte_needed;
            let data = entry[BX_LEVEL_PTE].to_le_bytes();
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let _ = mem.write_physical_page(
                &[cpu_ref],
                _page_write_stamp_table,
                entry_addr[BX_LEVEL_PTE],
                8,
                &mut data.clone(),
            );
        }

        ppf = entry[BX_LEVEL_PTE] & 0x000F_FFFF_FFFF_F000;
        Ok(ppf | (laddr & 0xFFF))
    }

    /// Long mode paging translation (slow path, used by translate_linear for prefetch).
    /// Based on Bochs translate_linear_long_mode in paging.cc:828.
    fn translate_linear_long_mode_slow(
        &mut self,
        laddr: BxAddress,
        user: bool,
        rw: MemoryAccessType,
        mem: &mut BxMemC,
        _page_write_stamp_table: &mut crate::cpu::icache::BxPageWriteStampTable,
    ) -> Result<BxPhyAddress> {
        let mut combined_access = BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER;
        let mut nx_page = false;

        let mut reserved = PAGING_PAE_PHY_RESERVED_BITS;
        if !self.efer.nxe() {
            reserved |= PAGE_DIRECTORY_NX_BIT;
        }

        let start_leaf = if self.cr4.la57() {
            BX_LEVEL_PML5
        } else {
            BX_LEVEL_PML4
        };
        let mut ppf = self.cr3 & BX_CR3_PAGING_MASK_PAE;
        let mut offset_mask = ((1u64 << self.linaddr_width as u64) - 1) as u64;

        let mut entry_addr = [0u64; 5];
        let mut entry = [0u64; 5];
        let mut leaf = start_leaf;
        let mut lpf_mask = 0xFFFu32;

        loop {
            entry_addr[leaf] = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0x1FF) * 8;

            let entry_val = {
                let mut buf = [0u8; 8];
                let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
                let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
                match mem.read_physical_page(&[cpu_ref], entry_addr[leaf], 8, &mut buf) {
                    Ok(()) => u64::from_le_bytes(buf),
                    Err(_) => {
                        return Err(super::CpuError::Memory(
                            crate::memory::MemoryError::PageNotPresent,
                        ))
                    }
                }
            };
            entry[leaf] = entry_val;

            offset_mask >>= 9;
            let curr_entry = entry[leaf];

            if curr_entry & 0x1 == 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }
            if curr_entry & reserved != 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }
            // PS at PML4/PML5 is reserved
            if curr_entry & 0x80 != 0 && leaf > BX_LEVEL_PDPTE {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }
            if curr_entry & PAGE_DIRECTORY_NX_BIT != 0 {
                nx_page = true;
            }

            ppf = curr_entry & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            if curr_entry & 0x80 != 0 {
                ppf &= 0x000F_FFFF_FFFF_E000;
                if ppf & offset_mask != 0 {
                    return Err(super::CpuError::Memory(
                        crate::memory::MemoryError::PageProtectionViolation,
                    ));
                }
                lpf_mask = offset_mask as u32;
                break;
            }

            combined_access &= curr_entry as u32;
            leaf -= 1;
        }

        // Leaf permission check
        combined_access &=
            (entry[leaf] as u32) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);
        let is_write = matches!(rw, MemoryAccessType::Write);
        let is_execute = matches!(rw, MemoryAccessType::Execute);
        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | combined_access
            | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 || (nx_page && is_execute) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }

        // A/D bits
        for level in (leaf + 1..=start_leaf).rev() {
            if entry[level] & 0x20 == 0 {
                entry[level] |= 0x20;
                let data = entry[level].to_le_bytes();
                let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
                let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
                let _ = mem.write_physical_page(
                    &[cpu_ref],
                    _page_write_stamp_table,
                    entry_addr[level],
                    8,
                    &mut data.clone(),
                );
            }
        }
        let leaf_needed = 0x20u64 | if is_write { 0x40 } else { 0 };
        if entry[leaf] & leaf_needed != leaf_needed {
            entry[leaf] |= leaf_needed;
            let data = entry[leaf].to_le_bytes();
            let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
            let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
            let _ = mem.write_physical_page(
                &[cpu_ref],
                _page_write_stamp_table,
                entry_addr[leaf],
                8,
                &mut data.clone(),
            );
        }

        Ok(ppf | (laddr & lpf_mask as u64))
    }
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    /// Page table walk for system writes (CPL=0).
    /// Updates Accessed/Dirty bits on PDE/PTE as required by x86 paging.
    /// Used by system_write_byte/word/dword for TSS, descriptor table writes.
    /// Based on Bochs access.cc system_write_word/dword which call
    /// access_write_linear → translate_linear with CPL=0.
    pub(super) fn translate_linear_system_write(
        &mut self,
        laddr: BxAddress,
    ) -> Result<BxPhyAddress> {
        let laddr = if self.long_mode() {
            laddr
        } else {
            laddr & 0xFFFFFFFF
        };

        // If paging disabled, linear = physical
        if !self.cr0.pg() {
            return Ok(laddr & self.a20_mask);
        }

        // Dispatch based on paging mode
        if self.long_mode() {
            return self.translate_linear_system_write_long_mode(laddr);
        }
        if self.cr4.pae() {
            return self.translate_linear_system_write_pae(laddr);
        }

        // Legacy 32-bit paging: two-level page table walk
        let cr3 = self.cr3;
        let ppf = (cr3 & BX_CR3_PAGING_MASK) as u32;

        // Read PDE (use fast host pointer path for page table entries)
        let pde_index = ((laddr >> 22) & 0x3FF) as u32;
        let pde_addr = ppf as u64 + (pde_index * 4) as u64;
        let pde = self.page_walk_read_dword(pde_addr);

        if (pde & 0x1) == 0 {
            tracing::debug!(
                "system_write page walk: PDE not present at {:#x}, laddr={:#x}",
                pde_addr,
                laddr
            );
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        // Check for 4MB page (PSE)
        if (pde & 0x80) != 0 && self.cr4.pse() {
            // Bochs paging.cc: check reserved bits in PSE PDE
            if (pde & PAGING_PDE4M_RESERVED_BITS) != 0 {
                tracing::debug!(
                    "system_write PSE PDE4M: reserved bit set: PDE={:#010x}",
                    pde
                );
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }
            // Set Accessed + Dirty bits on PDE for 4MB page
            let needed = 0x20 | 0x40; // A + D for write
            if pde & needed != needed {
                self.page_walk_write_dword(pde_addr, pde | needed);
            }
            let ppf_4m = (pde & 0xFFC00000) as u64;
            let offset = laddr & 0x3FFFFF;
            return Ok((ppf_4m | offset) & self.a20_mask);
        }

        // Read PTE
        let pt_base = (pde & 0xFFFFF000) as u64;
        let pte_index = ((laddr >> 12) & 0x3FF) as u32;
        let pte_addr = pt_base + (pte_index * 4) as u64;
        let pte = self.page_walk_read_dword(pte_addr);

        if (pte & 0x1) == 0 {
            tracing::debug!(
                "system_write page walk: PTE not present at {:#x}, laddr={:#x}",
                pte_addr,
                laddr
            );
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        // Set Accessed bit on PDE if needed
        if pde & 0x20 == 0 {
            self.page_walk_write_dword(pde_addr, pde | 0x20);
        }

        // Set Accessed + Dirty bits on PTE for write
        let pte_needed = 0x20 | 0x40; // A + D
        if pte & pte_needed != pte_needed {
            self.page_walk_write_dword(pte_addr, pte | pte_needed);
        }

        let page_base = (pte & 0xFFFFF000) as u64;
        let offset = laddr & 0xFFF;
        Ok((page_base | offset) & self.a20_mask)
    }

    /// PAE paging system write translation (3-level, 64-bit entries, CPL=0).
    fn translate_linear_system_write_pae(&mut self, laddr: BxAddress) -> Result<BxPhyAddress> {
        // PDPTE from cache
        let pdpte_index = ((laddr >> 30) & 0x3) as usize;
        let pdpte = self.pdptrcache.entry[pdpte_index];
        if pdpte & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        let mut ppf = pdpte & 0x000F_FFFF_FFFF_F000;

        // PDE
        let pde_addr = ppf + (((laddr >> 21) & 0x1FF) << 3);
        let mut pde = self.page_walk_read_qword(pde_addr);
        if pde & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        ppf = pde & 0x000F_FFFF_FFFF_F000;

        // 2MB page
        if pde & 0x80 != 0 {
            if pde & PAGING_PAE_PDE2M_RESERVED_BITS != 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }
            let needed = 0x20u64 | 0x40;
            if pde & needed != needed {
                pde |= needed;
                self.page_walk_write_qword(pde_addr, pde);
            }
            ppf = pde & 0x000F_FFFF_FFE0_0000;
            return Ok((ppf | (laddr & 0x1FFFFF)) & self.a20_mask);
        }

        // PTE
        let pte_addr = ppf + (((laddr >> 12) & 0x1FF) << 3);
        let mut pte = self.page_walk_read_qword(pte_addr);
        if pte & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        // A bit on PDE
        if pde & 0x20 == 0 {
            pde |= 0x20;
            self.page_walk_write_qword(pde_addr, pde);
        }
        // A+D on PTE
        let pte_needed = 0x20u64 | 0x40;
        if pte & pte_needed != pte_needed {
            pte |= pte_needed;
            self.page_walk_write_qword(pte_addr, pte);
        }

        ppf = pte & 0x000F_FFFF_FFFF_F000;
        Ok((ppf | (laddr & 0xFFF)) & self.a20_mask)
    }

    /// Long mode paging system write translation (4-level, 64-bit entries, CPL=0).
    fn translate_linear_system_write_long_mode(
        &mut self,
        laddr: BxAddress,
    ) -> Result<BxPhyAddress> {
        let start_leaf = if self.cr4.la57() {
            BX_LEVEL_PML5
        } else {
            BX_LEVEL_PML4
        };
        let mut ppf = self.cr3 & BX_CR3_PAGING_MASK_PAE;
        let mut offset_mask = ((1u64 << self.linaddr_width as u64) - 1) as u64;

        let mut entry_addr = [0u64; 5];
        let mut entry = [0u64; 5];
        let mut leaf = start_leaf;

        loop {
            entry_addr[leaf] = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0x1FF) * 8;
            entry[leaf] = self.page_walk_read_qword(entry_addr[leaf]);

            offset_mask >>= 9;

            if entry[leaf] & 0x1 == 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }

            ppf = entry[leaf] & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            if entry[leaf] & 0x80 != 0 {
                ppf &= 0x000F_FFFF_FFFF_E000;
                if ppf & offset_mask != 0 {
                    return Err(super::CpuError::Memory(
                        crate::memory::MemoryError::PageNotPresent,
                    ));
                }
                break;
            }

            leaf -= 1;
        }

        // A/D bits — non-leaf get A, leaf gets A+D
        for level in (leaf + 1..=start_leaf).rev() {
            if entry[level] & 0x20 == 0 {
                entry[level] |= 0x20;
                self.page_walk_write_qword(entry_addr[level], entry[level]);
            }
        }
        let leaf_needed = 0x20u64 | 0x40;
        if entry[leaf] & leaf_needed != leaf_needed {
            entry[leaf] |= leaf_needed;
            self.page_walk_write_qword(entry_addr[leaf], entry[leaf]);
        }

        let paddr = ppf | (laddr & offset_mask);
        Ok(paddr & self.a20_mask)
    }

    /// Lightweight page table walk for system reads (CPL=0, read-only).
    /// Uses mem_read_dword (reads from physical memory via mem_ptr).
    /// Does NOT update accessed/dirty bits or go through TLB.
    /// Matching Bochs translate_linear_legacy but read-only, no side effects.
    pub(super) fn translate_linear_system_read(&self, laddr: BxAddress) -> Result<BxPhyAddress> {
        let laddr = if self.long_mode() {
            laddr
        } else {
            laddr & 0xFFFFFFFF
        };

        // If paging disabled, linear = physical
        if !self.cr0.pg() {
            return Ok(laddr & self.a20_mask);
        }

        // Dispatch based on paging mode
        if self.long_mode() {
            return self.translate_linear_system_read_long_mode(laddr);
        }
        if self.cr4.pae() {
            return self.translate_linear_system_read_pae(laddr);
        }

        // Legacy 32-bit paging: two-level page table walk
        let cr3 = self.cr3;
        let ppf = (cr3 & BX_CR3_PAGING_MASK) as u32;

        // Read PDE (use fast host pointer path for page table entries)
        let pde_index = ((laddr >> 22) & 0x3FF) as u32;
        let pde_addr = ppf as u64 + (pde_index * 4) as u64;
        let pde = self.page_walk_read_dword_ro(pde_addr);

        if (pde & 0x1) == 0 {
            tracing::debug!(
                "system_read page walk: PDE not present at {:#x}, laddr={:#x}",
                pde_addr,
                laddr
            );
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        // Check for 4MB page (PSE)
        if (pde & 0x80) != 0 && self.cr4.pse() {
            let ppf_4m = (pde & 0xFFC00000) as u64;
            let offset = laddr & 0x3FFFFF;
            return Ok((ppf_4m | offset) & self.a20_mask);
        }

        // Read PTE
        let pt_base = (pde & 0xFFFFF000) as u64;
        let pte_index = ((laddr >> 12) & 0x3FF) as u32;
        let pte_addr = pt_base + (pte_index * 4) as u64;
        let pte = self.page_walk_read_dword_ro(pte_addr);

        if (pte & 0x1) == 0 {
            tracing::debug!(
                "system_read page walk: PTE not present at {:#x}, laddr={:#x}",
                pte_addr,
                laddr
            );
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        let page_base = (pte & 0xFFFFF000) as u64;
        let offset = laddr & 0xFFF;
        Ok((page_base | offset) & self.a20_mask)
    }

    /// PAE paging system read translation (read-only, no A/D updates).
    fn translate_linear_system_read_pae(&self, laddr: BxAddress) -> Result<BxPhyAddress> {
        let pdpte_index = ((laddr >> 30) & 0x3) as usize;
        let pdpte = self.pdptrcache.entry[pdpte_index];
        if pdpte & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        let ppf = pdpte & 0x000F_FFFF_FFFF_F000;

        let pde_addr = ppf + (((laddr >> 21) & 0x1FF) << 3);
        let pde = self.page_walk_read_qword(pde_addr);
        if pde & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        if pde & 0x80 != 0 {
            let ppf_2m = pde & 0x000F_FFFF_FFE0_0000;
            return Ok((ppf_2m | (laddr & 0x1FFFFF)) & self.a20_mask);
        }

        let ppf = pde & 0x000F_FFFF_FFFF_F000;
        let pte_addr = ppf + (((laddr >> 12) & 0x1FF) << 3);
        let pte = self.page_walk_read_qword(pte_addr);
        if pte & 0x1 == 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        let page_base = pte & 0x000F_FFFF_FFFF_F000;
        Ok((page_base | (laddr & 0xFFF)) & self.a20_mask)
    }

    /// Long mode paging system read translation (read-only, no A/D updates).
    fn translate_linear_system_read_long_mode(&self, laddr: BxAddress) -> Result<BxPhyAddress> {
        let start_leaf = if self.cr4.la57() {
            BX_LEVEL_PML5
        } else {
            BX_LEVEL_PML4
        };
        let mut ppf = self.cr3 & BX_CR3_PAGING_MASK_PAE;
        let mut offset_mask = ((1u64 << self.linaddr_width as u64) - 1) as u64;
        let mut leaf = start_leaf;

        loop {
            let entry_addr = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0x1FF) * 8;
            let entry = self.page_walk_read_qword(entry_addr);

            offset_mask >>= 9;

            if entry & 0x1 == 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }

            ppf = entry & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            if entry & 0x80 != 0 {
                ppf &= 0x000F_FFFF_FFFF_E000;
                break;
            }

            leaf -= 1;
        }

        let paddr = ppf | (laddr & offset_mask);
        Ok(paddr & self.a20_mask)
    }

    /// Apply the A20 mask to a physical address.
    #[inline]
    fn apply_a20(&self, paddr: u64) -> u64 {
        paddr & self.a20_mask
    }

    /// Deliver a #PF exception.
    fn page_fault(&mut self, fault: u32, laddr: u64, user: bool, is_write: bool) -> Result<()> {
        self.cr2 = laddr;
        let error_code = fault | ((user as u32) << 2) | ((is_write as u32) << 1);
        self.exception(super::cpu::Exception::Pf, error_code as u16)
    }

    /// Translate a linear address to physical for a data read.
    ///
    /// When paging is disabled (CR0.PG=0) this just applies the A20 mask.
    /// Otherwise it performs a two-level page-table walk using the physical
    /// memory bus (`mem_read_dword` / `mem_write_dword`), exactly matching
    /// Bochs `translate_linear` for legacy 32-bit paging.
    #[inline]
    pub(super) fn translate_data_read(&mut self, laddr: u64) -> Result<u64> {
        self.translate_data_access(laddr, false)
    }

    /// Translate a linear address to physical for a data write.
    #[inline]
    pub(super) fn translate_data_write(&mut self, laddr: u64) -> Result<u64> {
        self.translate_data_access(laddr, true)
    }

    #[inline]
    fn translate_data_access(&mut self, laddr: u64, is_write: bool) -> Result<u64> {
        // Mask to 32 bits if not in long mode
        let laddr = if self.long_mode() {
            laddr
        } else {
            laddr & 0xFFFF_FFFF
        };

        // Paging disabled → linear == physical (modulo A20).
        if !self.cr0.pg() {
            return Ok(self.apply_a20(laddr));
        }

        let user = self.user_pl;
        let lpf = laddr & 0xFFFF_F000; // linear page frame

        // ---- DTLB lookup ----
        // Compute which access bit we need:
        //   read  + supervisor(0) → bit 0 (TLB_SYS_READ_OK)
        //   read  + user(1)       → bit 1 (TLB_USER_READ_OK)
        //   write + supervisor(0) → bit 2 (TLB_SYS_WRITE_OK)
        //   write + user(1)       → bit 3 (TLB_USER_WRITE_OK)
        let needed_bit = 1u32 << (((is_write as u32) << 1) | (user as u32));
        {
            let tlb_entry = self.dtlb.get_entry_of(laddr, 0);
            if tlb_entry.lpf == lpf && (tlb_entry.access_bits & needed_bit) != 0 {
                // TLB hit — return cached physical address directly.
                let paddr = tlb_entry.ppf | (laddr & 0xFFF);
                return Ok(paddr);
            }
        }

        // ---- DTLB miss — full page table walk ----
        let (paddr, combined_access, lpf_mask) = self.page_walk_for_dtlb(laddr, user, is_write)?;
        let paddr = self.apply_a20(paddr);
        let is_large_page = lpf_mask > 0xFFF;

        // ---- Populate DTLB entry ----
        // Compute full access bits for this page so future accesses with
        // different user/write combinations can also hit the TLB.
        let wp = self.cr0.wp() as u32;
        let mut access_bits = 0u32;
        // Check all 4 combinations: {sys_read, user_read, sys_write, user_write}
        for &(bit, u, w) in &[
            (TLB_SYS_READ_OK, 0u32, 0u32),
            (TLB_USER_READ_OK, 1, 0),
            (TLB_SYS_WRITE_OK, 0, 1),
            (TLB_USER_WRITE_OK, 1, 1),
        ] {
            let priv_index = (wp << 4) | (u << 3) | combined_access | w;
            if PRIV_CHECK[priv_index as usize] != 0 {
                access_bits |= bit;
            }
        }
        // For writes, we also need the dirty bit to have been set.
        // If this was a read access but the page is writable, the dirty bit
        // may not be set yet. When a future write hits this TLB entry, we
        // need to ensure the dirty bit gets set. We handle this by only
        // granting write permission in the TLB if the dirty bit is already set,
        // OR if this was a write access (which already set the dirty bit).
        // For simplicity and correctness, only grant write TLB permission
        // when the current access is a write (dirty bit was just set).
        if !is_write {
            access_bits &= !(TLB_SYS_WRITE_OK | TLB_USER_WRITE_OK);
        }

        let ppf = paddr & 0xFFFF_F000;

        // Pre-compute host page address before borrowing the TLB entry mutably.
        // Cache host pointer for direct memory access on future TLB hits.
        // Bochs stores hostPageAddr in each TLB entry so that subsequent accesses
        // to the same page bypass get_host_mem_addr() entirely.
        // Pages with MMIO handlers (VGA 0xA0000-0xBFFFF) or ROM get host_page_addr=0
        // and fall through to the slow handler-based path.
        let host_page_addr = {
            let a20_ppf = self.apply_a20(ppf) as usize;
            let host_base = self.mem_host_base;
            let host_len = self.mem_host_len;
            if !host_base.is_null()
                && (a20_ppf < 0xA0000 || (a20_ppf >= 0x100000 && a20_ppf < host_len))
            {
                (unsafe { host_base.add(a20_ppf) }) as BxHostpageaddr
            } else {
                0
            }
        };

        {
            let tlb_entry = self.dtlb.get_entry_of(laddr, 0);
            tlb_entry.lpf = lpf;
            tlb_entry.ppf = ppf;
            tlb_entry.access_bits = access_bits;
            tlb_entry.lpf_mask = lpf_mask;
            tlb_entry.host_page_addr = host_page_addr;
        }

        if is_large_page {
            self.dtlb.split_large = true;
        }

        Ok(paddr)
    }

    /// Fast physical dword read for page walks — bypass full mem_read_dword overhead.
    /// Page table entries are always in plain RAM, so we can use mem_host_base directly.
    /// Used by both &self and &mut self callers.
    #[inline(always)]
    fn page_walk_read_dword_ro(&self, paddr: u64) -> u32 {
        self.page_walk_read_dword(paddr)
    }

    /// Fast physical dword read for page walks (mutable self variant).
    #[inline(always)]
    fn page_walk_read_dword(&self, paddr: u64) -> u32 {
        let a20_addr = (paddr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null() && a20_addr + 4 <= self.mem_host_len {
            return unsafe { (host_base.add(a20_addr) as *const u32).read_unaligned() };
        }
        // Fallback for addresses outside RAM (shouldn't happen for page tables)
        self.mem_read_dword(paddr)
    }

    /// Fast physical dword write for page walk A/D bit updates.
    #[inline(always)]
    fn page_walk_write_dword(&mut self, paddr: u64, val: u32) {
        let a20_addr = (paddr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null() && a20_addr + 4 <= self.mem_host_len {
            unsafe { (host_base.add(a20_addr) as *mut u32).write_unaligned(val) };
            return;
        }
        self.mem_write_dword(paddr, val);
    }

    /// Fast physical qword (64-bit) read for PAE/long mode page walks.
    #[inline(always)]
    fn page_walk_read_qword(&self, paddr: u64) -> u64 {
        let a20_addr = (paddr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null() && a20_addr + 8 <= self.mem_host_len {
            return unsafe { (host_base.add(a20_addr) as *const u64).read_unaligned() };
        }
        // Fallback
        self.mem_read_qword(paddr)
    }

    /// Fast physical qword (64-bit) write for PAE/long mode A/D bit updates.
    #[inline(always)]
    fn page_walk_write_qword(&mut self, paddr: u64, val: u64) {
        let a20_addr = (paddr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null() && a20_addr + 8 <= self.mem_host_len {
            unsafe { (host_base.add(a20_addr) as *mut u64).write_unaligned(val) };
            return;
        }
        self.mem_write_qword(paddr, val);
    }

    /// Load PDPTE entries from physical memory into the PDPTR cache.
    /// Called when CR3 is written in PAE mode (not long mode).
    /// Based on Bochs CheckPDPTR (paging.cc:958-993).
    pub(super) fn load_pdptrs(&mut self) {
        let cr3_val = self.cr3 & 0xFFFF_FFE0; // bits 31:5 of CR3
        for n in 0..4usize {
            let pdpe_addr = cr3_val | ((n as u64) << 3);
            let pdptr = self.page_walk_read_qword(pdpe_addr);
            self.pdptrcache.entry[n] = pdptr;
            // Bochs validates reserved bits and returns false on violation,
            // which causes #GP(0). We check reserved bits at walk time.
        }
    }

    /// Perform the actual page table walk for a data access.
    /// Returns (physical_address_before_a20, combined_access_bits, lpf_mask).
    /// The combined_access_bits are the intersection of PDE and PTE R/W + U/S bits
    /// (only bits 1 and 2), suitable for indexing into PRIV_CHECK.
    /// lpf_mask is 0xFFF for 4K pages, 0x1FFFFF for 2M, 0x3FFFFF for 4M, 0x3FFFFFFF for 1G.
    fn page_walk_for_dtlb(
        &mut self,
        laddr: u64,
        user: bool,
        is_write: bool,
    ) -> Result<(u64, u32, u32)> {
        if self.long_mode() {
            return self.page_walk_for_dtlb_long_mode(laddr, user, is_write);
        }
        if self.cr4.pae() {
            return self.page_walk_for_dtlb_pae(laddr, user, is_write);
        }
        self.page_walk_for_dtlb_legacy(laddr, user, is_write)
    }

    /// Legacy 32-bit paging page walk for DTLB (2-level, 32-bit entries).
    fn page_walk_for_dtlb_legacy(
        &mut self,
        laddr: u64,
        user: bool,
        is_write: bool,
    ) -> Result<(u64, u32, u32)> {
        // ---- PDE ----
        let pde_addr = (self.cr3 & BX_CR3_PAGING_MASK) | (((laddr >> 22) & 0x3FF) << 2);
        let pde = self.page_walk_read_dword(pde_addr);

        if pde & 0x1 == 0 {
            self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // ---- 4 MB page (PSE) ----
        if pde & 0x80 != 0 && self.cr4.pse() {
            // Bochs paging.cc: check reserved bits in PSE PDE
            if (pde & PAGING_PDE4M_RESERVED_BITS) != 0 {
                tracing::debug!("PSE PDE4M: reserved bit is set: PDE={:#010x}", pde);
                self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
            let combined = pde & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);
            let priv_index =
                ((self.cr0.wp() as u32) << 4) | ((user as u32) << 3) | combined | (is_write as u32);
            if PRIV_CHECK[priv_index as usize] == 0 {
                self.page_fault(ERROR_PROTECTION, laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
            // Set A/D bits on the PDE.
            let needed = 0x20 | if is_write { 0x40 } else { 0 };
            if pde & needed != needed {
                self.page_walk_write_dword(pde_addr, pde | needed);
            }
            let paddr = (pde as u64 & 0xFFC0_0000) | (laddr & 0x003F_FFFF);
            return Ok((paddr, combined, 0x3F_FFFF)); // 4MB lpf_mask
        }

        // ---- PTE ----
        let pte_addr = (pde as u64 & 0xFFFF_F000) | (((laddr >> 12) & 0x3FF) << 2);
        let pte = self.page_walk_read_dword(pte_addr);

        if pte & 0x1 == 0 {
            self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        let combined = (pde & pte) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);
        let priv_index =
            ((self.cr0.wp() as u32) << 4) | ((user as u32) << 3) | combined | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 {
            self.page_fault(ERROR_PROTECTION, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Set A bit on PDE if needed.
        if pde & 0x20 == 0 {
            self.page_walk_write_dword(pde_addr, pde | 0x20);
        }
        // Set A/D bits on PTE.
        let pte_needed = 0x20 | if is_write { 0x40 } else { 0 };
        if pte & pte_needed != pte_needed {
            self.page_walk_write_dword(pte_addr, pte | pte_needed);
        }

        let paddr = (pte as u64 & 0xFFFF_F000) | (laddr & 0xFFF);
        Ok((paddr, combined, 0xFFF)) // 4KB lpf_mask
    }

    /// PAE paging page walk for DTLB (3-level: PDPTE -> PDE -> PTE, 64-bit entries).
    /// Based on Bochs translate_linear_PAE in paging.cc:1044.
    fn page_walk_for_dtlb_pae(
        &mut self,
        laddr: u64,
        user: bool,
        is_write: bool,
    ) -> Result<(u64, u32, u32)> {
        let mut combined_access = BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER;
        let mut nx_page = false;

        // Compute reserved bits mask — in legacy PAE, bits [62:52] are reserved.
        // If NXE is not enabled, bit 63 is also reserved.
        let mut reserved = PAGING_LEGACY_PAE_RESERVED_BITS;
        if !self.efer.nxe() {
            reserved |= PAGE_DIRECTORY_NX_BIT;
        }

        // ---- PDPTE ----
        // In legacy PAE mode, PDPTRs are cached. Load from PDPTR_CACHE.
        let pdpte_index = ((laddr >> 30) & 0x3) as usize;
        let pdpte = self.pdptrcache.entry[pdpte_index];

        if pdpte & 0x1 == 0 {
            self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // PDPTE reserved bit check is done at CR3 load time (CheckPDPTR),
        // but also verify here for safety.
        if pdpte & PAGING_PAE_PDPTE_RESERVED_BITS != 0 {
            tracing::debug!("PAE PDPTE: reserved bit set: {:#018x}", pdpte);
            self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        let mut ppf = pdpte & 0x000F_FFFF_FFFF_F000;

        // ---- PDE ----
        let mut entry_addr = [0u64; 2];
        let mut entry = [0u64; 2];

        entry_addr[BX_LEVEL_PDE] = ppf + (((laddr >> (9 + 9)) & 0x1FF) << 3);
        entry[BX_LEVEL_PDE] = self.page_walk_read_qword(entry_addr[BX_LEVEL_PDE]);

        // Check present
        if entry[BX_LEVEL_PDE] & 0x1 == 0 {
            self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check reserved bits
        if entry[BX_LEVEL_PDE] & reserved != 0 {
            tracing::debug!(
                "PAE PDE: reserved bit set: {:#018x} (reserved mask: {:#018x})",
                entry[BX_LEVEL_PDE],
                entry[BX_LEVEL_PDE] & reserved
            );
            self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check NX bit
        if entry[BX_LEVEL_PDE] & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        ppf = entry[BX_LEVEL_PDE] & 0x000F_FFFF_FFFF_F000;

        // ---- 2MB large page (PDE.PS=1) ----
        // In PAE mode, CR4.PSE is ignored — PS bit in PDE is always checked.
        if entry[BX_LEVEL_PDE] & 0x80 != 0 {
            // Check 2MB PDE reserved bits (bits 20:13 must be zero)
            if entry[BX_LEVEL_PDE] & PAGING_PAE_PDE2M_RESERVED_BITS != 0 {
                tracing::debug!("PAE PDE2M: reserved bit set: {:#018x}", entry[BX_LEVEL_PDE]);
                self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // Physical page frame for 2MB page
            ppf = entry[BX_LEVEL_PDE] & 0x000F_FFFF_FFE0_0000;

            // Leaf entry permission check
            combined_access &=
                (entry[BX_LEVEL_PDE] as u32) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);

            let priv_index = ((self.cr0.wp() as u32) << 4)
                | ((user as u32) << 3)
                | combined_access
                | (is_write as u32);
            if PRIV_CHECK[priv_index as usize] == 0 || nx_page {
                // NX page check: in PAE, NX violation causes #PF with ERROR_PROTECTION
                // (for data accesses, NX doesn't apply — only for execute, which is not
                // handled in this data-access path, but we check nx_page for completeness
                // since Bochs does `if (!priv_check[priv_index] || (nx_page && rw == BX_EXECUTE))`)
                // For data access, nx_page has no effect on permission.
                if PRIV_CHECK[priv_index as usize] == 0 {
                    self.page_fault(ERROR_PROTECTION, laddr, user, is_write)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
            }

            // Update A/D bits on PDE (leaf for 2MB page)
            let needed = 0x20u64 | if is_write { 0x40 } else { 0 };
            if entry[BX_LEVEL_PDE] & needed != needed {
                entry[BX_LEVEL_PDE] |= needed;
                self.page_walk_write_qword(entry_addr[BX_LEVEL_PDE], entry[BX_LEVEL_PDE]);
            }

            let paddr = ppf | (laddr & 0x001F_FFFF);
            return Ok((paddr, combined_access, 0x1F_FFFF)); // 2MB lpf_mask
        }

        combined_access &= entry[BX_LEVEL_PDE] as u32; // U/S and R/W from PDE

        // ---- PTE ----
        entry_addr[BX_LEVEL_PTE] = ppf + (((laddr >> 12) & 0x1FF) << 3);
        entry[BX_LEVEL_PTE] = self.page_walk_read_qword(entry_addr[BX_LEVEL_PTE]);

        // Check present
        if entry[BX_LEVEL_PTE] & 0x1 == 0 {
            self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check reserved bits
        if entry[BX_LEVEL_PTE] & reserved != 0 {
            tracing::debug!("PAE PTE: reserved bit set: {:#018x}", entry[BX_LEVEL_PTE]);
            self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check NX on PTE
        if entry[BX_LEVEL_PTE] & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        // Leaf permission check
        combined_access &=
            (entry[BX_LEVEL_PTE] as u32) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);

        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | combined_access
            | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 {
            self.page_fault(ERROR_PROTECTION, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Update A/D bits — PDE gets A bit, PTE gets A+D
        if entry[BX_LEVEL_PDE] & 0x20 == 0 {
            entry[BX_LEVEL_PDE] |= 0x20;
            self.page_walk_write_qword(entry_addr[BX_LEVEL_PDE], entry[BX_LEVEL_PDE]);
        }
        let pte_needed = 0x20u64 | if is_write { 0x40 } else { 0 };
        if entry[BX_LEVEL_PTE] & pte_needed != pte_needed {
            entry[BX_LEVEL_PTE] |= pte_needed;
            self.page_walk_write_qword(entry_addr[BX_LEVEL_PTE], entry[BX_LEVEL_PTE]);
        }

        ppf = entry[BX_LEVEL_PTE] & 0x000F_FFFF_FFFF_F000;
        let paddr = ppf | (laddr & 0xFFF);
        Ok((paddr, combined_access, 0xFFF)) // 4KB lpf_mask
    }

    /// Long mode paging page walk for DTLB (4-level: PML4 -> PDPTE -> PDE -> PTE, 64-bit entries).
    /// Based on Bochs translate_linear_long_mode in paging.cc:828.
    fn page_walk_for_dtlb_long_mode(
        &mut self,
        laddr: u64,
        user: bool,
        is_write: bool,
    ) -> Result<(u64, u32, u32)> {
        let mut combined_access = BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER;
        let mut nx_page = false;

        // Reserved bits: in long mode, bits [62:52] are ignored (NOT reserved).
        // Only PHY_RESERVED bits are reserved. If NXE not enabled, bit 63 is reserved.
        let mut reserved = PAGING_PAE_PHY_RESERVED_BITS;
        if !self.efer.nxe() {
            reserved |= PAGE_DIRECTORY_NX_BIT;
        }

        let mut ppf = self.cr3 & BX_CR3_PAGING_MASK_PAE;
        let mut entry_addr = [0u64; 5];
        let mut entry = [0u64; 5];

        // Determine start level: PML5 if LA57, otherwise PML4
        let start_leaf = if self.cr4.la57() {
            BX_LEVEL_PML5
        } else {
            BX_LEVEL_PML4
        };
        let mut leaf = start_leaf;

        // Offset mask tracks how many bits of the linear address are used as the page offset.
        // We start with the full linear address width mask and shift right 9 bits per level.
        let mut offset_mask = ((1u64 << self.linaddr_width as u64) - 1) as u64;
        let mut lpf_mask = 0xFFFu32;

        loop {
            entry_addr[leaf] = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0x1FF) * 8;
            entry[leaf] = self.page_walk_read_qword(entry_addr[leaf]);

            offset_mask >>= 9;

            let curr_entry = entry[leaf];

            // Check present
            if curr_entry & 0x1 == 0 {
                self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // Check reserved bits
            if curr_entry & reserved != 0 {
                tracing::debug!(
                    "Long mode level {}: reserved bit set: {:#018x}",
                    leaf,
                    curr_entry
                );
                self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // PS bit at invalid level (only PDE level 1 and PDPTE level 2 with 1G support)
            if curr_entry & 0x80 != 0 {
                // PS bit set — valid only at BX_LEVEL_PDE (2MB) and BX_LEVEL_PDPTE (1GB)
                if leaf > BX_LEVEL_PDPTE {
                    // PS at PML4 or PML5 level is reserved
                    self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
            }

            // Check NX
            if curr_entry & PAGE_DIRECTORY_NX_BIT != 0 {
                nx_page = true;
            }

            ppf = curr_entry & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            // Large page?
            if curr_entry & 0x80 != 0 {
                ppf &= 0x000F_FFFF_FFFF_E000; // clear bit 12 for large pages
                if ppf & offset_mask != 0 {
                    tracing::debug!(
                        "Long mode level {}: reserved bits in large page frame: {:#018x}",
                        leaf,
                        curr_entry
                    );
                    self.page_fault(ERROR_RESERVED | ERROR_PROTECTION, laddr, user, is_write)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
                lpf_mask = offset_mask as u32;
                break;
            }

            combined_access &= curr_entry as u32; // Accumulate U/S and R/W from non-leaf entries
            leaf -= 1;
        }

        // Leaf entry permission check
        combined_access &=
            (entry[leaf] as u32) & (BX_COMBINED_ACCESS_WRITE | BX_COMBINED_ACCESS_USER);

        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | combined_access
            | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 {
            self.page_fault(ERROR_PROTECTION, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // SMEP check: supervisor cannot execute from user page
        // (not applicable for data access, handled by translate_linear for execute)

        // Update A/D bits for all levels
        // Non-leaf levels get A bit, leaf gets A+D
        for level in (leaf + 1..=start_leaf).rev() {
            if entry[level] & 0x20 == 0 {
                entry[level] |= 0x20;
                self.page_walk_write_qword(entry_addr[level], entry[level]);
            }
        }
        let leaf_needed = 0x20u64 | if is_write { 0x40 } else { 0 };
        if entry[leaf] & leaf_needed != leaf_needed {
            entry[leaf] |= leaf_needed;
            self.page_walk_write_qword(entry_addr[leaf], entry[leaf]);
        }

        let paddr = ppf | (laddr & lpf_mask as u64);
        Ok((paddr, combined_access, lpf_mask))
    }
}

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    fn is_virtual_apic_page(&self, _p_addr: &BxPhyAddress) -> bool {
        // TODO: Implement virtual APIC page check
        false
    }

    pub(crate) fn get_host_mem_addr(
        &self,
        p_addr: BxPhyAddress,
        rw: MemoryAccessType,
        mem: &'c mut BxMemC<'c>,
    ) -> crate::Result<Option<&'c mut [u8]>> {
        if self.is_virtual_apic_page(&p_addr) {
            return Ok(None); // Do not allow direct access to virtual apic page
        }

        let addr_option = mem.get_host_mem_addr(p_addr, rw, &[self])?;
        Ok(addr_option)
    }
}
