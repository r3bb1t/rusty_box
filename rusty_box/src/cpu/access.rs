#![allow(dead_code)]
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
use crate::config::{BxAddress, BxPhyAddress, BxPtrEquiv};

/// BX_MAX_MEM_ACCESS_LENGTH from Bochs — maximum access size for
/// segment limit checks.  Matches the largest scalar access (qword=8).
const BX_MAX_MEM_ACCESS_LENGTH: u32 = 8;

/// Compute a pointer into a host-mapped page at the given linear address's page offset.
#[inline(always)]
pub(super) fn host_at_page_offset(host: *const u8, laddr: u64) -> *const u8 {
    // SAFETY: host points to a valid page (validated during TLB fill),
    // offset is within page (masked to 12 bits)
    unsafe { host.add((laddr & 0xFFF) as usize) }
}

/// Mutable variant of [`host_at_page_offset`].
#[inline(always)]
pub(super) fn host_at_page_offset_mut(host: *mut u8, laddr: u64) -> *mut u8 {
    // SAFETY: host points to a valid page (validated during TLB fill),
    // offset is within page (masked to 12 bits)
    unsafe { host.add((laddr & 0xFFF) as usize) }
}

// --- Safe wrappers for unaligned memory access (ptr-based) ---

/// Read a `u16` from an unaligned `*const u8` pointer.
#[inline(always)]
pub(super) fn read_unaligned_u16(ptr: *const u8) -> u16 {
    unsafe { (ptr as *const u16).read_unaligned() }
}

/// Read a `u32` from an unaligned `*const u8` pointer.
#[inline(always)]
pub(super) fn read_unaligned_u32(ptr: *const u8) -> u32 {
    unsafe { (ptr as *const u32).read_unaligned() }
}

/// Read a `u64` from an unaligned `*const u8` pointer.
#[inline(always)]
pub(super) fn read_unaligned_u64(ptr: *const u8) -> u64 {
    unsafe { (ptr as *const u64).read_unaligned() }
}

/// Write a `u16` to an unaligned `*mut u8` pointer.
#[inline(always)]
pub(super) fn write_unaligned_u16(ptr: *mut u8, val: u16) {
    unsafe { (ptr as *mut u16).write_unaligned(val) }
}

/// Write a `u32` to an unaligned `*mut u8` pointer.
#[inline(always)]
pub(super) fn write_unaligned_u32(ptr: *mut u8, val: u32) {
    unsafe { (ptr as *mut u32).write_unaligned(val) }
}

/// Write a `u64` to an unaligned `*mut u8` pointer.
#[inline(always)]
pub(super) fn write_unaligned_u64(ptr: *mut u8, val: u64) {
    unsafe { (ptr as *mut u64).write_unaligned(val) }
}

// --- Safe wrappers for host pointer arithmetic ---

/// Offset a host pointer by `offset` bytes (const variant).
#[inline(always)]
pub(super) fn host_offset(base: *const u8, offset: usize) -> *const u8 {
    // SAFETY: caller guarantees base + offset is within a valid allocation
    unsafe { base.add(offset) }
}

/// Offset a host pointer by `offset` bytes (mut variant).
#[inline(always)]
pub(super) fn host_offset_mut(base: *mut u8, offset: usize) -> *mut u8 {
    // SAFETY: caller guarantees base + offset is within a valid allocation
    unsafe { base.add(offset) }
}

/// Read a single byte at `base + offset`.
#[inline(always)]
pub(super) fn read_host_byte(base: *const u8, offset: usize) -> u8 {
    // SAFETY: caller guarantees base + offset is valid and readable
    unsafe { *base.add(offset) }
}

/// Write a single byte at `base + offset`.
#[inline(always)]
pub(super) fn write_host_byte(base: *mut u8, offset: usize, val: u8) {
    // SAFETY: caller guarantees base + offset is valid and writable
    unsafe { *base.add(offset) = val }
}

/// Forward byte-by-byte copy from `src` to `dst` for `count` bytes.
/// Must NOT use memcpy: overlapping regions (LZ decompression) rely on
/// reading already-written bytes during forward copy.
#[inline(always)]
pub(super) fn forward_byte_copy(src: *const u8, dst: *mut u8, count: usize) {
    // SAFETY: caller guarantees both pointers are valid for `count` bytes
    unsafe {
        for j in 0..count {
            *dst.add(j) = *src.add(j);
        }
    }
}

/// Fill `count` bytes at `dst` with `val` (memset).
#[inline(always)]
pub(super) fn host_fill_bytes(dst: *mut u8, val: u8, count: usize) {
    // SAFETY: caller guarantees dst is valid for `count` bytes
    unsafe { core::ptr::write_bytes(dst, val, count) }
}

/// Create a mutable `&[u16]` slice from a raw `*mut u8` pointer.
///
/// # Safety
/// `ptr` must be valid for `count * 2` bytes. No aliasing references may exist.
#[inline(always)]
pub(super) unsafe fn host_slice_mut_u16<'a>(ptr: *mut u8, count: usize) -> &'a mut [u16] {
    core::slice::from_raw_parts_mut(ptr as *mut u16, count)
}

/// Create a mutable `&[u32]` slice from a raw `*mut u8` pointer.
///
/// # Safety
/// `ptr` must be valid for `count * 4` bytes. No aliasing references may exist.
#[inline(always)]
pub(super) unsafe fn host_slice_mut_u32<'a>(ptr: *mut u8, count: usize) -> &'a mut [u32] {
    core::slice::from_raw_parts_mut(ptr as *mut u32, count)
}

/// Create a mutable `&[u64]` slice from a raw `*mut u8` pointer.
///
/// # Safety
/// `ptr` must be valid for `count * 8` bytes. No aliasing references may exist.
#[inline(always)]
pub(super) unsafe fn host_slice_mut_u64<'a>(ptr: *mut u8, count: usize) -> &'a mut [u64] {
    core::slice::from_raw_parts_mut(ptr as *mut u64, count)
}

/// Create an immutable `&[u8]` slice from a raw pointer.
///
/// # Safety
/// `ptr` must be valid for `len` bytes. No mutable aliasing references may exist.
#[inline(always)]
pub(super) unsafe fn host_slice_u8<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    core::slice::from_raw_parts(ptr, len)
}

// --- Safe wrappers for unaligned memory access (address-based) ---

/// Read a `u8` from a host address stored as `BxPtrEquiv`.
#[inline(always)]
fn addr_read_u8(addr: BxPtrEquiv) -> u8 {
    unsafe { *(addr as *const u8) }
}

/// Read a `u16` (unaligned) from a host address stored as `BxPtrEquiv`.
#[inline(always)]
fn addr_read_u16(addr: BxPtrEquiv) -> u16 {
    unsafe { (addr as *const u16).read_unaligned() }
}

/// Read a `u32` (unaligned) from a host address stored as `BxPtrEquiv`.
#[inline(always)]
fn addr_read_u32(addr: BxPtrEquiv) -> u32 {
    unsafe { (addr as *const u32).read_unaligned() }
}

/// Read a `u64` (unaligned) from a host address stored as `BxPtrEquiv`.
#[inline(always)]
fn addr_read_u64(addr: BxPtrEquiv) -> u64 {
    unsafe { (addr as *const u64).read_unaligned() }
}

