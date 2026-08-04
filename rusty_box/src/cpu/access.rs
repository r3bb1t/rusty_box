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
use crate::{
    config::{BxAddress, BxPhyAddress, BxPtrEquiv},
    memory::memory_rusty_box::bx_guest_ram_span,
};

/// BX_MAX_MEM_ACCESS_LENGTH from Bochs — maximum access size for
/// segment limit checks.  Matches the largest scalar access (qword=8).
const BX_MAX_MEM_ACCESS_LENGTH: u32 = 8;

/// The protection-key allow-mask that applies to a TLB permission bit.
///
/// Bochs tlb.h ANDs the entry's key mask into EVERY hit test, not just into
/// the page walk:
///
/// ```text
/// #define isReadOK(tlbEntry, user)  (tlbEntry->accessBits & (0x01 << user) & rd_pkey[tlbEntry->pkey])
/// #define isWriteOK(tlbEntry, user) (tlbEntry->accessBits & (0x04 << user) & wr_pkey[tlbEntry->pkey])
/// ```
///
/// `needed_bit` says which side applies: bits 0-1 are the read permissions,
/// bits 2-3 the write permissions. Taking the arrays by reference (rather
/// than `&self`) keeps this usable while a `&mut` TLB entry borrowed from the
/// disjoint `dtlb` field is live.
#[inline]
fn pkey_allow(needed_bit: u32, pkey: u32, rd_pkey: &[u32; 16], wr_pkey: &[u32; 16]) -> u32 {
    if needed_bit & 0x0C != 0 {
        wr_pkey[pkey as usize]
    } else {
        rd_pkey[pkey as usize]
    }
}

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

    /// Raise #AC(0) when this access is misaligned and alignment checking is
    /// armed. Bochs `access_read_linear` / `access_write_linear` (access.cc)
    /// perform exactly this test — `alignment_check() && user`, then
    /// `pageOffset & ac_mask` — *before* the TLB lookup, so #AC takes
    /// precedence over #PF.
    ///
    /// `alignment_check_mask` is 0xF only while CS.RPL==3 && CR0.AM &&
    /// EFLAGS.AC (see `handle_alignment_check`), and `user_pl` is forced false
    /// around descriptor and other CPL-0 accesses, so both conditions must
    /// hold. Byte accesses are never checked; vector accesses use the separate
    /// `_aligned` #GP path, matching Bochs access2.cc.
    #[inline(always)]
    pub(super) fn check_alignment(&mut self, laddr: u64, ac_mask: u32) -> Result<()> {
        // Near-always-false: the mask is zero unless a CPL-3 guest has armed
        // both CR0.AM and EFLAGS.AC.
        if self.alignment_check_mask != 0
            && self.user_pl
            && (laddr as u32 & (ac_mask & self.alignment_check_mask)) != 0
        {
            return self.exception(Exception::Ac, 0);
        }
        Ok(())
    }

    // ===== Exception selector: #SS for SS, #GP for others (Bochs int_number) =====

    #[inline]
    pub(super) fn seg_exception(seg: BxSegregs) -> Exception {
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
    fn check_canonical_data(
        &mut self,
        seg: BxSegregs,
        laddr: u64,
        rw: MemoryAccessType,
    ) -> Result<()> {
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
    pub(super) fn write_virtual_checks(
        &mut self,
        seg_idx: usize,
        offset: u32,
        length: u32,
    ) -> bool {
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
    pub(super) fn read_virtual_checks(
        &mut self,
        seg_idx: usize,
        offset: u32,
        length: u32,
    ) -> bool {
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
    // Inline TLB lookup with a host pointer avoids the pinned host-mapping
    // slow path on TLB hits.

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

    /// Prepare a byte RMW translation without reading physical memory.
    /// This lets callers complete permission checks before MMIO callbacks.
    #[inline]
    pub(super) fn prepare_rmw_virtual_byte(&mut self, seg: BxSegregs, offset: u32) -> Result<u64> {
        let laddr = self.agen_write32(seg, offset, 1)? as u64;
        self.prepare_rmw_linear_byte(laddr)?;
        Ok(laddr)
    }

    /// Read phase of a read-modify-write byte access.
    /// Bochs: read_RMW_virtual_byte_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_rmw_virtual_byte(&mut self, seg: BxSegregs, offset: u32) -> Result<u8> {
        self.prepare_rmw_virtual_byte(seg, offset)?;
        Ok(self.read_prepared_rmw_byte())
    }

    /// Prepare a word RMW translation without reading physical memory.
    /// This lets callers complete permission checks before MMIO callbacks.
    #[inline]
    pub(super) fn prepare_rmw_virtual_word(&mut self, seg: BxSegregs, offset: u32) -> Result<u64> {
        let laddr = self.agen_write32(seg, offset, 2)? as u64;
        self.prepare_rmw_linear_word(laddr)?;
        Ok(laddr)
    }

    /// Read phase of a read-modify-write word access.
    /// Bochs: read_RMW_virtual_word_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_rmw_virtual_word(&mut self, seg: BxSegregs, offset: u32) -> Result<u16> {
        self.prepare_rmw_virtual_word(seg, offset)?;
        Ok(self.read_prepared_rmw_word())
    }

    /// Prepare a dword RMW translation without reading physical memory.
    /// This lets callers complete permission checks before MMIO callbacks.
    #[inline]
    pub(super) fn prepare_rmw_virtual_dword(
        &mut self,
        seg: BxSegregs,
        offset: u32,
    ) -> Result<u64> {
        let laddr = self.agen_write32(seg, offset, 4)? as u64;
        self.prepare_rmw_linear_dword(laddr)?;
        Ok(laddr)
    }

    /// Read phase of a read-modify-write dword access.
    /// Bochs: read_RMW_virtual_dword_32 (access.h) — thin wrapper
    #[inline]
    pub fn read_rmw_virtual_dword(&mut self, seg: BxSegregs, offset: u32) -> Result<u32> {
        self.prepare_rmw_virtual_dword(seg, offset)?;
        Ok(self.read_prepared_rmw_dword())
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
            // Bochs access.cc funnels system reads through access_read_linear,
            // which raises a nested #PF (CR2 = laddr, supervisor read error
            // code) on a translation fault — a raw error must never escape to
            // the caller: during exception/interrupt delivery it would
            // escalate straight to #DF where the is_exception_OK table wants
            // a recoverable nested #PF. (The long-mode arm above already
            // nests via translate_data_read; system WRITES nest inside
            // translate_linear_system_write.)
            match self.translate_linear_system_read(laddr) {
                Ok(paddr) => Ok(paddr),
                Err(super::error::CpuError::Memory(mem_err)) => {
                    use super::paging::PageFaultError;
                    let fault = match mem_err {
                        crate::memory::MemoryError::PageProtectionViolation => {
                            PageFaultError::PROTECTION.bits()
                        }
                        crate::memory::MemoryError::PageReservedBitViolation => {
                            PageFaultError::RESERVED.bits() | PageFaultError::PROTECTION.bits()
                        }
                        _ => PageFaultError::NOT_PRESENT.bits(),
                    };
                    self.page_fault(fault, laddr, false, false)?;
                    unreachable!("page_fault always raises")
                }
                Err(e) => Err(e),
            }
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
        let laddr_mask = if self.long_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else {
            0xFFFF_FFFF
        };
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
        let laddr_mask = if self.long_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else {
            0xFFFF_FFFF
        };
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
        let laddr_mask = if self.long_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else {
            0xFFFF_FFFF
        };
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
        let laddr_mask = if self.long_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else {
            0xFFFF_FFFF
        };
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
        let laddr_mask = if self.long_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else {
            0xFFFF_FFFF
        };
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
        let laddr_mask = if self.long_mode() {
            0xFFFF_FFFF_FFFF_FFFF
        } else {
            0xFFFF_FFFF
        };
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
    pub(crate) fn write_virtual_byte_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: u8,
    ) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_byte(seg, laddr, val)
    }

    /// Write a word to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_word (access.h) — thin wrapper: agen + canonical + write_linear_word
    #[inline]
    pub(crate) fn write_virtual_word_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: u16,
    ) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_word(seg, laddr, val)
    }

    /// Write a dword to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_dword (access.h) — thin wrapper: agen + canonical + write_linear_dword
    #[inline]
    pub(crate) fn write_virtual_dword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: u32,
    ) -> Result<()> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.write_linear_dword(seg, laddr, val)
    }

    /// Write a qword to virtual memory in 64-bit mode.
    /// Bochs: write_virtual_qword (access.h) — thin wrapper: agen + canonical + write_linear_qword
    #[inline]
    pub(crate) fn write_virtual_qword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: u64,
    ) -> Result<()> {
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
            for (i, &byte) in bytes[len1..len1 + len2].iter().enumerate() {
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
    pub(super) fn get_host_write_ptr(&mut self, laddr: u64) -> Result<Option<(*mut u8, usize)>> {
        let Some((ptr, remaining, paddr)) = self.resolve_host_write_ptr(laddr)? else {
            return Ok(None);
        };
        self.smc_write_check(paddr, remaining as u32);
        Ok(Some((ptr, remaining)))
    }

    /// Resolve direct writable RAM for a bulk operation without invalidating
    /// cached code yet. The caller must invoke `smc_write_check` with the exact
    /// byte count that was actually written before returning to the CPU loop.
    #[inline]
    pub(super) fn get_host_write_ptr_for_bulk(
        &mut self,
        laddr: u64,
    ) -> Result<Option<(*mut u8, usize, BxPhyAddress)>> {
        self.resolve_host_write_ptr(laddr)
    }

    #[inline]
    fn resolve_host_write_ptr(
        &mut self,
        laddr: u64,
    ) -> Result<Option<(*mut u8, usize, BxPhyAddress)>> {
        // A registered MMIO range may overlap otherwise host-backed RAM.
        // Without per-range clipping, direct bulk writes could bypass a
        // callback later in the page; stay scalar whenever MMIO is active.
        if !self.mmio.is_empty() {
            return Ok(None);
        }
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let mut translated = false;
        loop {
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf
                && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0
                && tlb.host_page_addr != 0
                && !self.mem_host_base.is_null()
            {
                let page_offset = (laddr & 0xFFF) as usize;
                let paddr = tlb.ppf | page_offset as BxPhyAddress;
                let a20_paddr = paddr & self.a20_mask;
                let Some(span) = bx_guest_ram_span(a20_paddr, 1, self.mem_host_len) else {
                    return Ok(None);
                };
                let page_remaining = 0x1000 - page_offset;
                let Some(memory_remaining) = self.mem_host_len.checked_sub(span.start) else {
                    return Ok(None);
                };
                let remaining = page_remaining.min(memory_remaining);
                if remaining == 0 {
                    return Ok(None);
                }
                let ptr = (tlb.host_page_addr as *mut u8).wrapping_add(page_offset);
                return Ok(Some((ptr, remaining, paddr)));
            }
            if translated || !self.cr0.pg() {
                break;
            }
            self.translate_data_write(laddr)?;
            translated = true;
        }

        // Paging-off accesses do not populate the DTLB, but Bochs
        // v2h_write_byte still returns a direct mapping for plain RAM. Use the
        // same RAM ranges as mem_write_byte's handler-aware fast path; VGA,
        // BIOS shadow/ROM, and addresses beyond host RAM stay on the slow path.
        if !self.cr0.pg() {
            let paddr = laddr & self.a20_mask;
            let host_base = self.mem_host_base;
            let plain_ram = (paddr < 0xA0000 || paddr >= 0x100000)
                && bx_guest_ram_span(paddr, 1, self.mem_host_len).is_some();
            if !host_base.is_null() && plain_ram {
                let Some(span) = bx_guest_ram_span(paddr, 1, self.mem_host_len) else {
                    return Ok(None);
                };
                let linear = span.start;
                let page_remaining = 0x1000 - ((paddr as usize) & 0x0fff);
                let Some(memory_remaining) = self.mem_host_len.checked_sub(linear) else {
                    return Ok(None);
                };
                let remaining = page_remaining.min(memory_remaining);
                if remaining == 0 {
                    return Ok(None);
                }
                // The checked translator excludes the PCI hole and maps
                // high GPAs down by the 1 GiB aperture before forming a host
                // address. `wrapping_add` is only pointer arithmetic here;
                // actual dereference remains proven by the checked span.
                let ptr = host_base.wrapping_add(linear);
                return Ok(Some((ptr, remaining, paddr)));
            }
        }

        Ok(None)
    }

    /// Resolve a linear address to a host read pointer via TLB.
    /// Returns (host_ptr, bytes_remaining_in_page) or None on miss.
    /// Bochs: v2h_read_byte (access.h)
    #[inline]
    pub(super) fn get_host_read_ptr(&mut self, laddr: u64) -> Result<Option<(*const u8, usize)>> {
        if !self.mmio.is_empty() {
            return Ok(None);
        }
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let mut translated = false;
        loop {
            let tlb = self.dtlb.get_entry_of(laddr, 0);
            if tlb.lpf == lpf
                && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0
                && tlb.host_page_addr != 0
                && !self.mem_host_base.is_null()
            {
                let page_offset = (laddr & 0xFFF) as usize;
                let paddr = tlb.ppf | page_offset as BxPhyAddress;
                let a20_paddr = paddr & self.a20_mask;
                let Some(span) = bx_guest_ram_span(a20_paddr, 1, self.mem_host_len) else {
                    return Ok(None);
                };
                let page_remaining = 0x1000 - page_offset;
                let Some(memory_remaining) = self.mem_host_len.checked_sub(span.start) else {
                    return Ok(None);
                };
                let remaining = page_remaining.min(memory_remaining);
                if remaining == 0 {
                    return Ok(None);
                }
                let ptr = (tlb.host_page_addr as *const u8).wrapping_add(page_offset);
                return Ok(Some((ptr, remaining)));
            }
            if translated || !self.cr0.pg() {
                break;
            }
            self.translate_data_read(laddr)?;
            translated = true;
        }

        if !self.cr0.pg() {
            let paddr = laddr & self.a20_mask;
            let host_base = self.mem_host_base;
            let plain_ram = (paddr < 0xA0000 || paddr >= 0x100000)
                && bx_guest_ram_span(paddr, 1, self.mem_host_len).is_some();
            if !host_base.is_null() && plain_ram {
                let Some(span) = bx_guest_ram_span(paddr, 1, self.mem_host_len) else {
                    return Ok(None);
                };
                let linear = span.start;
                let page_remaining = 0x1000 - ((paddr as usize) & 0x0FFF);
                let Some(memory_remaining) = self.mem_host_len.checked_sub(linear) else {
                    return Ok(None);
                };
                let remaining = page_remaining.min(memory_remaining);
                if remaining == 0 {
                    return Ok(None);
                }
                return Ok(Some((
                    host_base.wrapping_add(linear) as *const u8,
                    remaining,
                )));
            }
        }

        Ok(None)
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

    /// Apply instrumentation-only write permissions to a prepared RMW
    /// translation before an external side effect.
    #[inline]
    pub(super) fn check_rmw_write_permissions(&mut self, laddr: u64, size: usize) -> Result<()> {
        #[cfg(feature = "instrumentation")]
        {
            if self.address_xlation.pages == 2 {
                self.check_perm_write(
                    laddr,
                    self.address_xlation.paddress1,
                    self.address_xlation.len1 as usize,
                )?;
                let next_page = (laddr | 0x0fff).wrapping_add(1);
                self.check_perm_write(
                    next_page,
                    self.address_xlation.paddress2,
                    self.address_xlation.len2 as usize,
                )?;
            } else {
                self.check_perm_write(laddr, self.address_xlation.paddress1, size)?;
            }
        }
        #[cfg(not(feature = "instrumentation"))]
        let _ = (laddr, size);
        Ok(())
    }

    #[inline]
    pub(super) fn check_rmw_word_write_permissions(&mut self, laddr: u64) -> Result<()> {
        self.check_rmw_write_permissions(laddr, 2)
    }

    #[inline]
    pub(super) fn mmio_read(&mut self, paddr: u64, size: usize) -> Option<u64> {
        if self.mmio.is_empty() {
            return None;
        }
        if let Some(region) = self.mmio.find_mut(paddr) {
            return Some((region.read_cb)(paddr, size));
        }
        None
    }

    #[inline]
    pub(super) fn mmio_write(&mut self, paddr: u64, size: usize, val: u64) -> bool {
        if self.mmio.is_empty() {
            return false;
        }
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
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 1)?;
            let v = unsafe { *host_at_page_offset(host, laddr) };
            #[cfg(feature = "instrumentation")]
            {
                let _buf = [v];
                self.on_lin_access(
                    laddr,
                    paddr_hit,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
            return Ok(v);
        }
        let paddr = self.translate_data_read(laddr)?;
        #[cfg(feature = "instrumentation")]
        self.check_perm_read(laddr, paddr, 1)?;
        if let Some(val) = self.mmio_read(paddr, 1) {
            return Ok(val as u8);
        }
        let v = self.mem_read_byte(paddr);
        #[cfg(feature = "instrumentation")]
        {
            let _buf = [v];
            self.on_lin_access(
                laddr,
                paddr,
                &_buf,
                super::instrumentation::MemAccessRW::Read,
            );
        }
        Ok(v)
    }

    /// Read a word given a pre-computed linear address with cross-page handling.
    /// Bochs: read_linear_word (access2.cc)
    pub(crate) fn read_linear_word(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u16> {
        self.check_alignment(laddr, 1)?;
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 1);
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 2)?;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let v = read_unaligned_u16(ptr);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = v.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr_hit,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
            return Ok(v);
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr, 2)?;
            if let Some(val) = self.mmio_read(paddr, 2) {
                return Ok(val as u16);
            }
            let v = self.mem_read_word(paddr);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = v.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
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
        self.check_alignment(laddr, 3)?;
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 4)?;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let v = read_unaligned_u32(ptr);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = v.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr_hit,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
            return Ok(v);
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr, 4)?;
            if let Some(val) = self.mmio_read(paddr, 4) {
                return Ok(val as u32);
            }
            let v = self.mem_read_dword(paddr);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = v.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
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
        self.check_alignment(laddr, 7)?;
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
            let paddr_hit = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *const u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr_hit, 8)?;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            let v = read_unaligned_u64(ptr);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = v.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr_hit,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
            return Ok(v);
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_read(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_read(laddr, paddr, 8)?;
            if let Some(val) = self.mmio_read(paddr, 8) {
                return Ok(val);
            }
            let v = self.mem_read_qword(paddr);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = v.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Read,
                );
            }
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
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 1)?;
            self.smc_write_check(paddr, 1);
            unsafe { *host_at_page_offset_mut(host, laddr) = val };
            #[cfg(feature = "instrumentation")]
            {
                let _buf = [val];
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
            return Ok(());
        }
        let paddr = self.translate_data_write(laddr)?;
        #[cfg(feature = "instrumentation")]
        self.check_perm_write(laddr, paddr, 1)?;
        if self.mmio_write(paddr, 1, val as u64) {
            return Ok(());
        }
        self.smc_write_check(paddr, 1);
        self.mem_write_byte(paddr, val);
        #[cfg(feature = "instrumentation")]
        {
            let _buf = [val];
            self.on_lin_access(
                laddr,
                paddr,
                &_buf,
                super::instrumentation::MemAccessRW::Write,
            );
        }
        Ok(())
    }

    /// Write a word given a pre-computed linear address with cross-page handling.
    /// Bochs: write_linear_word (access2.cc)
    pub(crate) fn write_linear_word(
        &mut self,
        _seg: BxSegregs,
        laddr: u64,
        val: u16,
    ) -> Result<()> {
        self.check_alignment(laddr, 1)?;
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 1);
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 2)?;
            self.smc_write_check(paddr, 2);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            write_unaligned_u16(ptr, val);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = val.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
            return Ok(());
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 2)?;
            self.smc_write_check(paddr, 2);
            if !self.mmio_write(paddr, 2, val as u64) {
                self.mem_write_word(paddr, val);
            }
            #[cfg(feature = "instrumentation")]
            {
                let _buf = val.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
        } else {
            let bytes = val.to_le_bytes();
            let next_page = (laddr | 0xFFF).wrapping_add(1);
            let p0 = self.translate_data_write(laddr)?;
            let p1 = self.translate_data_write(next_page)?;
            #[cfg(feature = "instrumentation")]
            {
                self.check_perm_write(laddr, p0, 1)?;
                self.check_perm_write(next_page, p1, 1)?;
            }
            self.smc_write_check(p0, 1);
            if !self.mmio_write(p0, 1, u64::from(bytes[0])) {
                self.mem_write_byte(p0, bytes[0]);
            }
            #[cfg(feature = "instrumentation")]
            self.on_lin_access(
                laddr,
                p0,
                &bytes[..1],
                super::instrumentation::MemAccessRW::Write,
            );
            self.smc_write_check(p1, 1);
            if !self.mmio_write(p1, 1, u64::from(bytes[1])) {
                self.mem_write_byte(p1, bytes[1]);
            }
            #[cfg(feature = "instrumentation")]
            self.on_lin_access(
                next_page,
                p1,
                &bytes[1..],
                super::instrumentation::MemAccessRW::Write,
            );
        }
        Ok(())
    }

    /// Write a dword given a pre-computed linear address with cross-page handling.
    /// Bochs: write_linear_dword (access2.cc)
    fn check_gdt_watchpoint(&mut self, _laddr: u64, _val: u64, _size: u32) {
        // Disabled — the GDT 'corruption' was caused by our own diagnostic code
        // (v_read_byte in SYSCALL handler triggering page walks that set A/D bits)
    }

    pub(crate) fn write_linear_dword(
        &mut self,
        _seg: BxSegregs,
        laddr: u64,
        val: u32,
    ) -> Result<()> {
        self.check_alignment(laddr, 3)?;
        self.check_gdt_watchpoint(laddr, val as u64, 4);
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 4)?;
            self.smc_write_check(paddr, 4);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            write_unaligned_u32(ptr, val);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = val.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
            return Ok(());
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 4 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 4)?;
            if self.mmio_write(paddr, 4, val as u64) {
                return Ok(());
            }
            self.smc_write_check(paddr, 4);
            self.mem_write_dword(paddr, val);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = val.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
        } else {
            let bytes = val.to_le_bytes();
            for i in 0..4u64 {
                let p = self.translate_data_write(laddr.wrapping_add(i))?;
                self.smc_write_check(p, 1);
                self.mem_write_byte(p, bytes[i as usize]);
            }
        }
        Ok(())
    }

    /// Write a qword given a pre-computed linear address with cross-page handling.
    /// Bochs: write_linear_qword (access2.cc)
    pub(crate) fn write_linear_qword(
        &mut self,
        _seg: BxSegregs,
        laddr: u64,
        val: u64,
    ) -> Result<()> {
        self.check_alignment(laddr, 7)?;
        self.check_gdt_watchpoint(laddr, val, 8);
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        // DIAGNOSTIC: bypass TLB for writes to test stale-TLB theory
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 8)?;
            self.smc_write_check(paddr, 8);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: pointer valid from TLB/address translation; unaligned access intentional
            write_unaligned_u64(ptr, val);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = val.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
            return Ok(());
        }
        let page_offset = laddr & 0xFFF;
        if page_offset + 8 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            #[cfg(feature = "instrumentation")]
            self.check_perm_write(laddr, paddr, 8)?;
            if self.mmio_write(paddr, 8, val) {
                return Ok(());
            }
            self.smc_write_check(paddr, 8);
            self.mem_write_qword(paddr, val);
            #[cfg(feature = "instrumentation")]
            {
                let _buf = val.to_le_bytes();
                self.on_lin_access(
                    laddr,
                    paddr,
                    &_buf,
                    super::instrumentation::MemAccessRW::Write,
                );
            }
        } else {
            let bytes = val.to_le_bytes();
            for i in 0..8u64 {
                let p = self.translate_data_write(laddr.wrapping_add(i))?;
                self.smc_write_check(p, 1);
                self.mem_write_byte(p, bytes[i as usize]);
            }
        }
        Ok(())
    }

    /// CET shadow-stack: read a dword given a pre-computed linear address.
    /// Bochs access2.cc BX_CPU_C::shadow_stack_read_dword.
    /// `curr_pl` is the privilege level used for SS U/S matching (CPL).
    pub(crate) fn shadow_stack_read_linear_dword(
        &mut self,
        laddr: u64,
        curr_pl: u8,
    ) -> Result<u32> {
        let user = (curr_pl == 3) as u32;
        let lpf = laddr & super::tlb::LPF_MASK;
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        let pkey_mask = self.rd_pkey[tlb.pkey as usize];
        if tlb.lpf == lpf && tlb.is_shadow_stack_read_ok(user, pkey_mask) && tlb.host_page_addr != 0
        {
            let host = tlb.host_page_addr as *const u8;
            let ptr = host_at_page_offset(host, laddr);
            // SAFETY: TLB-validated host pointer; unaligned read OK.
            return Ok(read_unaligned_u32(ptr));
        }
        // Slow path — Bochs access2.cc access_read_linear with BX_SHADOW_STACK_READ.
        let paddr = self.translate_shadow_stack_read(laddr)?;
        Ok(self.mem_read_dword(paddr))
    }

    /// CET shadow-stack: read a qword given a pre-computed linear address.
    /// Bochs access2.cc BX_CPU_C::shadow_stack_read_qword.
    pub(crate) fn shadow_stack_read_linear_qword(
        &mut self,
        laddr: u64,
        curr_pl: u8,
    ) -> Result<u64> {
        let user = (curr_pl == 3) as u32;
        let lpf = laddr & super::tlb::LPF_MASK;
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        let pkey_mask = self.rd_pkey[tlb.pkey as usize];
        if tlb.lpf == lpf && tlb.is_shadow_stack_read_ok(user, pkey_mask) && tlb.host_page_addr != 0
        {
            let host = tlb.host_page_addr as *const u8;
            let ptr = host_at_page_offset(host, laddr);
            return Ok(read_unaligned_u64(ptr));
        }
        let paddr = self.translate_shadow_stack_read(laddr)?;
        Ok(self.mem_read_qword(paddr))
    }

    /// CET shadow-stack: write a dword given a pre-computed linear address.
    /// Bochs access2.cc BX_CPU_C::shadow_stack_write_dword.
    pub(crate) fn shadow_stack_write_linear_dword(
        &mut self,
        laddr: u64,
        curr_pl: u8,
        val: u32,
    ) -> Result<()> {
        let user = (curr_pl == 3) as u32;
        let lpf = laddr & super::tlb::LPF_MASK;
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        let pkey_mask = self.wr_pkey[tlb.pkey as usize];
        if tlb.lpf == lpf
            && tlb.is_shadow_stack_write_ok(user, pkey_mask)
            && tlb.host_page_addr != 0
        {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            self.smc_write_check(paddr, 4);
            let ptr = host_at_page_offset_mut(host, laddr);
            // SAFETY: TLB-validated host pointer; unaligned write OK.
            write_unaligned_u32(ptr, val);
            return Ok(());
        }
        let paddr = self.translate_shadow_stack_write(laddr)?;
        self.smc_write_check(paddr, 4);
        self.mem_write_dword(paddr, val);
        Ok(())
    }

    /// CET shadow-stack: write a qword given a pre-computed linear address.
    /// Bochs access2.cc BX_CPU_C::shadow_stack_write_qword.
    pub(crate) fn shadow_stack_write_linear_qword(
        &mut self,
        laddr: u64,
        curr_pl: u8,
        val: u64,
    ) -> Result<()> {
        let user = (curr_pl == 3) as u32;
        let lpf = laddr & super::tlb::LPF_MASK;
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        let pkey_mask = self.wr_pkey[tlb.pkey as usize];
        if tlb.lpf == lpf
            && tlb.is_shadow_stack_write_ok(user, pkey_mask)
            && tlb.host_page_addr != 0
        {
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            let host = tlb.host_page_addr as *mut u8;
            self.smc_write_check(paddr, 8);
            let ptr = host_at_page_offset_mut(host, laddr);
            write_unaligned_u64(ptr, val);
            return Ok(());
        }
        let paddr = self.translate_shadow_stack_write(laddr)?;
        self.smc_write_check(paddr, 8);
        self.mem_write_qword(paddr, val);
        Ok(())
    }

    /// Read phase of a RMW qword given a pre-computed linear address.
    /// Bochs: read_RMW_linear_qword (access2.cc)
    /// Returns (value, laddr). Caches translation in address_xlation.
    pub(crate) fn read_rmw_linear_qword(
        &mut self,
        _seg: BxSegregs,
        laddr: u64,
    ) -> Result<(u64, u64)> {
        // ---- Inline TLB fast path (Bochs access2.cc) ----
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 7);
        if tlb.lpf == lpf && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0 && tlb.host_page_addr != 0 {
            let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
            let host_addr = tlb.host_page_addr | page_offset;
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            self.smc_write_check(paddr, 8);
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
        self.prepare_rmw_linear_byte(laddr)?;
        Ok(self.read_prepared_rmw_byte())
    }

    /// Prepare the write translation for a byte RMW without reading RAM/MMIO.
    #[inline]
    pub(super) fn prepare_rmw_linear_byte(&mut self, laddr: u64) -> Result<()> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 0);
        if self.mmio.is_empty()
            && tlb.lpf == lpf
            && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0
            && tlb.host_page_addr != 0
        {
            self.address_xlation.pages =
                tlb.host_page_addr | (laddr & 0x0fff) as BxPtrEquiv;
            self.address_xlation.paddress1 =
                tlb.ppf | (laddr & 0x0fff) as BxPhyAddress;
            return Ok(());
        }

        self.address_xlation.pages = 1;
        self.address_xlation.paddress1 = self.translate_data_write(laddr)?;
        Ok(())
    }

    /// Read through the translation prepared by `prepare_rmw_linear_byte`.
    #[inline]
    pub(super) fn read_prepared_rmw_byte(&mut self) -> u8 {
        if self.address_xlation.pages > 2 {
            self.smc_write_check(self.address_xlation.paddress1, 1);
            addr_read_u8(self.address_xlation.pages)
        } else {
            debug_assert_eq!(self.address_xlation.pages, 1);
            let paddr = self.address_xlation.paddress1;
            self.smc_write_check(paddr, 1);
            self.mmio_read(paddr, 1)
                .map_or_else(|| self.mem_read_byte(paddr), |value| value as u8)
        }
    }

    /// Read phase of a RMW word given a pre-computed linear address.
    /// Bochs: read_RMW_linear_word (access2.cc)
    pub(crate) fn read_rmw_linear_word(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u16> {
        self.prepare_rmw_linear_word(laddr)?;
        Ok(self.read_prepared_rmw_word())
    }

    /// Prepare the write translation for a word RMW without reading RAM/MMIO.
    /// Both pages of a split access are translated before any physical side
    /// effect, matching Bochs `access_read_linear`.
    pub(super) fn prepare_rmw_linear_word(&mut self, laddr: u64) -> Result<()> {
        // ---- Inline TLB fast path (Bochs access2.cc) ----
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 1);
        if self.mmio.is_empty()
            && tlb.lpf == lpf
            && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0
            && tlb.host_page_addr != 0
        {
            let page_offset = (laddr & 0xFFF) as BxPtrEquiv;
            let host_addr = tlb.host_page_addr | page_offset;
            let paddr = tlb.ppf | (laddr & 0xFFF) as BxPhyAddress;
            self.address_xlation.pages = host_addr;
            self.address_xlation.paddress1 = paddr;
            return Ok(());
        }

        // ---- Slow path ----
        let page_offset = laddr & 0xFFF;
        if page_offset + 2 <= 0x1000 {
            let paddr = self.translate_data_write(laddr)?;
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = paddr;
        } else {
            let p0 = self.translate_data_write(laddr)?;
            let next_page = (laddr | 0xFFF).wrapping_add(1);
            let p1 = self.translate_data_write(next_page)?;
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = 1;
            self.address_xlation.len2 = 1;
        }
        Ok(())
    }

    /// Read through the translation prepared by `prepare_rmw_linear_word`.
    /// The caller may perform permission checks between preparation and this
    /// physical access.
    pub(super) fn read_prepared_rmw_word(&mut self) -> u16 {
        if self.address_xlation.pages > 2 {
            self.smc_write_check(self.address_xlation.paddress1, 2);
            // SAFETY: `pages > 2` stores the validated host pointer from the
            // TLB path above; the unaligned word access is intentional.
            addr_read_u16(self.address_xlation.pages)
        } else if self.address_xlation.pages == 1 {
            let paddr = self.address_xlation.paddress1;
            self.smc_write_check(paddr, 2);
            self.mmio_read(paddr, 2)
                .map_or_else(|| self.mem_read_word(paddr), |value| value as u16)
        } else {
            debug_assert_eq!(self.address_xlation.pages, 2);
            let p0 = self.address_xlation.paddress1;
            let p1 = self.address_xlation.paddress2;
            self.smc_write_check(p0, 1);
            self.smc_write_check(p1, 1);
            let b0 = self
                .mmio_read(p0, 1)
                .map_or_else(|| self.mem_read_byte(p0), |value| value as u8);
            let b1 = self
                .mmio_read(p1, 1)
                .map_or_else(|| self.mem_read_byte(p1), |value| value as u8);
            u16::from_le_bytes([b0, b1])
        }
    }

    /// Read phase of a RMW dword given a pre-computed linear address.
    /// Bochs: read_RMW_linear_dword (access2.cc)
    pub(crate) fn read_rmw_linear_dword(&mut self, _seg: BxSegregs, laddr: u64) -> Result<u32> {
        self.prepare_rmw_linear_dword(laddr)?;
        Ok(self.read_prepared_rmw_dword())
    }

    /// Prepare the write translation for a dword RMW without reading RAM/MMIO.
    /// Both pages of a split access are translated before any physical side
    /// effect, matching Bochs `access_read_linear`.
    pub(super) fn prepare_rmw_linear_dword(&mut self, laddr: u64) -> Result<()> {
        let lpf = laddr & super::tlb::LPF_MASK;
        let needed_bit = 1u32 << (2 + self.user_pl as u32);
        let tlb = self.dtlb.get_entry_of(laddr, 3);
        if self.mmio.is_empty()
            && tlb.lpf == lpf
            && (tlb.access_bits
                & needed_bit
                & pkey_allow(needed_bit, tlb.pkey, &self.rd_pkey, &self.wr_pkey))
                != 0
            && tlb.host_page_addr != 0
        {
            self.address_xlation.pages =
                tlb.host_page_addr | (laddr & 0x0fff) as BxPtrEquiv;
            self.address_xlation.paddress1 =
                tlb.ppf | (laddr & 0x0fff) as BxPhyAddress;
            return Ok(());
        }

        let page_offset = laddr & 0x0fff;
        if page_offset + 4 <= 0x1000 {
            self.address_xlation.pages = 1;
            self.address_xlation.paddress1 = self.translate_data_write(laddr)?;
        } else {
            let len1 = (0x1000 - page_offset) as u32;
            let next_page = (laddr | 0x0fff).wrapping_add(1);
            let p0 = self.translate_data_write(laddr)?;
            let p1 = self.translate_data_write(next_page)?;
            self.address_xlation.pages = 2;
            self.address_xlation.paddress1 = p0;
            self.address_xlation.paddress2 = p1;
            self.address_xlation.len1 = len1;
            self.address_xlation.len2 = 4 - len1;
        }
        Ok(())
    }

    /// Read through the translation prepared by `prepare_rmw_linear_dword`.
    pub(super) fn read_prepared_rmw_dword(&mut self) -> u32 {
        if self.address_xlation.pages > 2 {
            self.smc_write_check(self.address_xlation.paddress1, 4);
            addr_read_u32(self.address_xlation.pages)
        } else if self.address_xlation.pages == 1 {
            let paddr = self.address_xlation.paddress1;
            self.smc_write_check(paddr, 4);
            self.mmio_read(paddr, 4)
                .map_or_else(|| self.mem_read_dword(paddr), |value| value as u32)
        } else {
            debug_assert_eq!(self.address_xlation.pages, 2);
            let len1 = self.address_xlation.len1 as usize;
            let len2 = self.address_xlation.len2 as usize;
            let p0 = self.address_xlation.paddress1;
            let p1 = self.address_xlation.paddress2;
            let mut bytes = [0u8; 4];

            self.smc_write_check(p0, len1 as u32);
            if let Some(value) = self.mmio_read(p0, len1) {
                bytes[..len1].copy_from_slice(&value.to_le_bytes()[..len1]);
            } else {
                for (index, byte) in bytes[..len1].iter_mut().enumerate() {
                    *byte = self.mem_read_byte(p0 + index as u64);
                }
            }

            self.smc_write_check(p1, len2 as u32);
            if let Some(value) = self.mmio_read(p1, len2) {
                bytes[len1..].copy_from_slice(&value.to_le_bytes()[..len2]);
            } else {
                for (index, byte) in bytes[len1..].iter_mut().enumerate() {
                    *byte = self.mem_read_byte(p1 + index as u64);
                }
            }
            u32::from_le_bytes(bytes)
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
            for (i, &byte) in bytes[len1..len1 + len2].iter().enumerate() {
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
    pub fn v_write_dword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
        val: u32,
    ) -> Result<()> {
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

    pub fn v_write_qword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
        val: u64,
    ) -> Result<()> {
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

    // ===== 64-bit zmmword read/write functions =====

    /// Read a 512-bit ZMM word from virtual memory in 64-bit mode.
    ///
    /// Bochs `read_virtual_zmmword` (access.h) issues one 64-byte access; like
    /// the ymmword path above we compose it from qword accesses, which differ
    /// only in how a segment-limit violation part-way through the operand is
    /// reported.
    pub(super) fn read_virtual_zmmword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<super::xmm::BxPackedZmmRegister> {
        let mut r = super::xmm::BxPackedZmmRegister::default();
        for n in 0..8u64 {
            let q = self.read_virtual_qword_64(seg, offset.wrapping_add(n * 8))?;
            r.set_zmm64u(n as usize, q);
        }
        Ok(r)
    }

    /// Write a 512-bit ZMM word to virtual memory in 64-bit mode.
    pub(super) fn write_virtual_zmmword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
        val: &super::xmm::BxPackedZmmRegister,
    ) -> Result<()> {
        for n in 0..8u64 {
            self.write_virtual_qword_64(seg, offset.wrapping_add(n * 8), val.zmm64u(n as usize))?;
        }
        Ok(())
    }

    // ===== Mode-dispatching wrappers for zmmword =====

    pub fn v_read_zmmword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
    ) -> Result<super::xmm::BxPackedZmmRegister> {
        let offset = offset.into();
        // ZMM operands only exist under EVEX, which rusty_box decodes in long mode.
        self.read_virtual_zmmword_64(seg, offset)
    }

    pub fn v_write_zmmword(
        &mut self,
        seg: BxSegregs,
        offset: impl Into<u64>,
        val: &super::xmm::BxPackedZmmRegister,
    ) -> Result<()> {
        let offset = offset.into();
        self.write_virtual_zmmword_64(seg, offset, val)
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

    /// Prepare a 64-bit-mode byte RMW translation without reading memory.
    #[inline]
    pub(super) fn prepare_rmw_virtual_byte_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<u64> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.prepare_rmw_linear_byte(laddr)?;
        Ok(laddr)
    }

    /// RMW read byte in 64-bit mode.
    /// Bochs: read_RMW_virtual_byte (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_rmw_virtual_byte_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u8> {
        self.prepare_rmw_virtual_byte_64(seg, offset)?;
        Ok(self.read_prepared_rmw_byte())
    }

    /// Prepare a 64-bit-mode word RMW translation without reading memory.
    #[inline]
    pub(super) fn prepare_rmw_virtual_word_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<u64> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.prepare_rmw_linear_word(laddr)?;
        Ok(laddr)
    }

    /// RMW read word in 64-bit mode.
    /// Bochs: read_RMW_virtual_word (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_rmw_virtual_word_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u16> {
        self.prepare_rmw_virtual_word_64(seg, offset)?;
        Ok(self.read_prepared_rmw_word())
    }

    /// Prepare a 64-bit-mode dword RMW translation without reading memory.
    #[inline]
    pub(super) fn prepare_rmw_virtual_dword_64(
        &mut self,
        seg: BxSegregs,
        offset: u64,
    ) -> Result<u64> {
        let laddr = self.get_laddr64(seg as usize, offset);
        self.check_canonical_data(seg, laddr, MemoryAccessType::Write)?;
        self.prepare_rmw_linear_dword(laddr)?;
        Ok(laddr)
    }

    /// RMW read dword in 64-bit mode.
    /// Bochs: read_RMW_virtual_dword (access.h) — thin wrapper
    #[inline]
    pub(crate) fn read_rmw_virtual_dword_64(&mut self, seg: BxSegregs, offset: u64) -> Result<u32> {
        self.prepare_rmw_virtual_dword_64(seg, offset)?;
        Ok(self.read_prepared_rmw_dword())
    }
}

