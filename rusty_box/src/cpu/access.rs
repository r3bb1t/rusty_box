// access.rs — Virtual memory access functions
//
// Ported from Bochs cpu/access.cc + cpu/access2.cc
//
// This module implements the full memory access pipeline:
//   1. Segment validation (type, present, limit, expand-down)
//   2. Linear address computation (segment base + offset)
//   3. Paging translation (TLB + page walk)
//   4. Physical memory read/write
//
// Includes cross-page boundary handling for multi-byte accesses.

use super::cpu::Exception;
use super::decoder::BxSegregs;
use super::descriptor::{
    SEG_ACCESS_ROK, SEG_ACCESS_ROK4_G, SEG_ACCESS_WOK, SEG_ACCESS_WOK4_G, SEG_VALID_CACHE,
};
use super::rusty_box::MemoryAccessType;
use super::{BxCpuC, BxCpuIdTrait, Result};
use crate::config::{BxAddress, BxPtrEquiv};

/// BX_MAX_MEM_ACCESS_LENGTH from Bochs — maximum access size for
/// segment limit checks.  Matches the largest scalar access (qword=8).
const BX_MAX_MEM_ACCESS_LENGTH: u32 = 8;

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    // ===== Canonical address check (Bochs access.cc IsCanonicalAccess) =====

    pub(super) fn is_canonical_access(
        &self,
        laddr: BxAddress,
        rw: MemoryAccessType,
        user: bool,
    ) -> bool {
        if !self.is_canonical(laddr) {
            return false;
        }

        if self.long64_mode() && self.cr4.lass() {
            let access_user_space = (laddr >> 63) == 0;

            if user {
                if !access_user_space {
                    return false;
                }
                return true;
            }

            if (rw == MemoryAccessType::Execute || (self.cr4.smap() && self.get_ac() == 0))
                && access_user_space
            {
                return false;
            }
        }

        true
    }

    // ===== Exception selector: #SS for SS, #GP for others (Bochs int_number) =====

    #[inline]
    fn seg_exception(seg: BxSegregs) -> Exception {
        if matches!(seg, BxSegregs::Ss) {
            Exception::Ss
        } else {
            Exception::Gp
        }
    }

    // ===== Segment validation checks (Bochs access.cc) =====

    /// Validate a segment for write access.
    /// Returns true if the access is permitted, false if a segment fault should
    /// be raised.  On success, may set SegAccessWOK / SegAccessWOK4G in the
    /// descriptor cache for future fast-path use.
    ///
    /// Bochs: write_virtual_checks (access.cc)
    fn write_virtual_checks(&mut self, seg_idx: usize, offset: u32, length: u32) -> bool {
        let seg = &self.sregs[seg_idx];
        let cache = &seg.cache;

        let length = length - 1; // convert to zero-based for compare

        // Segment must be valid and present
        if (cache.valid & SEG_VALID_CACHE) == 0 || !cache.p {
            return false;
        }

        let seg_type = cache.r#type;

        // Must be a data/code segment (segment bit set)
        if !cache.segment {
            return false;
        }

        // Check type — only types 2,3,6,7 (read/write data) are writable
        // Bit 3 = code segment, bit 1 = writable/readable
        if (seg_type & 0x08) != 0 {
            // Code segment — never writable
            return false;
        }
        if (seg_type & 0x02) == 0 {
            // Data segment without write bit — read-only
            return false;
        }

        let limit_scaled = unsafe { cache.u.segment.limit_scaled };
        let base = unsafe { cache.u.segment.base };

        if (seg_type & 0x04) == 0 {
            // Normal data segment (expand-up, types 2,3)
            if limit_scaled == 0xFFFFFFFF && base == 0 {
                // Flat 4GB segment — cache fast-path flags
                self.sregs[seg_idx].cache.valid |=
                    SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
                return true;
            }
            if offset > limit_scaled.wrapping_sub(length) || length > limit_scaled {
                return false;
            }
            if limit_scaled >= (BX_MAX_MEM_ACCESS_LENGTH - 1) {
                self.sregs[seg_idx].cache.valid |= SEG_ACCESS_ROK | SEG_ACCESS_WOK;
            }
        } else {
            // Expand-down data segment (types 6,7)
            let d_b = unsafe { cache.u.segment.d_b };
            let upper_limit: u32 = if d_b { 0xFFFFFFFF } else { 0x0000FFFF };
            if offset <= limit_scaled || offset > upper_limit || (upper_limit - offset) < length {
                return false;
            }
        }

        true
    }

    /// Validate a segment for read access.
    /// Returns true if the access is permitted.
    ///
    /// Bochs: read_virtual_checks (access.cc)
    fn read_virtual_checks(&mut self, seg_idx: usize, offset: u32, length: u32) -> bool {
        let seg = &self.sregs[seg_idx];
        let cache = &seg.cache;

        let length = length - 1;

        if (cache.valid & SEG_VALID_CACHE) == 0 || !cache.p {
            return false;
        }

        let seg_type = cache.r#type;

        if !cache.segment {
            return false;
        }

        // Types 8,9,12,13 are execute-only (no read) => reject
        if (seg_type & 0x08) != 0 && (seg_type & 0x02) == 0 {
            return false;
        }

        let limit_scaled = unsafe { cache.u.segment.limit_scaled };
        let base = unsafe { cache.u.segment.base };

        // Expand-down segments (types 4,5,6,7)
        if (seg_type & 0x08) == 0 && (seg_type & 0x04) != 0 {
            let d_b = unsafe { cache.u.segment.d_b };
            let upper_limit: u32 = if d_b { 0xFFFFFFFF } else { 0x0000FFFF };
            if offset <= limit_scaled || offset > upper_limit || (upper_limit - offset) < length {
                return false;
            }
            return true;
        }

        // Normal (expand-up) data or readable code segment
        if limit_scaled == 0xFFFFFFFF && base == 0 {
            self.sregs[seg_idx].cache.valid |=
                SEG_ACCESS_ROK | SEG_ACCESS_WOK | SEG_ACCESS_ROK4_G | SEG_ACCESS_WOK4_G;
            return true;
        }
        if offset > limit_scaled.wrapping_sub(length) || length > limit_scaled {
            return false;
        }
        if limit_scaled >= (BX_MAX_MEM_ACCESS_LENGTH - 1) {
            self.sregs[seg_idx].cache.valid |= SEG_ACCESS_ROK | SEG_ACCESS_WOK;
        }

        true
    }

    // ===== Address generation (Bochs agen_read32 / agen_write32) =====

    /// Compute linear address for a read access with full segment validation.
    /// Bochs: agen_read32
    #[inline]
    pub(super) fn agen_read32(&mut self, seg: BxSegregs, offset: u32, len: u32) -> Result<u32> {
        let seg_idx = seg as usize;

        // Fast path: flat 4GB readable segment
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_ROK4_G) != 0 {
            return Ok(offset);
        }

        // Medium path: within cached limit
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_ROK) != 0 {
            let limit = unsafe { self.sregs[seg_idx].cache.u.segment.limit_scaled };
            if offset <= limit.wrapping_sub(len.wrapping_sub(1)) {
                return Ok(self.get_laddr32(seg_idx, offset));
            }
        }

        // Slow path: full segment checks
        if !self.read_virtual_checks(seg_idx, offset, len) {
            self.exception(Self::seg_exception(seg), 0)?;
        }
        Ok(self.get_laddr32(seg_idx, offset))
    }

    /// Compute linear address for a write access with full segment validation.
    /// Bochs: agen_write32
    #[inline]
    pub(super) fn agen_write32(&mut self, seg: BxSegregs, offset: u32, len: u32) -> Result<u32> {
        let seg_idx = seg as usize;

        // Fast path: flat 4GB writable segment
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_WOK4_G) != 0 {
            return Ok(offset);
        }

        // Medium path: within cached limit
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_WOK) != 0 {
            let limit = unsafe { self.sregs[seg_idx].cache.u.segment.limit_scaled };
            if offset <= limit.wrapping_sub(len.wrapping_sub(1)) {
                return Ok(self.get_laddr32(seg_idx, offset));
            }
        }

        // Slow path: full segment checks
        if !self.write_virtual_checks(seg_idx, offset, len) {
            self.exception(Self::seg_exception(seg), 0)?;
        }
        Ok(self.get_laddr32(seg_idx, offset))
    }

    // ===== Virtual read functions (Bochs access.h + access2.cc) =====
    //
    // Performance-critical: these are called on every memory-accessing instruction.
    // Inline TLB lookup with host pointer avoids calling translate_data_read() +
    // mem_read_byte() (which goes through get_host_mem_addr()) on TLB hits.

    /// Read a byte from virtual memory.
    /// Bochs: read_virtual_byte_32 -> agen_read32 + read_linear_byte
    #[inline]
    pub fn read_virtual_byte(&mut self, seg: BxSegregs, offset: u32) -> Result<u8> {
        let laddr = self.agen_read32(seg, offset, 1)? as u64;

        // ---- Inline TLB fast path (Bochs access2.cc pattern) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (self.user_pl as u32); // TLB_SYS_READ_OK or TLB_USER_READ_OK
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let host = tlb.host_page_addr as *const u8;
                return Ok(unsafe { *host.add((laddr & 0xFFF) as usize) });
            }
        }

        // ---- Slow path ----
        let paddr = self.translate_data_read(laddr)?;
        Ok(self.mem_read_byte(paddr))
    }

    /// Read a word from virtual memory with cross-page handling.
    /// Bochs: read_virtual_word_32 -> agen_read32 + read_linear_word
    #[inline]
    pub fn read_virtual_word(&mut self, seg: BxSegregs, offset: u32) -> Result<u16> {
        let laddr = self.agen_read32(seg, offset, 2)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 1)) ----
        // The len=1 trick indexes TLB by the LAST byte. If access crosses a page
        // boundary, laddr+1 is on the next page → different TLB slot → guaranteed miss.
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 1);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as usize;
                let host = tlb.host_page_addr as *const u8;
                let ptr = unsafe { host.add(page_offset) };
                return Ok(unsafe { (ptr as *const u16).read_unaligned() });
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            Ok(self.mem_read_word(paddr))
        } else {
            let b0 = self.read_virtual_byte_at_laddr(laddr)?;
            let b1 = self.read_virtual_byte_at_laddr(
                (laddr & 0xFFFF_F000).wrapping_add(0x1000) & 0xFFFF_FFFF,
            )?;
            Ok(u16::from_le_bytes([b0, b1]))
        }
    }

    /// Read a dword from virtual memory with cross-page handling.
    /// Bochs: read_virtual_dword_32 -> agen_read32 + read_linear_dword
    #[inline]
    pub fn read_virtual_dword(&mut self, seg: BxSegregs, offset: u32) -> Result<u32> {
        let laddr = self.agen_read32(seg, offset, 4)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 3)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 3);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as usize;
                let host = tlb.host_page_addr as *const u8;
                let ptr = unsafe { host.add(page_offset) };
                return Ok(unsafe { (ptr as *const u32).read_unaligned() });
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            Ok(self.mem_read_dword(paddr))
        } else {
            let mut buf = [0u8; 4];
            for i in 0..4u64 {
                buf[i as usize] =
                    self.read_virtual_byte_at_laddr((laddr.wrapping_add(i)) & 0xFFFF_FFFF)?;
            }
            Ok(u32::from_le_bytes(buf))
        }
    }

    /// Read a qword from virtual memory with cross-page handling.
    /// Bochs: read_virtual_qword_32 -> agen_read32 + read_linear_qword
    #[inline]
    pub(crate) fn read_virtual_qword(&mut self, seg: BxSegregs, offset: u32) -> Result<u64> {
        let laddr = self.agen_read32(seg, offset, 8)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 7)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 7);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as usize;
                let host = tlb.host_page_addr as *const u8;
                let ptr = unsafe { host.add(page_offset) };
                return Ok(unsafe { (ptr as *const u64).read_unaligned() });
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            Ok(self.mem_read_qword(paddr))
        } else {
            let mut buf = [0u8; 8];
            for i in 0..8u64 {
                buf[i as usize] =
                    self.read_virtual_byte_at_laddr((laddr.wrapping_add(i)) & 0xFFFF_FFFF)?;
            }
            Ok(u64::from_le_bytes(buf))
        }
    }

    /// Internal helper: read a single byte at a given linear address.
    /// Used by cross-page paths to avoid duplicating TLB fast-path code.
    #[inline]
    fn read_virtual_byte_at_laddr(&mut self, laddr: u64) -> Result<u8> {
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let host = tlb.host_page_addr as *const u8;
                return Ok(unsafe { *host.add((laddr & 0xFFF) as usize) });
            }
        }
        let paddr = self.translate_data_read(laddr)?;
        Ok(self.mem_read_byte(paddr))
    }

    // ===== Virtual write functions (Bochs access.h + access2.cc) =====

    /// Write a byte to virtual memory.
    /// Bochs: write_virtual_byte_32 -> agen_write32 + write_linear_byte
    #[inline]
    pub fn write_virtual_byte(&mut self, seg: BxSegregs, offset: u32, val: u8) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 1)? as u64;

        // ---- Inline TLB fast path ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            // write + user/sys: bit 2 (TLB_SYS_WRITE_OK) or bit 3 (TLB_USER_WRITE_OK)
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let host = tlb.host_page_addr as *mut u8;
                unsafe { *host.add((laddr & 0xFFF) as usize) = val };
                return Ok(());
            }
        }

        let paddr = self.translate_data_write(laddr)?;
        self.mem_write_byte(paddr, val);
        Ok(())
    }

    /// Write a word to virtual memory with cross-page handling.
    /// Bochs: write_virtual_word_32 -> agen_write32 + write_linear_word
    #[inline]
    pub(super) fn write_virtual_word(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        val: u16,
    ) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 2)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 1)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 1);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as usize;
                let host = tlb.host_page_addr as *mut u8;
                let ptr = unsafe { host.add(page_offset) };
                unsafe { (ptr as *mut u16).write_unaligned(val) };
                return Ok(());
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            self.mem_write_word(paddr, val);
        } else {
            let bytes = val.to_le_bytes();
            self.write_virtual_byte_at_laddr(laddr, bytes[0])?;
            let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & 0xFFFF_FFFF;
            self.write_virtual_byte_at_laddr(laddr2, bytes[1])?;
        }
        Ok(())
    }

    /// Write a dword to virtual memory with cross-page handling.
    /// Bochs: write_virtual_dword_32 -> agen_write32 + write_linear_dword
    #[inline]
    pub(super) fn write_virtual_dword(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        val: u32,
    ) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 4)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 3)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 3);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as usize;
                let host = tlb.host_page_addr as *mut u8;
                let ptr = unsafe { host.add(page_offset) };
                unsafe { (ptr as *mut u32).write_unaligned(val) };
                return Ok(());
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            self.mem_write_dword(paddr, val);
        } else {
            let bytes = val.to_le_bytes();
            for i in 0..4u64 {
                let la = (laddr.wrapping_add(i)) & 0xFFFF_FFFF;
                self.write_virtual_byte_at_laddr(la, bytes[i as usize])?;
            }
        }
        Ok(())
    }

    /// Write a qword to virtual memory with cross-page handling.
    /// Bochs: write_virtual_qword_32 -> agen_write32 + write_linear_qword
    #[inline]
    pub(crate) fn write_virtual_qword(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        val: u64,
    ) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 8)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 7)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 7);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as usize;
                let host = tlb.host_page_addr as *mut u8;
                let ptr = unsafe { host.add(page_offset) };
                unsafe { (ptr as *mut u64).write_unaligned(val) };
                return Ok(());
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            self.mem_write_qword(paddr, val);
        } else {
            let bytes = val.to_le_bytes();
            for i in 0..8u64 {
                let la = (laddr.wrapping_add(i)) & 0xFFFF_FFFF;
                self.write_virtual_byte_at_laddr(la, bytes[i as usize])?;
            }
        }
        Ok(())
    }

    /// Read a 128-bit XMM word from virtual memory.
    /// Implemented as two qword reads (low then high).
    /// Bochs: read_virtual_xmmword_32
    pub(super) fn read_virtual_xmmword(
        &mut self,
        seg: super::decoder::BxSegregs,
        offset: u32,
    ) -> Result<super::xmm::BxPackedXmmRegister> {
        let lo = self.read_virtual_qword(seg, offset)?;
        let hi = self.read_virtual_qword(seg, offset.wrapping_add(8))?;
        Ok(super::xmm::BxPackedXmmRegister { xmm64u: [lo, hi] })
    }

    /// Read a 128-bit XMM word with 16-byte alignment check.
    /// Raises #GP(0) if address is not 16-byte aligned.
    /// Bochs: read_virtual_xmmword_aligned_32
    pub(super) fn read_virtual_xmmword_aligned(
        &mut self,
        seg: super::decoder::BxSegregs,
        offset: u32,
    ) -> Result<super::xmm::BxPackedXmmRegister> {
        if (offset & 0xF) != 0 {
            self.exception(super::cpu::Exception::Gp, 0)?;
        }
        self.read_virtual_xmmword(seg, offset)
    }

    /// Write a 128-bit XMM word to virtual memory.
    /// Implemented as two qword writes (low then high).
    /// Bochs: write_virtual_xmmword_32
    pub(super) fn write_virtual_xmmword(
        &mut self,
        seg: super::decoder::BxSegregs,
        offset: u32,
        val: &super::xmm::BxPackedXmmRegister,
    ) -> Result<()> {
        unsafe {
            self.write_virtual_qword(seg, offset, val.xmm64u[0])?;
            self.write_virtual_qword(seg, offset.wrapping_add(8), val.xmm64u[1])?;
        }
        Ok(())
    }

    /// Write a 128-bit XMM word with 16-byte alignment check.
    /// Raises #GP(0) if address is not 16-byte aligned.
    /// Bochs: write_virtual_xmmword_aligned_32
    pub(super) fn write_virtual_xmmword_aligned(
        &mut self,
        seg: super::decoder::BxSegregs,
        offset: u32,
        val: &super::xmm::BxPackedXmmRegister,
    ) -> Result<()> {
        if (offset & 0xF) != 0 {
            self.exception(super::cpu::Exception::Gp, 0)?;
        }
        self.write_virtual_xmmword(seg, offset, val)
    }

    /// Internal helper: write a single byte at a given linear address.
    /// Used by cross-page paths to avoid duplicating TLB fast-path code.
    #[inline]
    fn write_virtual_byte_at_laddr(&mut self, laddr: u64, val: u8) -> Result<()> {
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let host = tlb.host_page_addr as *mut u8;
                unsafe { *host.add((laddr & 0xFFF) as usize) = val };
                return Ok(());
            }
        }
        let paddr = self.translate_data_write(laddr)?;
        self.mem_write_byte(paddr, val);
        Ok(())
    }

    // ===== Read-Modify-Write virtual functions (Bochs access2.cc) =====
    //
    // These populate `self.address_xlation` for the write-back phase:
    //   pages > 2  →  host pointer stored (direct write-back, fastest)
    //   pages == 1 →  single-page physical address in paddress1
    //   pages == 2 →  cross-page: paddress1/paddress2 + len1/len2

    /// Read phase of a read-modify-write byte access.
    /// Bochs: read_RMW_virtual_byte_32 -> read_RMW_linear_byte (access2.cc:608)
    #[inline]
    pub fn read_rmw_virtual_byte(&mut self, seg: BxSegregs, offset: u32) -> Result<u8> {
        let laddr = self.agen_write32(seg, offset, 1)? as u64;

        // ---- Inline TLB fast path (Bochs access2.cc:613-631) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
                let host_addr = tlb.host_page_addr | page_offset;
                let data = unsafe { *(host_addr as *const u8) };
                // Cache host pointer for write-back (Bochs: pages > 2 = host addr)
                self.address_xlation.pages = host_addr;
                self.address_xlation.paddress1 = tlb.ppf | (laddr & 0xFFF);
                return Ok(data);
            }
        }

        // ---- Slow path ----
        let paddr = self.translate_data_write(laddr)?;
        let data = self.mem_read_byte(paddr);
        self.address_xlation.pages = 1;
        self.address_xlation.paddress1 = paddr;
        Ok(data)
    }

    /// Read phase of a read-modify-write word access.
    /// Bochs: read_RMW_virtual_word_32 -> read_RMW_linear_word (access2.cc:639)
    #[inline]
    pub fn read_rmw_virtual_word(&mut self, seg: BxSegregs, offset: u32) -> Result<u16> {
        let laddr = self.agen_write32(seg, offset, 2)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 1)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 1);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
                let host_addr = tlb.host_page_addr | page_offset;
                let data = unsafe { (host_addr as *const u16).read_unaligned() };
                self.address_xlation.pages = host_addr;
                self.address_xlation.paddress1 = tlb.ppf | (laddr & 0xFFF);
                return Ok(data);
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            let data = self.mem_read_word(paddr);
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = paddr;
            Ok(data)
        } else {
            // Cross-page RMW (Bochs: access_read_linear sets pages=2)
            let p0 = self.translate_data_write(laddr)?;
            let b0 = self.mem_read_byte(p0);
            let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & 0xFFFF_FFFF;
            let p1 = self.translate_data_write(laddr2)?;
            let b1 = self.mem_read_byte(p1);
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = 1;
            self.address_xlation.len2 = 1;
            Ok(u16::from_le_bytes([b0, b1]))
        }
    }

    /// Read phase of a read-modify-write dword access.
    /// Bochs: read_RMW_virtual_dword_32 -> read_RMW_linear_dword (access2.cc:674)
    #[inline]
    pub fn read_rmw_virtual_dword(&mut self, seg: BxSegregs, offset: u32) -> Result<u32> {
        let laddr = self.agen_write32(seg, offset, 4)? as u64;

        // ---- Inline TLB fast path (Bochs: BX_DTLB_ENTRY_OF(laddr, 3)) ----
        if self.cr0.pg() {
            let lpf = laddr & 0xFFFF_F000;
            let needed_bit = 1u32 << (2 + self.user_pl as u32);
            let tlb = self.dtlb.get_entry_of(laddr, 3);
            if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
                let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
                let host_addr = tlb.host_page_addr | page_offset;
                let data = unsafe { (host_addr as *const u32).read_unaligned() };
                self.address_xlation.pages = host_addr;
                self.address_xlation.paddress1 = tlb.ppf | (laddr & 0xFFF);
                return Ok(data);
            }
        }

        // ---- Slow path: check cross-page ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            let data = self.mem_read_dword(paddr);
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = paddr;
            Ok(data)
        } else {
            // Cross-page RMW
            let len1 = (0x1000 - page_offset) as u32;
            let len2 = 4 - len1;
            let p0 = self.translate_data_write(laddr)?;
            let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & 0xFFFF_FFFF;
            let p1 = self.translate_data_write(laddr2)?;
            let mut buf = [0u8; 4];
            for i in 0..len1 as usize {
                buf[i] = self.mem_read_byte(p0 + i as u64);
            }
            for i in 0..len2 as usize {
                buf[len1 as usize + i] = self.mem_read_byte(p1 + i as u64);
            }
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = len1;
            self.address_xlation.len2 = len2;
            Ok(u32::from_le_bytes(buf))
        }
    }

    // ===== System read/write functions (Bochs access.cc) =====
    //
    // These bypass segment checks and operate on raw linear addresses at
    // CPL=0 (supervisor).  They still go through paging translation.

    /// Read a byte from a system (linear) address.
    /// Bochs: system_read_byte (access.cc)
    pub(super) fn system_read_byte(&self, laddr: BxAddress) -> Result<u8> {
        let paddr = self.translate_linear_system_read(laddr)?;
        Ok(self.mem_read_byte(paddr))
    }

    /// Read a word from a system (linear) address with cross-page handling.
    /// Bochs: system_read_word (access.cc)
    pub(super) fn system_read_word(&self, laddr: BxAddress) -> Result<u16> {
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_linear_system_read(laddr)?;
            Ok(self.mem_read_word(paddr))
        } else {
            let b0 = {
                let p = self.translate_linear_system_read(laddr)?;
                self.mem_read_byte(p)
            };
            let b1 = {
                let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & 0xFFFF_FFFF;
                let p = self.translate_linear_system_read(laddr2)?;
                self.mem_read_byte(p)
            };
            Ok(u16::from_le_bytes([b0, b1]))
        }
    }

    /// Read a dword from a system (linear) address with cross-page handling.
    /// Bochs: system_read_dword (access.cc)
    pub(super) fn system_read_dword(&self, laddr: BxAddress) -> Result<u32> {
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_linear_system_read(laddr)?;
            Ok(self.mem_read_dword(paddr))
        } else {
            let mut buf = [0u8; 4];
            for i in 0..4u64 {
                let la = (laddr.wrapping_add(i)) & 0xFFFF_FFFF;
                let pa = self.translate_linear_system_read(la)?;
                buf[i as usize] = self.mem_read_byte(pa);
            }
            Ok(u32::from_le_bytes(buf))
        }
    }

    /// Read a qword from a system (linear) address with cross-page handling.
    /// Bochs: system_read_qword (access.cc)
    pub(super) fn system_read_qword(&self, laddr: BxAddress) -> Result<u64> {
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_linear_system_read(laddr)?;
            Ok(self.mem_read_qword(paddr))
        } else {
            let mut buf = [0u8; 8];
            for i in 0..8u64 {
                let la = (laddr.wrapping_add(i)) & 0xFFFF_FFFF;
                let pa = self.translate_linear_system_read(la)?;
                buf[i as usize] = self.mem_read_byte(pa);
            }
            Ok(u64::from_le_bytes(buf))
        }
    }

    /// Write a byte to a system (linear) address.
    /// Bochs: system_write_byte (access.cc)
    pub(super) fn system_write_byte(&mut self, laddr: BxAddress, data: u8) -> Result<()> {
        let paddr = self.translate_linear_system_write(laddr)?;
        self.mem_write_byte(paddr, data);
        Ok(())
    }

    /// Write a word to a system (linear) address with cross-page handling.
    /// Bochs: system_write_word (access.cc)
    pub(super) fn system_write_word(&mut self, laddr: BxAddress, data: u16) -> Result<()> {
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_linear_system_write(laddr)?;
            self.mem_write_word(paddr, data);
        } else {
            let bytes = data.to_le_bytes();
            let p0 = self.translate_linear_system_write(laddr)?;
            self.mem_write_byte(p0, bytes[0]);
            let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & 0xFFFF_FFFF;
            let p1 = self.translate_linear_system_write(laddr2)?;
            self.mem_write_byte(p1, bytes[1]);
        }
        Ok(())
    }

    /// Write a dword to a system (linear) address with cross-page handling.
    /// Bochs: system_write_dword (access.cc)
    pub(super) fn system_write_dword(&mut self, laddr: BxAddress, data: u32) -> Result<()> {
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_linear_system_write(laddr)?;
            self.mem_write_dword(paddr, data);
        } else {
            let bytes = data.to_le_bytes();
            for i in 0..4u64 {
                let la = (laddr.wrapping_add(i)) & 0xFFFF_FFFF;
                let pa = self.translate_linear_system_write(la)?;
                self.mem_write_byte(pa, bytes[i as usize]);
            }
        }
        Ok(())
    }

    /// Write a qword to a system (linear) address with cross-page handling.
    /// Bochs: system_write_qword (access.cc)
    pub(super) fn system_write_qword(&mut self, laddr: BxAddress, data: u64) -> Result<()> {
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_linear_system_write(laddr)?;
            self.mem_write_qword(paddr, data);
        } else {
            let bytes = data.to_le_bytes();
            for i in 0..8u64 {
                let la = (laddr.wrapping_add(i)) & 0xFFFF_FFFF;
                let pa = self.translate_linear_system_write(la)?;
                self.mem_write_byte(pa, bytes[i as usize]);
            }
        }
        Ok(())
    }

    // ===== Legacy helpers (kept for backward compatibility) =====

    /// Compute linear address with limit check only.
    /// This is the old get_laddr32_seg_checked, now reimplemented using
    /// agen_read32 for proper segment type validation.
    pub fn get_laddr32_seg_checked(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        len: u32,
    ) -> Result<u32> {
        // In real mode, just add base (no segment type checks)
        if self.real_mode() {
            let base = self.get_segment_base(seg);
            return Ok((base.wrapping_add(offset as u64)) as u32);
        }
        self.agen_read32(seg, offset, len)
    }

    /// Simple linear address without any checks (used internally).
    #[inline]
    pub fn get_laddr32_seg(&self, seg: BxSegregs, offset: u32) -> u32 {
        let seg_base = self.get_segment_base(seg);
        (seg_base.wrapping_add(offset as u64)) as u32
    }
}