/// Write a `u64` (unaligned) to a host address stored as `BxPtrEquiv`.
#[inline(always)]
fn addr_write_u64(addr: BxPtrEquiv, val: u64) {
    unsafe { (addr as *mut u64).write_unaligned(val) }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
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

    /// Check canonical address for 64-bit data access.
    /// Raises #GP(0) for non-stack segments, #SS(0) for SS.
    /// Bochs: access_read_linear (access.cc) / access_write_linear (access.cc)
    #[inline]
    fn check_canonical_data(&mut self, seg: BxSegregs, laddr: u64, rw: MemoryAccessType) -> Result<()> {
        if self.long64_mode() {
            let user = self.user_pl;
            if !self.is_canonical_access(laddr, rw, user) {
                self.exception(Self::seg_exception(seg), 0)?;
            }
        }
        Ok(())
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

        let limit_scaled = cache.u.segment_limit_scaled();
        let base = cache.u.segment_base();

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
            let d_b = cache.u.segment_d_b();
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

        let limit_scaled = cache.u.segment_limit_scaled();
        let base = cache.u.segment_base();

        // Expand-down segments (types 4,5,6,7)
        if (seg_type & 0x08) == 0 && (seg_type & 0x04) != 0 {
            let d_b = cache.u.segment_d_b();
            let upper_limit: u32 = if d_b { 0xFFFFFFFF } else { 0x0000FFFF };
            if offset <= limit_scaled || offset > upper_limit || (upper_limit - offset) < length {
                return false;
            }
            return true;
        }

        // Normal (expand-up) data or readable code segment
        // Bochs access.cc: read checks only set ROK flags, NOT WOK
        if limit_scaled == 0xFFFFFFFF && base == 0 {
            self.sregs[seg_idx].cache.valid |= SEG_ACCESS_ROK | SEG_ACCESS_ROK4_G;
            return true;
        }
        if offset > limit_scaled.wrapping_sub(length) || length > limit_scaled {
            return false;
        }
        if limit_scaled >= (BX_MAX_MEM_ACCESS_LENGTH - 1) {
            self.sregs[seg_idx].cache.valid |= SEG_ACCESS_ROK;
        }

        true
    }

    // ===== Address generation (Bochs agen_read32 / agen_write32) =====

    /// Compute linear address for a read access with full segment validation.
    /// Bochs: agen_read32
    #[inline]
    pub(super) fn agen_read32(&mut self, seg: BxSegregs, offset: u32, len: u32) -> Result<u32> {
        let seg_idx = seg as usize;

        // In long mode, segment limits don't apply (Bochs uses separate agen_read64).
        // Only FS/GS have non-zero bases; for DS/ES/SS/CS base is forced to 0.
        if self.long_mode() {
            return Ok(self.get_laddr32(seg_idx, offset));
        }

        // Fast path: flat 4GB readable segment
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_ROK4_G) != 0 {
            return Ok(offset);
        }

        // Medium path: within cached limit
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_ROK) != 0 {
            let limit = self.sregs[seg_idx].cache.u.segment_limit_scaled();
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

        // In long mode, segment limits don't apply (Bochs uses separate agen_write64).
        if self.long_mode() {
            return Ok(self.get_laddr32(seg_idx, offset));
        }

        // Fast path: flat 4GB writable segment
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_WOK4_G) != 0 {
            return Ok(offset);
        }

        // Medium path: within cached limit
        if (self.sregs[seg_idx].cache.valid & SEG_ACCESS_WOK) != 0 {
            let limit = self.sregs[seg_idx].cache.u.segment_limit_scaled();
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
    /// Bochs: read_virtual_byte_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_virtual_byte(&mut self, seg: BxSegregs, offset: u32) -> Result<u8> {
        let laddr = self.agen_read32(seg, offset, 1)? as u64;
        self.read_linear_byte(seg, laddr)
    }

    /// Read a word from virtual memory.
    /// Bochs: read_virtual_word_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_virtual_word(&mut self, seg: BxSegregs, offset: u32) -> Result<u16> {
        let laddr = self.agen_read32(seg, offset, 2)? as u64;
        self.read_linear_word(seg, laddr)
    }

    /// Read a dword from virtual memory.
    /// Bochs: read_virtual_dword_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_virtual_dword(&mut self, seg: BxSegregs, offset: u32) -> Result<u32> {
        let laddr = self.agen_read32(seg, offset, 4)? as u64;
        self.read_linear_dword(seg, laddr)
    }

    /// Read a qword from virtual memory.
    /// Bochs: read_virtual_qword_32 (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_virtual_qword(&mut self, seg: BxSegregs, offset: u32) -> Result<u64> {
        let laddr = self.agen_read32(seg, offset, 8)? as u64;
        self.read_linear_qword(seg, laddr)
    }

    /// Internal helper: read a single byte at a given linear address.
    /// Delegates to read_linear_byte (no segment needed for cross-page helpers).
    #[inline]
    pub(super) fn read_virtual_byte_at_laddr(&mut self, laddr: u64) -> Result<u8> {
        self.read_linear_byte(BxSegregs::Ds, laddr)
    }

    // ===== Virtual write functions (Bochs access.h + access2.cc) =====

    /// Write a byte to virtual memory.
    /// Bochs: write_virtual_byte_32 (access.h) — thin wrapper
    #[inline]
    pub fn write_virtual_byte(&mut self, seg: BxSegregs, offset: u32, val: u8) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 1)? as u64;
        self.write_linear_byte(seg, laddr, val)
    }

    /// Write a word to virtual memory.
    /// Bochs: write_virtual_word_32 (access.h) — thin wrapper
    #[inline]
    pub(super) fn write_virtual_word(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        val: u16,
    ) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 2)? as u64;
        self.write_linear_word(seg, laddr, val)
    }

    /// Write a dword to virtual memory.
    /// Bochs: write_virtual_dword_32 (access.h) — thin wrapper
    #[inline]
    pub(super) fn write_virtual_dword(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        val: u32,
    ) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 4)? as u64;
        self.write_linear_dword(seg, laddr, val)
    }

    /// Write a qword to virtual memory.
    /// Bochs: write_virtual_qword_32 (access.h) — thin wrapper
    #[inline]
    pub(crate) fn write_virtual_qword(
        &mut self,
        seg: BxSegregs,
        offset: u32,
        val: u64,
    ) -> Result<()> {
        let laddr = self.agen_write32(seg, offset, 8)? as u64;
        self.write_linear_qword(seg, laddr, val)
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
        let mut r = super::xmm::BxPackedXmmRegister::default();
        r.set_xmm64u(0, lo);
        r.set_xmm64u(1, hi);
        Ok(r)
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
        self.write_virtual_qword(seg, offset, val.xmm64u(0))?;
        self.write_virtual_qword(seg, offset.wrapping_add(8), val.xmm64u(1))?;
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
    /// Delegates to write_linear_byte (no segment needed for cross-page helpers).
    #[inline]
    pub(super) fn write_virtual_byte_at_laddr(&mut self, laddr: u64, val: u8) -> Result<()> {
        self.write_linear_byte(BxSegregs::Ds, laddr, val)
    }

    // ===== Read-Modify-Write virtual functions (Bochs access2.cc) =====
    //
    // These populate `self.address_xlation` for the write-back phase:
    //   pages > 2  →  host pointer stored (direct write-back, fastest)
    //   pages == 1 →  single-page physical address in paddress1
    //   pages == 2 →  cross-page: paddress1/paddress2 + len1/len2

    /// Read phase of a read-modify-write byte access.
    /// Bochs: read_RMW_virtual_byte_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_rmw_virtual_byte(&mut self, seg: BxSegregs, offset: u32) -> Result<u8> {
        let laddr = self.agen_write32(seg, offset, 1)? as u64;
        self.read_rmw_linear_byte(seg, laddr)
    }

    /// Read phase of a read-modify-write word access.
    /// Bochs: read_RMW_virtual_word_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_rmw_virtual_word(&mut self, seg: BxSegregs, offset: u32) -> Result<u16> {
        let laddr = self.agen_write32(seg, offset, 2)? as u64;
        self.read_rmw_linear_word(seg, laddr)
    }

    /// Read phase of a read-modify-write dword access.
    /// Bochs: read_RMW_virtual_dword_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_rmw_virtual_dword(&mut self, seg: BxSegregs, offset: u32) -> Result<u32> {
        let laddr = self.agen_write32(seg, offset, 4)? as u64;
        self.read_rmw_linear_dword(seg, laddr)
    }

    /// RMW read qword in 32-bit mode.
    /// Bochs: read_RMW_virtual_qword_32 (access.h) — thin wrapper
    pub fn read_rmw_virtual_qword(&mut self, seg: BxSegregs, offset: u32) -> Result<u64> {
        let laddr = self.agen_write32(seg, offset, 8)? as u64;
        let (data, _) = self.read_rmw_linear_qword(seg, laddr)?;
        Ok(data)
    }

    // ===== System read/write functions (Bochs access.cc) =====
    //
    // These bypass segment checks and operate on raw linear addresses at
    // CPL=0 (supervisor).  They still go through paging translation.

    /// Translate a system-level linear address to physical using the DTLB.
    /// Falls back to a raw page walk if paging is disabled or in non-long mode.
    /// In long mode, routes through translate_data_access so the DTLB is
    /// populated — matching Bochs where access_read_linear always uses the TLB.
    fn translate_system_read_via_dtlb(&mut self, laddr: BxAddress) -> Result<u64> {
        if self.cr0.pg() && self.long_mode() {
            // In long mode, use the DTLB path (supervisor read).
            // Temporarily force supervisor access so user_pl doesn't interfere.
            let saved_user_pl = self.user_pl;
            self.user_pl = false;
            let result = self.translate_data_read(laddr);
            self.user_pl = saved_user_pl;
            result
        } else {
            self.translate_linear_system_read(laddr)
        }
    }

    /// Read a byte from a system (linear) address.
    /// Bochs: system_read_byte (access.cc)
    pub(super) fn system_read_byte(&mut self, laddr: BxAddress) -> Result<u8> {
        let paddr = self.translate_system_read_via_dtlb(laddr)?;
        Ok(self.mem_read_byte(paddr))
    }

    /// Read a word from a system (linear) address with cross-page handling.
    /// Bochs: system_read_word (access.cc)
    pub(super) fn system_read_word(&mut self, laddr: BxAddress) -> Result<u16> {
        let page_offset = laddr & 0xFFF;
        let laddr_mask = if self.long_mode() { 0xFFFF_FFFF_FFFF_FFFF } else { 0xFFFF_FFFF };
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_system_read_via_dtlb(laddr)?;
            Ok(self.mem_read_word(paddr))
        } else {
            let p0 = self.translate_system_read_via_dtlb(laddr)?;
            let b0 = self.mem_read_byte(p0);
            let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & laddr_mask;
            let p1 = self.translate_system_read_via_dtlb(laddr2)?;
            let b1 = self.mem_read_byte(p1);
            Ok(u16::from_le_bytes([b0, b1]))
        }
    }

    /// Read a dword from a system (linear) address with cross-page handling.
    /// Bochs: system_read_dword (access.cc)
    pub(super) fn system_read_dword(&mut self, laddr: BxAddress) -> Result<u32> {
        let page_offset = laddr & 0xFFF;
        let laddr_mask = if self.long_mode() { 0xFFFF_FFFF_FFFF_FFFF } else { 0xFFFF_FFFF };
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_system_read_via_dtlb(laddr)?;
            Ok(self.mem_read_dword(paddr))
        } else {
            let mut buf = [0u8; 4];
            for i in 0..4u64 {
                let la = (laddr.wrapping_add(i)) & laddr_mask;
                let pa = self.translate_system_read_via_dtlb(la)?;
                buf[i as usize] = self.mem_read_byte(pa);
            }
            Ok(u32::from_le_bytes(buf))
        }
    }

    /// Read a qword from a system (linear) address with cross-page handling.
    /// Bochs: system_read_qword (access.cc)
    pub(super) fn system_read_qword(&mut self, laddr: BxAddress) -> Result<u64> {
        let page_offset = laddr & 0xFFF;
        let laddr_mask = if self.long_mode() { 0xFFFF_FFFF_FFFF_FFFF } else { 0xFFFF_FFFF };
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_system_read_via_dtlb(laddr)?;
            Ok(self.mem_read_qword(paddr))
        } else {
            let mut buf = [0u8; 8];
            for i in 0..8u64 {
                let la = (laddr.wrapping_add(i)) & laddr_mask;
                let pa = self.translate_system_read_via_dtlb(la)?;
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
        let laddr_mask = if self.long_mode() { 0xFFFF_FFFF_FFFF_FFFF } else { 0xFFFF_FFFF };
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_linear_system_write(laddr)?;
            self.mem_write_word(paddr, data);
        } else {
            let bytes = data.to_le_bytes();
            let p0 = self.translate_linear_system_write(laddr)?;
            self.mem_write_byte(p0, bytes[0]);
            let laddr2 = (laddr & 0xFFFF_F000).wrapping_add(0x1000) & laddr_mask;
            let p1 = self.translate_linear_system_write(laddr2)?;
            self.mem_write_byte(p1, bytes[1]);
        }
        Ok(())
    }

    /// Write a dword to a system (linear) address with cross-page handling.
    /// Bochs: system_write_dword (access.cc)
    pub(super) fn system_write_dword(&mut self, laddr: BxAddress, data: u32) -> Result<()> {
        self.check_gdt_watchpoint(laddr, data as u64, 4);
        let page_offset = laddr & 0xFFF;
        let laddr_mask = if self.long_mode() { 0xFFFF_FFFF_FFFF_FFFF } else { 0xFFFF_FFFF };
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_linear_system_write(laddr)?;
            self.mem_write_dword(paddr, data);
        } else {
            let bytes = data.to_le_bytes();
            for i in 0..4u64 {
                let la = (laddr.wrapping_add(i)) & laddr_mask;
                let pa = self.translate_linear_system_write(la)?;
                self.mem_write_byte(pa, bytes[i as usize]);
            }
        }
        Ok(())
    }

    /// Write a qword to a system (linear) address with cross-page handling.
    /// Bochs: system_write_qword (access.cc)
    pub(super) fn system_write_qword(&mut self, laddr: BxAddress, data: u64) -> Result<()> {
        self.check_gdt_watchpoint(laddr, data, 8);
        let page_offset = laddr & 0xFFF;
        let laddr_mask = if self.long_mode() { 0xFFFF_FFFF_FFFF_FFFF } else { 0xFFFF_FFFF };
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_linear_system_write(laddr)?;
            self.mem_write_qword(paddr, data);
        } else {
            let bytes = data.to_le_bytes();
            for i in 0..8u64 {
                let la = (laddr.wrapping_add(i)) & laddr_mask;
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

    // ===== 64-bit Virtual read functions (Bochs access64.cc) =====
    //
    // In 64-bit long mode:
    //  - Segment limits are not checked (flat addressing)
    //  - Only FS and GS have non-zero segment bases
    //  - Linear addresses are 64-bit (canonical check in translate_data_access)
    //  - Paging is always active (CR0.PG must be set for long mode)

    /// Read a byte from virtual memory in 64-bit mode.
    /// Bochs: read_virtual_byte (access.h) — thin wrapper: agen + canonical + read_linear_byte
    #[inline]
    pub(crate) fn read_virtual_byte_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u8> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Read)?;
        self.read_linear_byte(seg, laddr)
    }

    /// Read a word from virtual memory in 64-bit mode.
    /// Bochs: read_virtual_word (access.h) — thin wrapper: agen + canonical + read_linear_word
    #[inline]
    pub(crate) fn read_virtual_word_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u16> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Read)?;
        self.read_linear_word(seg, laddr)
    }

    /// Read a dword from virtual memory in 64-bit mode.
    /// Bochs: read_virtual_dword (access.h) — thin wrapper: agen + canonical + read_linear_dword
    #[inline]
    pub(crate) fn read_virtual_dword_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u32> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Read)?;
        self.read_linear_dword(seg, laddr)
    }

    /// Read a qword from virtual memory in 64-bit mode.
    /// Bochs: read_virtual_qword (access.h) — thin wrapper: agen + canonical + read_linear_qword
    #[inline]
    pub(crate) fn read_virtual_qword_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u64> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Read)?;
        self.read_linear_qword(seg, laddr)
    }

    // ===== 64-bit Virtual write functions =====

    /// Write a byte to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_byte (access.h) — thin wrapper: agen + canonical + write_linear_byte
    #[inline]
    pub(crate) fn write_virtual_byte_64(&mut self, seg: BxSegregs, offset: u64, val: u8) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_byte(seg, laddr, val)
    }

    /// Write a word to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_word (access.h) — thin wrapper: agen + canonical + write_linear_word
    #[inline]
    pub(crate) fn write_virtual_word_64(&mut self, seg: BxSegregs, offset: u64, val: u16) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_word(seg, laddr, val)
    }

    /// Write a dword to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_dword (access.h) — thin wrapper: agen + canonical + write_linear_dword
    #[inline]
    pub(crate) fn write_virtual_dword_64(&mut self, seg: BxSegregs, offset: u64, val: u32) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_dword(seg, laddr, val)
    }

    /// Write a qword to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_qword (access.h) — thin wrapper: agen + canonical + write_linear_qword
    #[inline]
    pub(crate) fn write_virtual_qword_64(&mut self, seg: BxSegregs, offset: u64, val: u64) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_qword(seg, laddr, val)
    }

    // ===== 64-bit Read-Modify-Write functions =====

    /// Read phase of a RMW qword access in 64-bit mode.
    /// Bochs: read_RMW_virtual_qword (access.h) — thin wrapper
    pub(crate) fn read_rmw_virtual_qword_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u64> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        let (data, _) = self.read_rmw_linear_qword(seg, laddr)?;
        Ok(data)
    }

    /// Write phase of a RMW qword access (uses cached address_xlation).
    pub(crate) fn write_rmw_virtual_qword_back_64(&mut self, val: u64) {
        let pages = self.address_xlation.pages;
        if pages > 2 {
            // Host pointer cached from TLB hit — direct write (fastest path)
            // SAFETY: address_xlation.pages set during address translation; pointer valid for write
            addr_write_u64(pages, val);
        } else if pages == 1 {
            let paddr = self.address_xlation.paddress1;
            self.mem_write_qword(paddr, val);
        } else {
            let bytes = val.to_le_bytes();
            let len1 = self.address_xlation.len1 as usize;
            let len2 = self.address_xlation.len2 as usize;
            let p0 = self.address_xlation.paddress1;
            let p1 = self.address_xlation.paddress2;
            for (i, &byte) in bytes[..len1].iter().enumerate() {
                self.mem_write_byte(p0 + i as u64, byte);
            }
            for (i, &byte) in bytes[len1..len1+len2].iter().enumerate() {
                self.mem_write_byte(p1 + i as u64, byte);
            }
        }
    }

    // ===== 64-bit Stack access functions =====

    /// Read a word from the stack in 64-bit mode (SS segment).
    /// Bochs: stack_read_word (long64 path)
    #[inline]
    pub(crate) fn stack_read_word_64(&mut self, offset: u64) -> Result<u16> {
        self.read_virtual_word_64(BxSegregs::Ss, offset)
    }

    /// Write a word to the stack in 64-bit mode (SS segment).
    /// Bochs: stack_write_word (long64 path)
    #[inline]
    pub(crate) fn stack_write_word_64(&mut self, offset: u64, val: u16) -> Result<()> {
        self.write_virtual_word_64(BxSegregs::Ss, offset, val)
    }

    /// Read a dword from the stack in 64-bit mode (SS segment).
    /// Bochs: stack_read_dword (long64 path)
    #[inline]
    pub(crate) fn stack_read_dword_64(&mut self, offset: u64) -> Result<u32> {
        self.read_virtual_dword_64(BxSegregs::Ss, offset)
    }

    /// Write a dword to the stack in 64-bit mode (SS segment).
    /// Bochs: stack_write_dword (long64 path)
    #[inline]
    pub(crate) fn stack_write_dword_64(&mut self, offset: u64, val: u32) -> Result<()> {
        self.write_virtual_dword_64(BxSegregs::Ss, offset, val)
    }

    /// Read a qword from the stack in 64-bit mode (SS segment).
    /// Bochs: stack_read_qword
    #[inline]
    pub(crate) fn stack_read_qword_64(&mut self, offset: u64) -> Result<u64> {
        self.read_virtual_qword_64(BxSegregs::Ss, offset)
    }

    /// Write a qword to the stack in 64-bit mode (SS segment).
    /// Bochs: stack_write_qword
    #[inline]
    pub(crate) fn stack_write_qword_64(&mut self, offset: u64, val: u64) -> Result<()> {
        self.write_virtual_qword_64(BxSegregs::Ss, offset, val)
    }

    // ===== Host pointer resolution for bulk operations (Bochs v2h_write_byte / v2h_read_byte) =====
    //
    // Used by FastRep string ops and REP INSW for direct memcpy/memset to host memory.
    // Returns a mutable host pointer if the linear address hits a TLB entry with a valid
    // host page addr. Returns None on TLB miss or MMIO (host_page_addr == 0).

    /// Resolve a linear address to a host write pointer via TLB.
    /// Returns (host_ptr, bytes_remaining_in_page) or None on miss.
    /// Bochs: v2h_write_byte (access.h)
    #[inline]
    pub(super) fn get_host_write_ptr(&mut self, laddr: u64) -> Option<(*mut u8, usize)> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 0);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as usize;
            let host = tlb.host_page_addr as *mut u8;
            // SAFETY: host pointer validated during TLB fill; offset within page bounds
            let ptr = unsafe { host.add(page_offset) };
            let remaining = 0x1000 - page_offset;
            // SMC check for the page
            let paddr = tlb.ppf | page_offset as BxPhyAddress;
            self.i_cache.smc_write_check(paddr, remaining as u32);
            Some((ptr, remaining))
        } else {
            None
        }
    }

    /// Resolve a linear address to a host read pointer via TLB.
    /// Returns (host_ptr, bytes_remaining_in_page) or None on miss.
    /// Bochs: v2h_read_byte (access.h)
    #[inline]
    pub(super) fn get_host_read_ptr(&mut self, laddr: u64) -> Option<(*const u8, usize)> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 0);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as usize;
            let host = tlb.host_page_addr as *const u8;
            // SAFETY: host pointer validated during TLB fill; offset within page bounds
            let ptr = unsafe { host.add(page_offset) };
            let remaining = 0x1000 - page_offset;
            Some((ptr, remaining))
        } else {
            None
        }
    }

    // ===== Linear address paging wrappers (Bochs access2.cc) =====
    //
    // These accept a PRE-COMPUTED linear address and translate it through paging
    // with inline TLB fast paths. Used by both the 64-bit virtual_*_64 thin
    // wrappers and by arith64/logical64/shift64/mult64/bit64 which compute
    // laddr before calling the access function.
    //
    // Matches the Bochs read_linear_byte/word/dword/qword and
    // write_linear_byte/word/dword/qword functions in access2.cc.

    // ── Permission & MMIO helpers for hot-path memory access ──

    #[cfg(feature = "instrumentation")]
    #[inline]
    fn check_perm_read(&mut self, laddr: u64, paddr: u64, size: usize) -> Result<()> {
        if let Some(ref pp) = self.page_permissions {
            if !pp.check(paddr, super::instrumentation::MemPerms::READ) {
                if self.instrumentation.active.has_mem_perm()
                    && self.instrumentation.fire_mem_perm_violation(
                        &super::instrumentation::MemPermViolation {
                            laddr,
                            size,
                            rw: super::instrumentation::MemAccessRW::Read,
                            required: super::instrumentation::MemPerms::READ,
                        },
                    )
                {
                    return Ok(()); // hook suppressed
                }
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        Ok(())
    }

    #[cfg(feature = "instrumentation")]
    #[inline]
    fn check_perm_write(&mut self, laddr: u64, paddr: u64, size: usize) -> Result<()> {
        if let Some(ref pp) = self.page_permissions {
            if !pp.check(paddr, super::instrumentation::MemPerms::WRITE) {
                if self.instrumentation.active.has_mem_perm()
                    && self.instrumentation.fire_mem_perm_violation(
                        &super::instrumentation::MemPermViolation {
                            laddr,
                            size,
                            rw: super::instrumentation::MemAccessRW::Write,
                            required: super::instrumentation::MemPerms::WRITE,
                        },
                    )
                {
                    return Ok(()); // hook suppressed
                }
                return self.exception(super::cpu::Exception::Gp, 0);
            }
        }
        Ok(())
    }

    #[cfg(feature = "alloc")]
    #[inline]
    fn mmio_read(&mut self, paddr: u64, size: usize) -> Option<u64> {
        if self.mmio.is_empty() { return None; }
        if let Some(region) = self.mmio.find_mut(paddr) {
            return Some((region.read_cb)(paddr, size));
        }
        None
    }

    #[cfg(feature = "alloc")]
    #[inline]
    fn mmio_write(&mut self, paddr: u64, size: usize, val: u64) -> bool {
        if self.mmio.is_empty() { return false; }
        if let Some(region) = self.mmio.find_mut(paddr) {
            (region.write_cb)(paddr, size, val);
            return true;
        }
        false
    }

    /// Read a byte given a pre-computed linear address.
    /// Bochs: read_linear_byte (access2.cc)
    pub(crate) fn read_linear_byte(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u8> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 0);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 1)?;
            let v = unsafe { *host_at_page_offset(host, laddr) };
            #[cfg(feature = "instrumentation")]
            { let _buf = [v]; self.on_lin_access(laddr, paddr_hit, &_buf, super::instrumentation::MemAccessRW::Read); }
            return Ok(v);
        }
        let paddr = self.translate_data_read(laddr)?;
        #[cfg(feature = "instrumentation")]
        self.check_perm_read(laddr, paddr, 1)?;
        #[cfg(feature = "alloc")]
        if let Some(val) = self.mmio_read(paddr, 1) {
            return Ok(val as u8);
        }
        let v = self.mem_read_byte(paddr);
        #[cfg(feature = "instrumentation")]
            { let _buf = [v]; self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Read); }
        Ok(v)
    }

    /// Read a word given a pre-computed linear address with cross-page handling.
    /// Bochs: read_linear_word (access2.cc)
    pub(crate) fn read_linear_word(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u16> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 1);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 2)?;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let v = read_unaligned_u16(ptr);
            #[cfg(feature = "instrumentation")]
            { let _buf = v.to_le_bytes(); self.on_lin_access(laddr, paddr_hit, &_buf, super::instrumentation::MemAccessRW::Read); }
            return Ok(v);
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr, 2)?;
            #[cfg(feature = "alloc")]
            if let Some(val) = self.mmio_read(paddr, 2) {
                return Ok(val as u16);
            }
            let v = self.mem_read_word(paddr);
            #[cfg(feature = "instrumentation")]
            { let _buf = v.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Read); }
            Ok(v)
        } else {
            let p0 = self.translate_data_read(laddr)?;
            let b0 = self.mem_read_byte(p0);
            let p1 = self.translate_data_read((laddr | 0xFFF).wrapping_add(1))?;
            let b1 = self.mem_read_byte(p1);
            Ok(u16::from_le_bytes([b0, b1]))
        }
    }

    /// Read a dword given a pre-computed linear address with cross-page handling.
    /// Bochs: read_linear_dword (access2.cc)
    pub(crate) fn read_linear_dword(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u32> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 4)?;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let v = read_unaligned_u32(ptr);
            #[cfg(feature = "instrumentation")]
            { let _buf = v.to_le_bytes(); self.on_lin_access(laddr, paddr_hit, &_buf, super::instrumentation::MemAccessRW::Read); }
            return Ok(v);
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr, 4)?;
            #[cfg(feature = "alloc")]
            if let Some(val) = self.mmio_read(paddr, 4) {
                return Ok(val as u32);
            }
            let v = self.mem_read_dword(paddr);
            #[cfg(feature = "instrumentation")]
            { let _buf = v.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Read); }
            Ok(v)
        } else {
            let mut buf = [0u8; 4];
            for i in 0..4u64 {
                let p = self.translate_data_read(laddr.wrapping_add(i))?;
                buf[i as usize] = self.mem_read_byte(p);
            }
            Ok(u32::from_le_bytes(buf))
        }
    }

    /// Read a qword given a pre-computed linear address with cross-page handling.
    /// Bochs: read_linear_qword (access2.cc)
    pub(crate) fn read_linear_qword(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u64> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 8)?;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let v = read_unaligned_u64(ptr);
            #[cfg(feature = "instrumentation")]
            { let _buf = v.to_le_bytes(); self.on_lin_access(laddr, paddr_hit, &_buf, super::instrumentation::MemAccessRW::Read); }
            return Ok(v);
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr, 8)?;
            #[cfg(feature = "alloc")]
            if let Some(val) = self.mmio_read(paddr, 8) {
                return Ok(val);
            }
            let v = self.mem_read_qword(paddr);
            #[cfg(feature = "instrumentation")]
            { let _buf = v.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Read); }
            Ok(v)
        } else {
            let mut buf = [0u8; 8];
            for i in 0..8u64 {
                let p = self.translate_data_read(laddr.wrapping_add(i))?;
                buf[i as usize] = self.mem_read_byte(p);
            }
            Ok(u64::from_le_bytes(buf))
        }
    }

    /// Write a byte given a pre-computed linear address.
    /// Bochs: write_linear_byte (access2.cc)
    pub(crate) fn write_linear_byte(&mut self, _seg: BxSegregs, laddr: u64, val: u8) -> Result<()> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 0);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 1)?;
            self.i_cache.smc_write_check(paddr, 1);
            unsafe { *host_at_page_offset_mut(host, laddr) = val };
            #[cfg(feature = "instrumentation")]
            { let _buf = [val]; self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
            return Ok(());
        }
        let paddr = self.translate_data_write(laddr)?;
        #[cfg(feature = "instrumentation")]
        self.check_perm_write(laddr, paddr, 1)?;
        #[cfg(feature = "alloc")]
        if self.mmio_write(paddr, 1, val as u64) {
            return Ok(());
        }
        self.i_cache.smc_write_check(paddr, 1);
        self.mem_write_byte(paddr, val);
        #[cfg(feature = "instrumentation")]
        { let _buf = [val]; self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
        Ok(())
    }

    /// Write a word given a pre-computed linear address with cross-page handling.
    /// Bochs: write_linear_word (access2.cc)
    pub(crate) fn write_linear_word(&mut self, _seg: BxSegregs, laddr: u64, val: u16) -> Result<()> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 1);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 2)?;
            self.i_cache.smc_write_check(paddr, 2);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            write_unaligned_u16(ptr, val);
            #[cfg(feature = "instrumentation")]
            { let _buf = val.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
            return Ok(());
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 2)?;
            #[cfg(feature = "alloc")]
            if self.mmio_write(paddr, 2, val as u64) {
                return Ok(());
            }
            self.i_cache.smc_write_check(paddr, 2);
            self.mem_write_word(paddr, val);
            #[cfg(feature = "instrumentation")]
            { let _buf = val.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
        } else {
            let bytes = val.to_le_bytes();
            let p0 = self.translate_data_write(laddr)?;
            self.i_cache.smc_write_check(p0, 1);
            self.mem_write_byte(p0, bytes[0]);
            let p1 = self.translate_data_write((laddr | 0xFFF).wrapping_add(1))?;
            self.i_cache.smc_write_check(p1, 1);
            self.mem_write_byte(p1, bytes[1]);
        }
        Ok(())
    }

    /// Write a dword given a pre-computed linear address with cross-page handling.
    /// Bochs: write_linear_dword (access2.cc)
    fn check_gdt_watchpoint(&mut self, _laddr: u64, _val: u64, _size: u32) {
        // Disabled — the GDT 'corruption' was caused by our own diagnostic code
        // (v_read_byte in SYSCALL handler triggering page walks that set A/D bits)
    }

    pub(crate) fn write_linear_dword(&mut self, _seg: BxSegregs, laddr: u64, val: u32) -> Result<()> {
        self.check_gdt_watchpoint(laddr, val as u64, 4);
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 4)?;
            self.i_cache.smc_write_check(paddr, 4);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            write_unaligned_u32(ptr, val);
            #[cfg(feature = "instrumentation")]
            { let _buf = val.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
            return Ok(());
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 4)?;
            #[cfg(feature = "alloc")]
            if self.mmio_write(paddr, 4, val as u64) {
                return Ok(());
            }
            self.i_cache.smc_write_check(paddr, 4);
            self.mem_write_dword(paddr, val);
            #[cfg(feature = "instrumentation")]
            { let _buf = val.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
        } else {
            let bytes = val.to_le_bytes();
            for i in 0..4u64 {
                let p = self.translate_data_write(laddr.wrapping_add(i))?;
                self.i_cache.smc_write_check(p, 1);
                self.mem_write_byte(p, bytes[i as usize]);
            }
        }
        Ok(())
    }

    /// Write a qword given a pre-computed linear address with cross-page handling.
    /// Bochs: write_linear_qword (access2.cc)
    pub(crate) fn write_linear_qword(&mut self, _seg: BxSegregs, laddr: u64, val: u64) -> Result<()> {
        self.check_gdt_watchpoint(laddr, val, 8);
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        // DIAGNOSTIC: bypass TLB for writes to test stale-TLB theory
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 8)?;
            self.i_cache.smc_write_check(paddr, 8);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            write_unaligned_u64(ptr, val);
            #[cfg(feature = "instrumentation")]
            { let _buf = val.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
            return Ok(());
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 8)?;
            #[cfg(feature = "alloc")]
            if self.mmio_write(paddr, 8, val) {
                return Ok(());
            }
            self.i_cache.smc_write_check(paddr, 8);
            self.mem_write_qword(paddr, val);
            #[cfg(feature = "instrumentation")]
            { let _buf = val.to_le_bytes(); self.on_lin_access(laddr, paddr, &_buf, super::instrumentation::MemAccessRW::Write); }
        } else {
            let bytes = val.to_le_bytes();
            for i in 0..8u64 {
                let p = self.translate_data_write(laddr.wrapping_add(i))?;
                self.i_cache.smc_write_check(p, 1);
                self.mem_write_byte(p, bytes[i as usize]);
            }
        }
        Ok(())
    }

    /// Read phase of a RMW qword given a pre-computed linear address.
    /// Bochs: read_RMW_linear_qword (access2.cc)
    /// Returns (value, laddr). Caches translation in address_xlation.
    pub(crate) fn read_rmw_linear_qword(&mut self, _seg: BxSegregs, laddr: u64) -> Result<(u64, u64)> {
        // ---- Inline TLB fast path (Bochs access2.cc) ----
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
            let host_addr = tlb.host_page_addr | page_offset;
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            self.i_cache.smc_write_check(paddr, 8);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let data = addr_read_u64(host_addr);
            self.address_xlation.pages = host_addr;
            self.address_xlation.paddress1 = paddr;
            return Ok((data, laddr));
        }

        // ---- Slow path (Bochs: access_read_linear) ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            let data = self.mem_read_qword(paddr);
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = paddr;
            Ok((data, laddr))
        } else {
            let len1 = (0x1000 - page_offset) as u32;
            let len2 = 8 - len1;
            let p0 = self.translate_data_write(laddr)?;
            let next_page = (laddr | 0xFFF).wrapping_add(1);
            let p1 = self.translate_data_write(next_page)?;
            let mut buf = [0u8; 8];
            for (i, byte) in buf[..len1 as usize].iter_mut().enumerate() {
                *byte = self.mem_read_byte(p0 + i as u64);
            }
            for (i, byte) in buf[len1 as usize..].iter_mut().enumerate() {
                *byte = self.mem_read_byte(p1 + i as u64);
            }
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = len1;
            self.address_xlation.len2 = len2;
            Ok((u64::from_le_bytes(buf), laddr))
        }
    }

    /// Read phase of a RMW byte given a pre-computed linear address.
    /// Bochs: read_RMW_linear_byte (access2.cc)
    pub(crate) fn read_rmw_linear_byte(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u8> {
        // ---- Inline TLB fast path (Bochs access2.cc) ----
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 0);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
            let host_addr = tlb.host_page_addr | page_offset;
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            self.i_cache.smc_write_check(paddr, 1);
            // SAFETY: host pointer validated during TLB fill; offset within page bounds
            let data = addr_read_u8(host_addr);
            self.address_xlation.pages = host_addr;
            self.address_xlation.paddress1 = paddr;
            return Ok(data);
        }

        // ---- Slow path (Bochs: access_read_linear) ----
        let paddr = self.translate_data_write(laddr)?;
        let data = self.mem_read_byte(paddr);
        self.address_xlation.pages = 1;
        self.address_xlation.paddress1 = paddr;
        Ok(data)
    }

    /// Read phase of a RMW word given a pre-computed linear address.
    /// Bochs: read_RMW_linear_word (access2.cc)
    pub(crate) fn read_rmw_linear_word(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u16> {
        // ---- Inline TLB fast path (Bochs access2.cc) ----
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 1);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
            let host_addr = tlb.host_page_addr | page_offset;
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            self.i_cache.smc_write_check(paddr, 2);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let data = addr_read_u16(host_addr);
            self.address_xlation.pages = host_addr;
            self.address_xlation.paddress1 = paddr;
            return Ok(data);
        }

        // ---- Slow path ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            let data = self.mem_read_word(paddr);
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = paddr;
            Ok(data)
        } else {
            let p0 = self.translate_data_write(laddr)?;
            let b0 = self.mem_read_byte(p0);
            let next_page = (laddr | 0xFFF).wrapping_add(1);
            let p1 = self.translate_data_write(next_page)?;
            let b1 = self.mem_read_byte(p1);
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = 1;
            self.address_xlation.len2 = 1;
            Ok(u16::from_le_bytes([b0, b1]))
        }
    }

    /// Read phase of a RMW dword given a pre-computed linear address.
    /// Bochs: read_RMW_linear_dword (access2.cc)
    pub(crate) fn read_rmw_linear_dword(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u32> {
        // ---- Inline TLB fast path (Bochs access2.cc) ----
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        if tlb.lpf == lpf && (tlb.access_bits & needed_bit) != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
            let host_addr = tlb.host_page_addr | page_offset;
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            self.i_cache.smc_write_check(paddr, 4);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let data = addr_read_u32(host_addr);
            self.address_xlation.pages = host_addr;
            self.address_xlation.paddress1 = paddr;
            return Ok(data);
        }

        // ---- Slow path ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            let data = self.mem_read_dword(paddr);
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = paddr;
            Ok(data)
        } else {
            let len1 = (0x1000 - page_offset) as u32;
            let len2 = 4 - len1;
            let p0 = self.translate_data_write(laddr)?;
            let next_page = (laddr | 0xFFF).wrapping_add(1);
            let p1 = self.translate_data_write(next_page)?;
            let mut buf = [0u8; 4];
            for (i, byte) in buf[..len1 as usize].iter_mut().enumerate() {
                *byte = self.mem_read_byte(p0 + i as u64);
            }
            for (i, byte) in buf[len1 as usize..].iter_mut().enumerate() {
                *byte = self.mem_read_byte(p1 + i as u64);
            }
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = len1;
            self.address_xlation.len2 = len2;
            Ok(u32::from_le_bytes(buf))
        }
    }

    /// Write phase of a RMW qword (uses cached address_xlation from read phase).
    #[inline]
    pub(crate) fn write_rmw_linear_qword(&mut self, _laddr: u64, val: u64) {
        let pages = self.address_xlation.pages;
        if pages > 2 {
            // Host pointer cached from TLB hit — direct write (fastest path)
            // SAFETY: address_xlation.pages set during address translation; pointer valid for write
            addr_write_u64(pages, val);
        } else if pages == 1 {
            let paddr = self.address_xlation.paddress1;
            self.mem_write_qword(paddr, val);
        } else {
            let bytes = val.to_le_bytes();
            let len1 = self.address_xlation.len1 as usize;
            let len2 = self.address_xlation.len2 as usize;
            let p0 = self.address_xlation.paddress1;
            let p1 = self.address_xlation.paddress2;
            for (i, &byte) in bytes[..len1].iter().enumerate() {
                self.mem_write_byte(p0 + i as u64, byte);
            }
            for (i, &byte) in bytes[len1..len1+len2].iter().enumerate() {
                self.mem_write_byte(p1 + i as u64, byte);
            }
        }
    }

    /// Read a qword from the stack given a pre-computed linear address.
    /// Used by segment_ctrl_pro.rs which computes RSP directly.
    #[inline]
    pub(crate) fn stack_read_qword(&mut self, laddr: u64) -> Result<u64> {
        self.read_linear_qword(BxSegregs::Ss, laddr)
    }

    /// Write a qword to the stack given a pre-computed linear address.
    #[inline]
    pub(crate) fn stack_write_qword(&mut self, laddr: u64, val: u64) -> Result<()> {
        self.write_linear_qword(BxSegregs::Ss, laddr, val)
    }

    // =========================================================================
    // Mode-dispatching virtual memory access wrappers
    // =========================================================================
    // These dispatch to _32 or _64 variants based on long64_mode(),
    // allowing 8/16/32-bit instruction handlers to work correctly in both modes.

    /// Read byte — dispatches to read_virtual_byte or read_virtual_byte_64.
    #[inline]
    pub fn v_read_byte(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u8> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_virtual_byte_64(seg, offset)
        } else {
            self.read_virtual_byte(seg, offset as u32)
        }
    }

    /// Read word — dispatches to read_virtual_word or read_virtual_word_64.
    #[inline]
    pub fn v_read_word(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u16> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_virtual_word_64(seg, offset)
        } else {
            self.read_virtual_word(seg, offset as u32)
        }
    }

    /// Read dword — dispatches to read_virtual_dword or read_virtual_dword_64.
    #[inline]
    pub fn v_read_dword(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u32> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_virtual_dword_64(seg, offset)
        } else {
            self.read_virtual_dword(seg, offset as u32)
        }
    }

    /// Write byte — dispatches to write_virtual_byte or write_virtual_byte_64.
    #[inline]
    pub fn v_write_byte(&mut self, seg: BxSegregs, offset: impl Into<u64>, val: u8) -> Result<()> {
        let offset = offset.into();
        if self.long64_mode() {
            self.write_virtual_byte_64(seg, offset, val)
        } else {
            self.write_virtual_byte(seg, offset as u32, val)
        }
    }

    /// Write word — dispatches to write_virtual_word or write_virtual_word_64.
    #[inline]
    pub fn v_write_word(&mut self, seg: BxSegregs, offset: impl Into<u64>, val: u16) -> Result<()> {
        let offset = offset.into();
        if self.long64_mode() {
            self.write_virtual_word_64(seg, offset, val)
        } else {
            self.write_virtual_word(seg, offset as u32, val)
        }
    }

    /// Write dword — dispatches to write_virtual_dword or write_virtual_dword_64.
    #[inline]
    pub fn v_write_dword(&mut self, seg: BxSegregs, offset: impl Into<u64>, val: u32) -> Result<()> {
        let offset = offset.into();
        if self.long64_mode() {
            self.write_virtual_dword_64(seg, offset, val)
        } else {
            self.write_virtual_dword(seg, offset as u32, val)
        }
    }

    // =========================================================================
    // Mode-dispatching RMW read wrappers
    // =========================================================================

    /// RMW read byte — dispatches to read_rmw_virtual_byte or read_rmw_virtual_byte_64.
    #[inline]
    pub fn v_read_rmw_byte(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u8> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_rmw_virtual_byte_64(seg, offset)
        } else {
            self.read_rmw_virtual_byte(seg, offset as u32)
        }
    }

    /// RMW read word — dispatches to read_rmw_virtual_word or read_rmw_virtual_word_64.
    #[inline]
    pub fn v_read_rmw_word(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u16> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_rmw_virtual_word_64(seg, offset)
        } else {
            self.read_rmw_virtual_word(seg, offset as u32)
        }
    }

    /// RMW read dword — dispatches to read_rmw_virtual_dword or read_rmw_virtual_dword_64.
    #[inline]
    pub fn v_read_rmw_dword(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u32> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_rmw_virtual_dword_64(seg, offset)
        } else {
            self.read_rmw_virtual_dword(seg, offset as u32)
        }
    }

    // ===== Mode-dispatching wrappers for qword =====

    pub fn v_read_qword(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u64> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_virtual_qword_64(seg, offset)
        } else {
            self.read_virtual_qword(seg, offset as u32)
        }
    }

    pub fn v_write_qword(&mut self, seg: BxSegregs, offset: impl Into<u64>, val: u64) -> Result<()> {
        let offset = offset.into();
        if self.long64_mode() {
            self.write_virtual_qword_64(seg, offset, val)
        } else {
            self.write_virtual_qword(seg, offset as u32, val)
        }
    }

    pub fn v_read_rmw_qword(&mut self, seg: BxSegregs, offset: impl Into<u64>) -> Result<u64> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_rmw_virtual_qword_64(seg, offset)
        } else {
            self.read_rmw_virtual_qword(seg, offset as u32)
        }
    }

    // ===== Mode-dispatching wrappers for xmmword =====

    pub fn v_read_xmmword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
    ) -> Result<super::xmm::BxPackedXmmRegister> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_virtual_xmmword_64(seg, offset)
        } else {
            self.read_virtual_xmmword(seg, offset as u32)
        }
    }

    pub fn v_read_xmmword_aligned(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
    ) -> Result<super::xmm::BxPackedXmmRegister> {
        let offset = offset.into();
        if self.long64_mode() {
            self.read_virtual_xmmword_aligned_64(seg, offset)
        } else {
            self.read_virtual_xmmword_aligned(seg, offset as u32)
        }
    }

    pub fn v_write_xmmword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
        val: &super::xmm::BxPackedXmmRegister,
    ) -> Result<()> {
        let offset = offset.into();
        if self.long64_mode() {
            self.write_virtual_xmmword_64(seg, offset, val)
        } else {
            self.write_virtual_xmmword(seg, offset as u32, val)
        }
    }

    pub fn v_write_xmmword_aligned(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
        val: &super::xmm::BxPackedXmmRegister,
    ) -> Result<()> {
        let offset = offset.into();
        if self.long64_mode() {
            self.write_virtual_xmmword_aligned_64(seg, offset, val)
        } else {
            self.write_virtual_xmmword_aligned(seg, offset as u32, val)
        }
    }

    // ===== 64-bit xmmword read/write functions =====

    /// Read a 128-bit XMM word from virtual memory in 64-bit mode.
    /// Bochs: read_virtual_xmmword_64
    pub(super) fn read_virtual_xmmword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<super::xmm::BxPackedXmmRegister> {
        let lo = self.read_virtual_qword_64(seg, offset)?;
        let hi = self.read_virtual_qword_64(seg, offset.wrapping_add(8))?;
        let mut r = super::xmm::BxPackedXmmRegister::default();
        r.set_xmm64u(0, lo);
        r.set_xmm64u(1, hi);
        Ok(r)
    }

    /// Read a 128-bit XMM word with 16-byte alignment check in 64-bit mode.
    /// Bochs: read_virtual_xmmword_aligned_64
    pub(super) fn read_virtual_xmmword_aligned_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<super::xmm::BxPackedXmmRegister> {
        if (offset & 0xF) != 0 {
            self.exception(super::cpu::Exception::Gp, 0)?;
        }
        self.read_virtual_xmmword_64(seg, offset)
    }

    /// Write a 128-bit XMM word to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_xmmword_64
    pub(super) fn write_virtual_xmmword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: &super::xmm::BxPackedXmmRegister,
    ) -> Result<()> {
        self.write_virtual_qword_64(seg, offset, val.xmm64u(0))?;
        self.write_virtual_qword_64(seg, offset.wrapping_add(8), val.xmm64u(1))?;
        Ok(())
    }

    /// Write a 128-bit XMM word with 16-byte alignment check in 64-bit mode.
    /// Bochs: write_virtual_xmmword_aligned_64
    pub(super) fn write_virtual_xmmword_aligned_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: &super::xmm::BxPackedXmmRegister,
    ) -> Result<()> {
        if (offset & 0xF) != 0 {
            self.exception(super::cpu::Exception::Gp, 0)?;
        }
        self.write_virtual_xmmword_64(seg, offset, val)
    }

    // ===== 64-bit ymmword read/write functions =====

    /// Read a 256-bit YMM word from virtual memory in 64-bit mode.
    pub(super) fn read_virtual_ymmword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<super::xmm::BxPackedYmmRegister> {
        let q0 = self.read_virtual_qword_64(seg, offset)?;
        let q1 = self.read_virtual_qword_64(seg, offset.wrapping_add(8))?;
        let q2 = self.read_virtual_qword_64(seg, offset.wrapping_add(16))?;
        let q3 = self.read_virtual_qword_64(seg, offset.wrapping_add(24))?;
        let mut r = super::xmm::BxPackedYmmRegister::default();
        r.set_ymm64u(0, q0);
        r.set_ymm64u(1, q1);
        r.set_ymm64u(2, q2);
        r.set_ymm64u(3, q3);
        Ok(r)
    }

    /// Write a 256-bit YMM word to virtual memory in 64-bit mode.
    pub(super) fn write_virtual_ymmword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: &super::xmm::BxPackedYmmRegister,
    ) -> Result<()> {
        self.write_virtual_qword_64(seg, offset, val.ymm64u(0))?;
        self.write_virtual_qword_64(seg, offset.wrapping_add(8), val.ymm64u(1))?;
        self.write_virtual_qword_64(seg, offset.wrapping_add(16), val.ymm64u(2))?;
        self.write_virtual_qword_64(seg, offset.wrapping_add(24), val.ymm64u(3))?;
        Ok(())
    }

    // ===== Mode-dispatching wrappers for ymmword =====

    pub fn v_read_ymmword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
    ) -> Result<super::xmm::BxPackedYmmRegister> {
        let offset = offset.into();
        // YMM operations are only used in long mode (VEX/EVEX)
        self.read_virtual_ymmword_64(seg, offset)
    }

    pub fn v_write_ymmword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
        val: &super::xmm::BxPackedYmmRegister,
    ) -> Result<()> {
        let offset = offset.into();
        self.write_virtual_ymmword_64(seg, offset, val)
    }

    // =========================================================================
    // 64-bit RMW read functions for byte/word/dword
    // =========================================================================
    // Mirrors read_rmw_virtual_qword_64 pattern but for smaller data sizes.

    /// RMW read byte in 64-bit mode.
    /// Bochs: read_RMW_virtual_byte (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_rmw_virtual_byte_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u8> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.read_rmw_linear_byte(seg, laddr)
    }

    /// RMW read word in 64-bit mode.
    /// Bochs: read_RMW_virtual_word (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_rmw_virtual_word_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u16> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.read_rmw_linear_word(seg, laddr)
    }

    /// RMW read dword in 64-bit mode.
    /// Bochs: read_RMW_virtual_dword (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_rmw_virtual_dword_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u32> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.read_rmw_linear_dword(seg, laddr)
    }
}
