//! Paging support
//!
//! Based on Bochs cpu/paging.cc
//! Implements page table walking and address translation

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, Result};
use crate::{
    config::{BxAddress, BxPhyAddress},
    cpu::{rusty_box::MemoryAccessType, tlb::TLBEntry},
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

// Paging level constants
const BX_LEVEL_PDE: usize = 1;
const BX_LEVEL_PTE: usize = 0;

// CR3 paging mask (bits 31:12)
const BX_CR3_PAGING_MASK: u64 = 0xFFFFF000;

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
        let laddr = laddr & 0xFFFFFFFF;

        // If paging is disabled, linear address = physical address (with A20 mask)
        if !self.cr0.pg() {
            let paddr = laddr & a20_mask;
            // tracing::trace!("translate_linear (no paging): laddr={:#x} -> paddr={:#x}", laddr, paddr);
            return Ok(paddr);
        }

        // Paging is enabled - walk page tables
        // tracing::trace!("translate_linear (paging enabled): laddr={:#x}", laddr);

        // For now, only support legacy 32-bit paging
        // TODO: Add PAE and long mode support
        let result = self.translate_linear_legacy(laddr, user, rw, mem, page_write_stamp_table);

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
                let error_code = match &e {
                    super::CpuError::Memory(
                        crate::memory::MemoryError::PageProtectionViolation,
                    ) => ERROR_PROTECTION | ((user as u32) << 2) | ((is_write as u32) << 1),
                    _ => ERROR_NOT_PRESENT | ((user as u32) << 2) | ((is_write as u32) << 1),
                };

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
    /// Page table walk for system writes (CPL=0).
    /// Updates Accessed/Dirty bits on PDE/PTE as required by x86 paging.
    /// Used by system_write_byte/word/dword for TSS, descriptor table writes.
    /// Based on Bochs access.cc system_write_word/dword which call
    /// access_write_linear → translate_linear with CPL=0.
    pub(super) fn translate_linear_system_write(
        &mut self,
        laddr: BxAddress,
    ) -> Result<BxPhyAddress> {
        let laddr = laddr & 0xFFFFFFFF; // Mask to 32 bits

        // If paging disabled, linear = physical
        if !self.cr0.pg() {
            return Ok(laddr & self.a20_mask);
        }

        // Legacy 32-bit paging: two-level page table walk
        let cr3 = self.cr3;
        let ppf = (cr3 & BX_CR3_PAGING_MASK) as u32;

        // Read PDE
        let pde_index = ((laddr >> 22) & 0x3FF) as u32;
        let pde_addr = ppf as u64 + (pde_index * 4) as u64;
        let pde = self.mem_read_dword(pde_addr);

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
            // Set Accessed + Dirty bits on PDE for 4MB page
            let needed = 0x20 | 0x40; // A + D for write
            if pde & needed != needed {
                self.mem_write_dword(pde_addr, pde | needed);
            }
            let ppf_4m = (pde & 0xFFC00000) as u64;
            let offset = laddr & 0x3FFFFF;
            return Ok((ppf_4m | offset) & self.a20_mask);
        }

        // Read PTE
        let pt_base = (pde & 0xFFFFF000) as u64;
        let pte_index = ((laddr >> 12) & 0x3FF) as u32;
        let pte_addr = pt_base + (pte_index * 4) as u64;
        let pte = self.mem_read_dword(pte_addr);

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
            self.mem_write_dword(pde_addr, pde | 0x20);
        }

        // Set Accessed + Dirty bits on PTE for write
        let pte_needed = 0x20 | 0x40; // A + D
        if pte & pte_needed != pte_needed {
            self.mem_write_dword(pte_addr, pte | pte_needed);
        }

        let page_base = (pte & 0xFFFFF000) as u64;
        let offset = laddr & 0xFFF;
        Ok((page_base | offset) & self.a20_mask)
    }

    /// Lightweight page table walk for system reads (CPL=0, read-only).
    /// Uses mem_read_dword (reads from physical memory via mem_ptr).
    /// Does NOT update accessed/dirty bits or go through TLB.
    /// Matching Bochs translate_linear_legacy but read-only, no side effects.
    pub(super) fn translate_linear_system_read(&self, laddr: BxAddress) -> Result<BxPhyAddress> {
        let laddr = laddr & 0xFFFFFFFF; // Mask to 32 bits

        // If paging disabled, linear = physical
        if !self.cr0.pg() {
            return Ok(laddr & self.a20_mask);
        }

        // Legacy 32-bit paging: two-level page table walk
        let cr3 = self.cr3;
        let ppf = (cr3 & BX_CR3_PAGING_MASK) as u32;

        // Read PDE
        let pde_index = ((laddr >> 22) & 0x3FF) as u32;
        let pde_addr = ppf as u64 + (pde_index * 4) as u64;
        let pde = self.mem_read_dword(pde_addr);

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
        let pte = self.mem_read_dword(pte_addr);

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
    pub(super) fn translate_data_read(&mut self, laddr: u64) -> Result<u64> {
        self.translate_data_access(laddr, false)
    }

    /// Translate a linear address to physical for a data write.
    pub(super) fn translate_data_write(&mut self, laddr: u64) -> Result<u64> {
        self.translate_data_access(laddr, true)
    }

    fn translate_data_access(&mut self, laddr: u64, is_write: bool) -> Result<u64> {
        let laddr = laddr & 0xFFFF_FFFF;

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
        let (paddr, combined_access, is_large_page) =
            self.page_walk_for_dtlb(laddr, user, is_write)?;
        let paddr = self.apply_a20(paddr);

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
        {
            let tlb_entry = self.dtlb.get_entry_of(laddr, 0);
            tlb_entry.lpf = lpf;
            tlb_entry.ppf = ppf;
            tlb_entry.access_bits = access_bits;
            tlb_entry.lpf_mask = if is_large_page { 0x3F_FFFF } else { 0xFFF };
            tlb_entry.host_page_addr = 0; // Not used for data access
        }

        if is_large_page {
            self.dtlb.split_large = true;
        }

        Ok(paddr)
    }

    /// Perform the actual page table walk for a data access.
    /// Returns (physical_address_before_a20, combined_access_bits, is_large_page).
    /// The combined_access_bits are the intersection of PDE and PTE R/W + U/S bits
    /// (only bits 1 and 2), suitable for indexing into PRIV_CHECK.
    fn page_walk_for_dtlb(
        &mut self,
        laddr: u64,
        user: bool,
        is_write: bool,
    ) -> Result<(u64, u32, bool)> {
        // ---- PDE ----
        let pde_addr = (self.cr3 & BX_CR3_PAGING_MASK) | (((laddr >> 22) & 0x3FF) << 2);
        let pde = self.mem_read_dword(pde_addr);

        if pde & 0x1 == 0 {
            self.page_fault(ERROR_NOT_PRESENT, laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // ---- 4 MB page (PSE) ----
        if pde & 0x80 != 0 && self.cr4.pse() {
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
                self.mem_write_dword(pde_addr, pde | needed);
            }
            let paddr = (pde as u64 & 0xFFC0_0000) | (laddr & 0x003F_FFFF);
            return Ok((paddr, combined, true));
        }

        // ---- PTE ----
        let pte_addr = (pde as u64 & 0xFFFF_F000) | (((laddr >> 12) & 0x3FF) << 2);
        let pte = self.mem_read_dword(pte_addr);

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
            self.mem_write_dword(pde_addr, pde | 0x20);
        }
        // Set A/D bits on PTE.
        let pte_needed = 0x20 | if is_write { 0x40 } else { 0 };
        if pte & pte_needed != pte_needed {
            self.mem_write_dword(pte_addr, pte | pte_needed);
        }

        let paddr = (pte as u64 & 0xFFFF_F000) | (laddr & 0xFFF);
        Ok((paddr, combined, false))
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
