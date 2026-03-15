//! Paging support
//!
//! Based on Bochs cpu/paging.cc
//! Implements page table walking and address translation

use super::{cpu::BxCpuC, cpuid::BxCpuIdTrait, Result};
use crate::{
    config::{BxAddress, BxPhyAddress},
    cpu::{
        rusty_box::MemoryAccessType,
        tlb::{BxHostpageaddr, LPF_MASK, TLBEntry},
    },
    memory::BxMemC,
};

use bitflags::bitflags;

bitflags! {
    /// Page fault error code bits (Bochs paging.cc).
    /// Combined to build the error code pushed on #PF exceptions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PageFaultError: u32 {
        /// Page not present (bit 0 clear)
        const NOT_PRESENT  = 0x00;
        /// Protection violation (bit 0 set)
        const PROTECTION   = 0x01;
        /// Caused by a write access (bit 1)
        const WRITE_ACCESS = 0x02;
        /// User-mode access (bit 2)
        const USER_ACCESS  = 0x04;
        /// Reserved bit violation (bit 3)
        const RESERVED     = 0x08;
        /// Instruction fetch (bit 4, NX violation)
        const CODE_ACCESS  = 0x10;
    }
}

bitflags! {
    /// Combined page access permission bits (Bochs paging.cc).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CombinedAccess: u32 {
        const WRITE = 0x2;
        const USER  = 0x4;
    }
}

bitflags! {
    /// DTLB access permission bits (matching Bochs tlb.h).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TlbAccess: u32 {
        const SYS_READ_OK   = 0x01;
        const USER_READ_OK  = 0x02;
        const SYS_WRITE_OK  = 0x04;
        const USER_WRITE_OK = 0x08;
    }
}

bitflags! {
    /// Page table entry flag bits (used in both 32-bit and 64-bit page table entries).
    /// Based on Bochs paging.cc PTE/PDE bit definitions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PteBits: u64 {
        /// Present bit (bit 0)
        const PRESENT  = 0x01;
        /// Read/Write bit (bit 1): 0=read-only, 1=read-write
        const RW       = 0x02;
        /// User/Supervisor bit (bit 2): 0=supervisor, 1=user
        const US       = 0x04;
        /// Page-level Write Through (bit 3)
        const PWT      = 0x08;
        /// Page-level Cache Disable (bit 4)
        const PCD      = 0x10;
        /// Accessed bit (bit 5)
        const ACCESSED = 0x20;
        /// Dirty bit (bit 6)
        const DIRTY    = 0x40;
        /// Page Size bit (bit 7): 1=large page (4MB/2MB/1GB)
        const PS       = 0x80;
    }
}

impl PteBits {
    /// Wrap a raw page table entry value for flag operations.
    /// Retains all bits (address + flags) — safe to call `.bits()` later.
    #[inline(always)]
    pub fn from_raw(raw: u64) -> Self {
        Self::from_bits_retain(raw)
    }
}

/// 32-bit aliases for use in legacy (non-PAE) paging with u32 page entries.
mod pte_bits32 {
    use super::PteBits;
    pub const PRESENT: u32  = PteBits::PRESENT.bits() as u32;
    pub const ACCESSED: u32 = PteBits::ACCESSED.bits() as u32;
    pub const DIRTY: u32    = PteBits::DIRTY.bits() as u32;
    pub const PS: u32       = PteBits::PS.bits() as u32;
}

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

// Physical address width (matching Bochs config.h BX_PHY_ADDRESS_WIDTH for BX_PHY_ADDRESS_LONG)
const BX_PHY_ADDRESS_WIDTH: u32 = 40;

// BX_PAGING_PHY_ADDRESS_RESERVED_BITS = (~((1 << WIDTH) - 1)) & 0x000F_FFFF_FFFF_FFFF
// Strips bit 63 (NX) and bits 62:52 (ignored) — only bits [51:WIDTH] are reserved
const PAGING_PAE_PHY_RESERVED_BITS: u64 = {
    let mask = (1u64 << BX_PHY_ADDRESS_WIDTH) - 1;
    (!mask) & 0x000F_FFFF_FFFF_FFFFu64
};

// Legacy PAE: bits [62:52] are also reserved (bit 63 = NX)
const PAGING_LEGACY_PAE_RESERVED_BITS: u64 = PAGING_PAE_PHY_RESERVED_BITS | 0x7FF0_0000_0000_0000;

// PAE PDE 2MB: bits 20:13 must be zero + PHY reserved
const PAGING_PAE_PDE2M_RESERVED_BITS: u64 = PAGING_PAE_PHY_RESERVED_BITS | 0x001F_E000;

// PAE PDPTE reserved: PHY + bits 63:52, 8:5, 2:1
const PAGING_PAE_PDPTE_RESERVED_BITS: u64 = PAGING_PAE_PHY_RESERVED_BITS | 0xFFF0_0000_0000_01E6;