#[cfg(test)]
mod tests {
    use crate::cpu::{
        builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX, crregs::BxCr0,
    };

    #[test]
    fn bulk_host_mapping_rejects_pci_hole_and_translates_above_4g() {
        const GIB: usize = 1 << 30;
        const HIGH_GPA: u64 = 0x1_0000_0100;
        const PCI_HOLE: u64 = 0xC000_0000;

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let fake_base = 0x1000usize as *mut u8;
        // This test exercises pointer selection only; no returned pointer is
        // dereferenced. The synthetic extent avoids a multi-GiB allocation
        // while making the high-GPA translation observable.
        cpu.mem_host_base = fake_base;
        cpu.mem_host_len = 3 * GIB + 0x180;
        cpu.a20_mask = u64::MAX;
        cpu.cr0.remove(BxCr0::PG);

        // Paging-off bulk mapping rejects the PCI aperture outright.
        assert!(cpu.get_host_write_ptr_for_bulk(PCI_HOLE).unwrap().is_none());

        // GPA 4 GiB maps to linear host offset 3 GiB and is clipped at RAM.
        let (ptr, remaining, paddr) = cpu
            .get_host_write_ptr_for_bulk(HIGH_GPA)
            .unwrap()
            .expect("translated high RAM must have a direct bulk mapping");
        assert_eq!(ptr, fake_base.wrapping_add(3 * GIB + 0x100));
        assert_eq!(remaining, 0x80);
        assert_eq!(paddr, HIGH_GPA);

        // The same bounds proof runs for a populated DTLB entry, preventing a
        // stale direct pointer from making PCI-hole RAM reachable.
        let laddr = 0x4100u64;
        let entry = cpu.dtlb.get_entry_of(laddr, 0);
        entry.lpf = laddr & super::super::tlb::LPF_MASK;
        entry.ppf = 0x1_0000_0000;
        entry.access_bits = 1 << 2; // supervisor write
        entry.host_page_addr = fake_base.wrapping_add(3 * GIB) as _;
        let (ptr, remaining, paddr) = cpu
            .get_host_write_ptr_for_bulk(laddr)
            .unwrap()
            .expect("DTLB mapping must retain translated high-RAM bounds");
        assert_eq!(ptr, fake_base.wrapping_add(3 * GIB + 0x100));
        assert_eq!(remaining, 0x80);
        assert_eq!(paddr, 0x1_0000_0100);

        cpu.cr0.insert(BxCr0::PG);
        let laddr = 0x8000u64;
        let entry = cpu.dtlb.get_entry_of(laddr, 0);
        entry.lpf = laddr & super::super::tlb::LPF_MASK;
        entry.ppf = PCI_HOLE;
        entry.access_bits = 1 << 2;
        entry.host_page_addr = fake_base as _;
        assert!(
            cpu.get_host_write_ptr_for_bulk(laddr).unwrap().is_none(),
            "a TLB pointer must not bypass the PCI hole"
        );
    }
}