// Long mode PDPTE 1GB: bits 29:13 + PHY reserved
const PAGING_PAE_PDPTE1G_RESERVED_BITS: u64 = PAGING_PAE_PHY_RESERVED_BITS | 0x3FFF_E000;

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

        let mut combined_access = CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits();
        let mut entry_addr = [0u64; 2];
        let mut entry = [0u32; 2];

        // Walk page directory (PDE)
        let pde_index = ((laddr >> 22) & 0x3FF) as u32;
        entry_addr[BX_LEVEL_PDE] = ppf as u64 + (pde_index * 4) as u64;

        entry[BX_LEVEL_PDE] = self.read_physical_dword(entry_addr[BX_LEVEL_PDE], mem)?;

        // Check present bit
        if (entry[BX_LEVEL_PDE] & pte_bits32::PRESENT) == 0 {
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
        if (entry[BX_LEVEL_PDE] & pte_bits32::PS) != 0 && self.cr4.pse() {
            // Bochs paging.cc: check reserved bits in PSE PDE
            if (entry[BX_LEVEL_PDE] & PAGING_PDE4M_RESERVED_BITS) != 0 {
                tracing::debug!(
                    "PSE PDE4M: reserved bit is set: PDE={:#010x}",
                    entry[BX_LEVEL_PDE]
                );
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageReservedBitViolation,
                ));
            }
            // 4MB page — permission check using combined access from PDE only
            let combined =
                entry[BX_LEVEL_PDE] & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());
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
            let needed = pte_bits32::ACCESSED | if is_write { pte_bits32::DIRTY } else { 0 };
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
        if (entry[BX_LEVEL_PTE] & pte_bits32::PRESENT) == 0 {
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
            | (combined_access & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits()))
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
            if (entry[BX_LEVEL_PDE] & pte_bits32::ACCESSED) == 0 {
                entry[BX_LEVEL_PDE] |= pte_bits32::ACCESSED;
                self.write_physical_dword(
                    entry_addr[BX_LEVEL_PDE],
                    entry[BX_LEVEL_PDE],
                    mem,
                    page_write_stamp_table,
                )?;
            }
        }

        // Update PTE accessed/dirty bits
        let set_dirty = write && (entry[leaf] & pte_bits32::DIRTY) == 0;
        if (entry[leaf] & pte_bits32::ACCESSED) == 0 || set_dirty {
            entry[leaf] |= pte_bits32::ACCESSED; // Set accessed bit
            if set_dirty {
                entry[leaf] |= pte_bits32::DIRTY; // Set dirty bit
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
                        crate::memory::MemoryError::PageReservedBitViolation,
                    ) => PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits() | ((user as u32) << 2) | ((is_write as u32) << 1),
                    super::CpuError::Memory(
                        crate::memory::MemoryError::PageProtectionViolation,
                    ) => PageFaultError::PROTECTION.bits() | ((user as u32) << 2) | ((is_write as u32) << 1),
                    _ => PageFaultError::NOT_PRESENT.bits() | ((user as u32) << 2) | ((is_write as u32) << 1),
                };
                // Set I/D bit for execute access when PAE+NXE is enabled
                if is_execute && self.cr4.pae() && self.efer.nxe() {
                    error_code |= PageFaultError::CODE_ACCESS.bits();
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
        let mut combined_access = CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits();
        let mut nx_page = false;

        let mut reserved = PAGING_LEGACY_PAE_RESERVED_BITS;
        if !self.efer.nxe() {
            reserved |= PAGE_DIRECTORY_NX_BIT;
        }

        // ---- PDPTE from cache ----
        let pdpte_index = ((laddr >> 30) & 0x3) as usize;
        let pdpte = PteBits::from_raw(self.pdptrcache.entry[pdpte_index]);
        if !pdpte.contains(PteBits::PRESENT) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        let mut ppf = pdpte.bits() & 0x000F_FFFF_FFFF_F000;

        let mut entry_addr = [0u64; 2];
        let mut entry = [PteBits::empty(); 2];

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
        entry[BX_LEVEL_PDE] = PteBits::from_raw(pde_bytes);

        if !entry[BX_LEVEL_PDE].contains(PteBits::PRESENT) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        if entry[BX_LEVEL_PDE].bits() & reserved != 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageReservedBitViolation,
            ));
        }
        if entry[BX_LEVEL_PDE].bits() & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        ppf = entry[BX_LEVEL_PDE].bits() & 0x000F_FFFF_FFFF_F000;

        // ---- 2MB large page ----
        if entry[BX_LEVEL_PDE].contains(PteBits::PS) {
            if entry[BX_LEVEL_PDE].bits() & PAGING_PAE_PDE2M_RESERVED_BITS != 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageReservedBitViolation,
                ));
            }
            ppf = entry[BX_LEVEL_PDE].bits() & 0x000F_FFFF_FFE0_0000;

            combined_access &=
                (entry[BX_LEVEL_PDE].bits() as u32) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());
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

            // SMEP check for 2MB page (Bochs paging.cc:740-743)
            if is_execute
                && self.cr4.smep()
                && !user
                && (combined_access & CombinedAccess::USER.bits()) != 0
            {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageProtectionViolation,
                ));
            }

            // A/D bits
            let needed = PteBits::ACCESSED | if is_write { PteBits::DIRTY } else { PteBits::empty() };
            if !entry[BX_LEVEL_PDE].contains(needed) {
                entry[BX_LEVEL_PDE].insert(needed);
                let data = entry[BX_LEVEL_PDE].bits().to_le_bytes();
                let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
                let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
                // A/D bit update on page table entry — write to physical RAM cannot meaningfully fail
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

        combined_access &= entry[BX_LEVEL_PDE].bits() as u32;

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
        entry[BX_LEVEL_PTE] = PteBits::from_raw(pte_bytes);

        if !entry[BX_LEVEL_PTE].contains(PteBits::PRESENT) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        if entry[BX_LEVEL_PTE].bits() & reserved != 0 {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageReservedBitViolation,
            ));
        }
        if entry[BX_LEVEL_PTE].bits() & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        combined_access &=
            (entry[BX_LEVEL_PTE].bits() as u32) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());
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

        // SMEP check: supervisor cannot execute from user page (Bochs paging.cc:740-743)
        if is_execute
            && self.cr4.smep()
            && !user
            && (combined_access & CombinedAccess::USER.bits()) != 0
        {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }

        // A/D bits — PDE gets A, PTE gets A+D
        if !entry[BX_LEVEL_PDE].contains(PteBits::ACCESSED) {
            entry[BX_LEVEL_PDE].insert(PteBits::ACCESSED);
            let data = entry[BX_LEVEL_PDE].bits().to_le_bytes();
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
        let pte_needed = PteBits::ACCESSED | if is_write { PteBits::DIRTY } else { PteBits::empty() };
        if !entry[BX_LEVEL_PTE].contains(pte_needed) {
            entry[BX_LEVEL_PTE].insert(pte_needed);
            let data = entry[BX_LEVEL_PTE].bits().to_le_bytes();
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

        ppf = entry[BX_LEVEL_PTE].bits() & 0x000F_FFFF_FFFF_F000;
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
        let mut combined_access = CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits();
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
        let mut entry = [PteBits::empty(); 5];
        let mut leaf = start_leaf;
        let mut lpf_mask = 0xFFFu32;

        loop {
            // Bochs paging.cc: entry_addr[leaf] = ppf + ((laddr >> (9 + 9*leaf)) & 0xff8);
            // The & 0xFF8 mask extracts bits 11:3 of shifted value = 9-bit index * 8
            entry_addr[leaf] = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0xFF8);

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
            entry[leaf] = PteBits::from_raw(entry_val);

            offset_mask >>= 9;
            let curr_entry = entry[leaf];

            if !curr_entry.contains(PteBits::PRESENT) {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }
            if curr_entry.bits() & reserved != 0 {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageReservedBitViolation,
                ));
            }
            // PS at PML4/PML5 is reserved
            if curr_entry.contains(PteBits::PS) && leaf > BX_LEVEL_PDPTE {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageReservedBitViolation,
                ));
            }
            if curr_entry.bits() & PAGE_DIRECTORY_NX_BIT != 0 {
                nx_page = true;
            }

            ppf = curr_entry.bits() & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            if curr_entry.contains(PteBits::PS) {
                ppf &= 0x000F_FFFF_FFFF_E000;
                if ppf & offset_mask != 0 {
                    return Err(super::CpuError::Memory(
                        crate::memory::MemoryError::PageReservedBitViolation,
                    ));
                }
                lpf_mask = offset_mask as u32;
                break;
            }

            combined_access &= curr_entry.bits() as u32;
            leaf -= 1;
        }

        // Leaf permission check
        combined_access &=
            (entry[leaf].bits() as u32) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());
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

        // SMEP check: supervisor cannot execute from user page (Bochs paging.cc:740-743)
        if is_execute
            && self.cr4.smep()
            && !user
            && (combined_access & CombinedAccess::USER.bits()) != 0
        {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }

        // SMAP check: supervisor data access to user page when AC=0 (Bochs paging.cc:746-749)
        if !is_execute
            && self.cr4.smap()
            && !user
            && self.get_ac() == 0
            && (combined_access & CombinedAccess::USER.bits()) != 0
        {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageProtectionViolation,
            ));
        }

        // A/D bits
        for level in (leaf + 1..=start_leaf).rev() {
            if !entry[level].contains(PteBits::ACCESSED) {
                entry[level].insert(PteBits::ACCESSED);
                let data = entry[level].bits().to_le_bytes();
                let cpu_ptr: *const BxCpuC<I> = self as *const BxCpuC<I>;
                let cpu_ref: &BxCpuC<I> = unsafe { &*cpu_ptr };
                // A/D bit update on page table entry — write to physical RAM cannot meaningfully fail
                let _ = mem.write_physical_page(
                    &[cpu_ref],
                    _page_write_stamp_table,
                    entry_addr[level],
                    8,
                    &mut data.clone(),
                );
            }
        }
        let leaf_needed = PteBits::ACCESSED | if is_write { PteBits::DIRTY } else { PteBits::empty() };
        if !entry[leaf].contains(leaf_needed) {
            entry[leaf].insert(leaf_needed);
            let data = entry[leaf].bits().to_le_bytes();
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

        if (pde & pte_bits32::PRESENT) == 0 {
            tracing::debug!(
                "system_write page walk: PDE not present at {:#x}, laddr={:#x}",
                pde_addr,
                laddr
            );
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, false, true)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check for 4MB page (PSE)
        if (pde & pte_bits32::PS) != 0 && self.cr4.pse() {
            // Bochs paging.cc: check reserved bits in PSE PDE
            if (pde & PAGING_PDE4M_RESERVED_BITS) != 0 {
                tracing::debug!(
                    "system_write PSE PDE4M: reserved bit set: PDE={:#010x}",
                    pde
                );
                self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, false, true)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
            // Set Accessed + Dirty bits on PDE for 4MB page
            let needed = pte_bits32::ACCESSED | pte_bits32::DIRTY;
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

        if (pte & pte_bits32::PRESENT) == 0 {
            tracing::debug!(
                "system_write page walk: PTE not present at {:#x}, laddr={:#x}",
                pte_addr,
                laddr
            );
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, false, true)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Set Accessed bit on PDE if needed
        if pde & pte_bits32::ACCESSED == 0 {
            self.page_walk_write_dword(pde_addr, pde | pte_bits32::ACCESSED);
        }

        // Set Accessed + Dirty bits on PTE for write
        let pte_needed = pte_bits32::ACCESSED | pte_bits32::DIRTY;
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
        let pdpte = PteBits::from_raw(self.pdptrcache.entry[pdpte_index]);
        if !pdpte.contains(PteBits::PRESENT) {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, false, true)?;
            return Err(super::CpuError::CpuLoopRestart);
        }
        let mut ppf = pdpte.bits() & 0x000F_FFFF_FFFF_F000;

        // PDE
        let pde_addr = ppf + (((laddr >> 21) & 0x1FF) << 3);
        let mut pde = PteBits::from_raw(self.page_walk_read_qword(pde_addr));
        if !pde.contains(PteBits::PRESENT) {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, false, true)?;
            return Err(super::CpuError::CpuLoopRestart);
        }
        ppf = pde.bits() & 0x000F_FFFF_FFFF_F000;

        // 2MB page
        if pde.contains(PteBits::PS) {
            if pde.bits() & PAGING_PAE_PDE2M_RESERVED_BITS != 0 {
                self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, false, true)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
            let needed = PteBits::ACCESSED | PteBits::DIRTY;
            if !pde.contains(needed) {
                pde.insert(needed);
                self.page_walk_write_qword(pde_addr, pde.bits());
            }
            ppf = pde.bits() & 0x000F_FFFF_FFE0_0000;
            return Ok((ppf | (laddr & 0x1FFFFF)) & self.a20_mask);
        }

        // PTE
        let pte_addr = ppf + (((laddr >> 12) & 0x1FF) << 3);
        let mut pte = PteBits::from_raw(self.page_walk_read_qword(pte_addr));
        if !pte.contains(PteBits::PRESENT) {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, false, true)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // A bit on PDE
        if !pde.contains(PteBits::ACCESSED) {
            pde.insert(PteBits::ACCESSED);
            self.page_walk_write_qword(pde_addr, pde.bits());
        }
        // A+D on PTE
        let pte_needed = PteBits::ACCESSED | PteBits::DIRTY;
        if !pte.contains(pte_needed) {
            pte.insert(pte_needed);
            self.page_walk_write_qword(pte_addr, pte.bits());
        }

        ppf = pte.bits() & 0x000F_FFFF_FFFF_F000;
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
        let mut entry = [PteBits::empty(); 5];
        let mut leaf = start_leaf;

        loop {
            entry_addr[leaf] = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0xFF8);
            entry[leaf] = PteBits::from_raw(self.page_walk_read_qword(entry_addr[leaf]));

            offset_mask >>= 9;

            if !entry[leaf].contains(PteBits::PRESENT) {
                // Deliver #PF so double-fault detection works during exception delivery
                self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, false, true)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            ppf = entry[leaf].bits() & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            if entry[leaf].contains(PteBits::PS) {
                ppf &= 0x000F_FFFF_FFFF_E000;
                if ppf & offset_mask != 0 {
                    self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, false, true)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
                break;
            }

            leaf -= 1;
        }

        // A/D bits — non-leaf get A, leaf gets A+D
        for level in (leaf + 1..=start_leaf).rev() {
            if !entry[level].contains(PteBits::ACCESSED) {
                entry[level].insert(PteBits::ACCESSED);
                self.page_walk_write_qword(entry_addr[level], entry[level].bits());
            }
        }
        let leaf_needed = PteBits::ACCESSED | PteBits::DIRTY;
        if !entry[leaf].contains(leaf_needed) {
            entry[leaf].insert(leaf_needed);
            self.page_walk_write_qword(entry_addr[leaf], entry[leaf].bits());
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

        if (pde & pte_bits32::PRESENT) == 0 {
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
        if (pde & pte_bits32::PS) != 0 && self.cr4.pse() {
            let ppf_4m = (pde & 0xFFC00000) as u64;
            let offset = laddr & 0x3FFFFF;
            return Ok((ppf_4m | offset) & self.a20_mask);
        }

        // Read PTE
        let pt_base = (pde & 0xFFFFF000) as u64;
        let pte_index = ((laddr >> 12) & 0x3FF) as u32;
        let pte_addr = pt_base + (pte_index * 4) as u64;
        let pte = self.page_walk_read_dword_ro(pte_addr);

        if (pte & pte_bits32::PRESENT) == 0 {
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
        let pdpte = PteBits::from_raw(self.pdptrcache.entry[pdpte_index]);
        if !pdpte.contains(PteBits::PRESENT) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }
        let ppf = pdpte.bits() & 0x000F_FFFF_FFFF_F000;

        let pde_addr = ppf + (((laddr >> 21) & 0x1FF) << 3);
        let pde = PteBits::from_raw(self.page_walk_read_qword(pde_addr));
        if !pde.contains(PteBits::PRESENT) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        if pde.contains(PteBits::PS) {
            let ppf_2m = pde.bits() & 0x000F_FFFF_FFE0_0000;
            return Ok((ppf_2m | (laddr & 0x1FFFFF)) & self.a20_mask);
        }

        let ppf = pde.bits() & 0x000F_FFFF_FFFF_F000;
        let pte_addr = ppf + (((laddr >> 12) & 0x1FF) << 3);
        let pte = PteBits::from_raw(self.page_walk_read_qword(pte_addr));
        if !pte.contains(PteBits::PRESENT) {
            return Err(super::CpuError::Memory(
                crate::memory::MemoryError::PageNotPresent,
            ));
        }

        let page_base = pte.bits() & 0x000F_FFFF_FFFF_F000;
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
            let entry_addr = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0xFF8);
            let entry = PteBits::from_raw(self.page_walk_read_qword(entry_addr));

            offset_mask >>= 9;

            if !entry.contains(PteBits::PRESENT) {
                return Err(super::CpuError::Memory(
                    crate::memory::MemoryError::PageNotPresent,
                ));
            }

            ppf = entry.bits() & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            if entry.contains(PteBits::PS) {
                ppf &= 0x000F_FFFF_FFFF_E000;
                break;
            }

            leaf -= 1;
        }

        let paddr = ppf | (laddr & offset_mask);
        Ok(paddr & self.a20_mask)
    }

    /// Diagnostic-only: translate a linear address to physical without raising exceptions.
    /// Returns None if translation fails.
    pub(super) fn translate_linear_for_diag(&self, laddr: u64) -> Option<u64> {
        if !self.cr0.pg() {
            return Some(laddr & self.a20_mask);
        }
        self.translate_linear_system_read_long_mode(laddr)
            .ok()
            .or_else(|| self.translate_linear_system_read(laddr).ok())
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
        let lpf = laddr & LPF_MASK; // linear page frame

        // ---- DTLB lookup ----
        // Compute which access bit we need:
        //   read  + supervisor(0) → bit 0 (TlbAccess::SYS_READ_OK.bits())
        //   read  + user(1)       → bit 1 (TlbAccess::USER_READ_OK.bits())
        //   write + supervisor(0) → bit 2 (TlbAccess::SYS_WRITE_OK.bits())
        //   write + user(1)       → bit 3 (TlbAccess::USER_WRITE_OK.bits())
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
            (TlbAccess::SYS_READ_OK.bits(), 0u32, 0u32),
            (TlbAccess::USER_READ_OK.bits(), 1, 0),
            (TlbAccess::SYS_WRITE_OK.bits(), 0, 1),
            (TlbAccess::USER_WRITE_OK.bits(), 1, 1),
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
            access_bits &= !(TlbAccess::SYS_WRITE_OK.bits() | TlbAccess::USER_WRITE_OK.bits());
        }

        let ppf = paddr & LPF_MASK;

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

        // DIAGNOSTIC: log when page walk resolves to interesting physical pages
        if ppf >= 0x1436000 && ppf < 0x1437000 && self.icount > 13_000_000 {
            eprintln!("[TLB-POPULATE-1436] laddr={:#x} ppf={:#x} paddr={:#x} user={} is_write={} icount={} rip={:#x}",
                laddr, ppf, paddr, user, is_write, self.icount, self.prev_rip);
        }

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
    fn page_walk_write_qword(&mut self, paddr: u64, val: u64) {
        let a20_addr = (paddr & self.a20_mask) as usize;
        let host_base = self.mem_host_base;
        if !host_base.is_null() && a20_addr + 8 <= self.mem_host_len {
            unsafe { (host_base.add(a20_addr) as *mut u64).write_unaligned(val) };
            return;
        }
        self.mem_write_qword(paddr, val);
    }

    /// DIAGNOSTIC: public wrapper for page_walk_read_qword (read-only PTE read)
    pub(super) fn page_walk_read_qword_diag(&self, paddr: u64) -> u64 {
        self.page_walk_read_qword(paddr)
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

    /// DIAGNOSTIC: Read-only 4-level page walk that does NOT modify PTEs or TLB.
    /// Returns the physical address for the given linear address, or None if not present.
    /// Used to verify TLB entries against actual page table state.
    pub(super) fn diag_verify_laddr_ro(&self, laddr: u64) -> Option<u64> {
        if !self.long_mode() { return None; } // only long mode for now
        let cr3 = self.cr3;
        // PML4E
        let pml4e_addr = (cr3 & 0x000F_FFFF_FFFF_F000) | (((laddr >> 39) & 0x1FF) << 3);
        let pml4e = self.page_walk_read_qword(pml4e_addr);
        if pml4e & 1 == 0 { return None; }
        // PDPE
        let pdpe_addr = (pml4e & 0x000F_FFFF_FFFF_F000) | (((laddr >> 30) & 0x1FF) << 3);
        let pdpe = self.page_walk_read_qword(pdpe_addr);
        if pdpe & 1 == 0 { return None; }
        if pdpe & 0x80 != 0 { // 1GB page
            return Some((pdpe & 0x000F_FFFF_C000_0000) | (laddr & 0x3FFF_FFFF));
        }
        // PDE
        let pde_addr = (pdpe & 0x000F_FFFF_FFFF_F000) | (((laddr >> 21) & 0x1FF) << 3);
        let pde = self.page_walk_read_qword(pde_addr);
        if pde & 1 == 0 { return None; }
        if pde & 0x80 != 0 { // 2MB page
            return Some((pde & 0x000F_FFFF_FFE0_0000) | (laddr & 0x001F_FFFF));
        }
        // PTE
        let pte_addr = (pde & 0x000F_FFFF_FFFF_F000) | (((laddr >> 12) & 0x1FF) << 3);
        let pte = self.page_walk_read_qword(pte_addr);
        if pte & 1 == 0 { return None; }
        Some((pte & 0x000F_FFFF_FFFF_F000) | (laddr & 0xFFF))
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

        if pde & pte_bits32::PRESENT == 0 {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // ---- 4 MB page (PSE) ----
        if pde & pte_bits32::PS != 0 && self.cr4.pse() {
            // Bochs paging.cc: check reserved bits in PSE PDE
            if (pde & PAGING_PDE4M_RESERVED_BITS) != 0 {
                tracing::debug!("PSE PDE4M: reserved bit is set: PDE={:#010x}", pde);
                self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
            let combined = pde & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());
            let priv_index =
                ((self.cr0.wp() as u32) << 4) | ((user as u32) << 3) | combined | (is_write as u32);
            if PRIV_CHECK[priv_index as usize] == 0 {
                self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
            // Set A/D bits on the PDE.
            let needed = pte_bits32::ACCESSED | if is_write { pte_bits32::DIRTY } else { 0 };
            if pde & needed != needed {
                self.page_walk_write_dword(pde_addr, pde | needed);
            }
            let paddr = (pde as u64 & 0xFFC0_0000) | (laddr & 0x003F_FFFF);
            return Ok((paddr, combined, 0x3F_FFFF)); // 4MB lpf_mask
        }

        // ---- PTE ----
        let pte_addr = (pde as u64 & 0xFFFF_F000) | (((laddr >> 12) & 0x3FF) << 2);
        let pte = self.page_walk_read_dword(pte_addr);

        if pte & pte_bits32::PRESENT == 0 {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        let combined = (pde & pte) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());
        let priv_index =
            ((self.cr0.wp() as u32) << 4) | ((user as u32) << 3) | combined | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 {
            self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Set A bit on PDE if needed.
        if pde & pte_bits32::ACCESSED == 0 {
            self.page_walk_write_dword(pde_addr, pde | pte_bits32::ACCESSED);
        }
        // Set A/D bits on PTE.
        let pte_needed = pte_bits32::ACCESSED | if is_write { pte_bits32::DIRTY } else { 0 };
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
        let mut combined_access = CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits();
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
        let pdpte = PteBits::from_raw(self.pdptrcache.entry[pdpte_index]);

        if !pdpte.contains(PteBits::PRESENT) {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // PDPTE reserved bit check is done at CR3 load time (CheckPDPTR),
        // but also verify here for safety.
        if pdpte.bits() & PAGING_PAE_PDPTE_RESERVED_BITS != 0 {
            tracing::debug!("PAE PDPTE: reserved bit set: {:#018x}", pdpte.bits());
            self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        let mut ppf = pdpte.bits() & 0x000F_FFFF_FFFF_F000;

        // ---- PDE ----
        let mut entry_addr = [0u64; 2];
        let mut entry = [PteBits::empty(); 2];

        // Bochs: ppf + ((laddr >> (9 + 9*1)) & 0xFF8) — extracts laddr bits 29:21 as byte offset
        entry_addr[BX_LEVEL_PDE] = ppf + ((laddr >> 18) & 0xFF8);
        entry[BX_LEVEL_PDE] = PteBits::from_raw(self.page_walk_read_qword(entry_addr[BX_LEVEL_PDE]));

        // Check present
        if !entry[BX_LEVEL_PDE].contains(PteBits::PRESENT) {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check reserved bits
        if entry[BX_LEVEL_PDE].bits() & reserved != 0 {
            tracing::debug!(
                "PAE PDE: reserved bit set: {:#018x} (reserved mask: {:#018x})",
                entry[BX_LEVEL_PDE].bits(),
                entry[BX_LEVEL_PDE].bits() & reserved
            );
            self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check NX bit
        if entry[BX_LEVEL_PDE].bits() & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        ppf = entry[BX_LEVEL_PDE].bits() & 0x000F_FFFF_FFFF_F000;

        // ---- 2MB large page (PDE.PS=1) ----
        // In PAE mode, CR4.PSE is ignored — PS bit in PDE is always checked.
        if entry[BX_LEVEL_PDE].contains(PteBits::PS) {
            // Check 2MB PDE reserved bits (bits 20:13 must be zero)
            if entry[BX_LEVEL_PDE].bits() & PAGING_PAE_PDE2M_RESERVED_BITS != 0 {
                tracing::debug!("PAE PDE2M: reserved bit set: {:#018x}", entry[BX_LEVEL_PDE].bits());
                self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // Physical page frame for 2MB page
            ppf = entry[BX_LEVEL_PDE].bits() & 0x000F_FFFF_FFE0_0000;

            // Leaf entry permission check
            combined_access &=
                (entry[BX_LEVEL_PDE].bits() as u32) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());

            let priv_index = ((self.cr0.wp() as u32) << 4)
                | ((user as u32) << 3)
                | combined_access
                | (is_write as u32);
            if PRIV_CHECK[priv_index as usize] == 0 {
                self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // SMAP check for 2MB page
            if self.cr4.smap() && !user && (combined_access & CombinedAccess::USER.bits()) != 0 {
                if self.get_ac() == 0 {
                    self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
            }

            // Update A/D bits on PDE (leaf for 2MB page)
            let needed = PteBits::ACCESSED | if is_write { PteBits::DIRTY } else { PteBits::empty() };
            if !entry[BX_LEVEL_PDE].contains(needed) {
                entry[BX_LEVEL_PDE].insert(needed);
                self.page_walk_write_qword(entry_addr[BX_LEVEL_PDE], entry[BX_LEVEL_PDE].bits());
            }

            let paddr = ppf | (laddr & 0x001F_FFFF);
            return Ok((paddr, combined_access, 0x1F_FFFF)); // 2MB lpf_mask
        }

        combined_access &= entry[BX_LEVEL_PDE].bits() as u32; // U/S and R/W from PDE

        // ---- PTE ----
        entry_addr[BX_LEVEL_PTE] = ppf + (((laddr >> 12) & 0x1FF) << 3);
        entry[BX_LEVEL_PTE] = PteBits::from_raw(self.page_walk_read_qword(entry_addr[BX_LEVEL_PTE]));

        // Check present
        if !entry[BX_LEVEL_PTE].contains(PteBits::PRESENT) {
            self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check reserved bits
        if entry[BX_LEVEL_PTE].bits() & reserved != 0 {
            tracing::debug!("PAE PTE: reserved bit set: {:#018x}", entry[BX_LEVEL_PTE].bits());
            self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // Check NX on PTE
        if entry[BX_LEVEL_PTE].bits() & PAGE_DIRECTORY_NX_BIT != 0 {
            nx_page = true;
        }

        // Leaf permission check
        combined_access &=
            (entry[BX_LEVEL_PTE].bits() as u32) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());

        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | combined_access
            | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 {
            self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // SMAP check: supervisor data access to user page when AC=0
        if self.cr4.smap() && !user && (combined_access & CombinedAccess::USER.bits()) != 0 {
            if self.get_ac() == 0 {
                self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
        }

        // Update A/D bits — PDE gets A bit, PTE gets A+D
        if !entry[BX_LEVEL_PDE].contains(PteBits::ACCESSED) {
            entry[BX_LEVEL_PDE].insert(PteBits::ACCESSED);
            self.page_walk_write_qword(entry_addr[BX_LEVEL_PDE], entry[BX_LEVEL_PDE].bits());
        }
        let pte_needed = PteBits::ACCESSED | if is_write { PteBits::DIRTY } else { PteBits::empty() };
        if !entry[BX_LEVEL_PTE].contains(pte_needed) {
            entry[BX_LEVEL_PTE].insert(pte_needed);
            self.page_walk_write_qword(entry_addr[BX_LEVEL_PTE], entry[BX_LEVEL_PTE].bits());
        }

        ppf = entry[BX_LEVEL_PTE].bits() & 0x000F_FFFF_FFFF_F000;
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
        let mut combined_access = CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits();
        let mut nx_page = false;

        // Reserved bits: in long mode, bits [62:52] are ignored (NOT reserved).
        // Only PHY_RESERVED bits are reserved. If NXE not enabled, bit 63 is reserved.
        let mut reserved = PAGING_PAE_PHY_RESERVED_BITS;
        if !self.efer.nxe() {
            reserved |= PAGE_DIRECTORY_NX_BIT;
        }

        let mut ppf = self.cr3 & BX_CR3_PAGING_MASK_PAE;
        let mut entry_addr = [0u64; 5];
        let mut entry = [PteBits::empty(); 5];

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
            entry_addr[leaf] = ppf + ((laddr >> (9 + 9 * leaf as u64)) & 0xFF8);
            entry[leaf] = PteBits::from_raw(self.page_walk_read_qword(entry_addr[leaf]));

            offset_mask >>= 9;

            let curr_entry = entry[leaf];

            // Check present
            if !curr_entry.contains(PteBits::PRESENT) {
                self.page_fault(PageFaultError::NOT_PRESENT.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // Check reserved bits
            if curr_entry.bits() & reserved != 0 {
                self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }

            // PS bit at invalid level (only PDE level 1 and PDPTE level 2 with 1G support)
            if curr_entry.contains(PteBits::PS) {
                // PS bit set — valid only at BX_LEVEL_PDE (2MB) and BX_LEVEL_PDPTE (1GB)
                if leaf > BX_LEVEL_PDPTE {
                    // PS at PML4 or PML5 level is reserved
                    self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
            }

            // Check NX
            if curr_entry.bits() & PAGE_DIRECTORY_NX_BIT != 0 {
                nx_page = true;
            }

            ppf = curr_entry.bits() & 0x000F_FFFF_FFFF_F000;

            if leaf == BX_LEVEL_PTE {
                break;
            }

            // Large page?
            if curr_entry.contains(PteBits::PS) {
                ppf &= 0x000F_FFFF_FFFF_E000; // clear bit 12 for large pages
                if ppf & offset_mask != 0 {
                    self.page_fault(PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                    return Err(super::CpuError::CpuLoopRestart);
                }
                lpf_mask = offset_mask as u32;
                break;
            }

            combined_access &= curr_entry.bits() as u32; // Accumulate U/S and R/W from non-leaf entries
            leaf -= 1;
        }

        // Leaf entry permission check
        combined_access &=
            (entry[leaf].bits() as u32) & (CombinedAccess::WRITE.bits() | CombinedAccess::USER.bits());

        let priv_index = ((self.cr0.wp() as u32) << 4)
            | ((user as u32) << 3)
            | combined_access
            | (is_write as u32);
        if PRIV_CHECK[priv_index as usize] == 0 {
            self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
            return Err(super::CpuError::CpuLoopRestart);
        }

        // SMEP check: not applicable for data access (handled by translate_linear for execute)

        // SMAP check: supervisor data access to user page when AC=0
        // Bochs paging.cc:740-749
        if self.cr4.smap() && !user && (combined_access & CombinedAccess::USER.bits()) != 0 {
            if self.get_ac() == 0 {
                self.page_fault(PageFaultError::PROTECTION.bits(), laddr, user, is_write)?;
                return Err(super::CpuError::CpuLoopRestart);
            }
        }

        // Update A/D bits for all levels
        // Non-leaf levels get A bit, leaf gets A+D
        for level in (leaf + 1..=start_leaf).rev() {
            if !entry[level].contains(PteBits::ACCESSED) {
                entry[level].insert(PteBits::ACCESSED);
                self.page_walk_write_qword(entry_addr[level], entry[level].bits());
            }
        }
        let leaf_needed = PteBits::ACCESSED | if is_write { PteBits::DIRTY } else { PteBits::empty() };
        if !entry[leaf].contains(leaf_needed) {
            entry[leaf].insert(leaf_needed);
            self.page_walk_write_qword(entry_addr[leaf], entry[leaf].bits());
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
