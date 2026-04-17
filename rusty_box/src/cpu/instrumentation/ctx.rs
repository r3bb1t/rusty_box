//! `HookCtx` — Unicorn-style hook context giving callbacks full CPU access.
//!
//! Exposed through a type-erased trait (`CpuAccess`) so hook signatures don't
//! need to be generic over `I` / `T`. `BxCpuC<'c, I, T>` implements
//! `CpuAccess` for all `I: BxCpuIdTrait`, `T: Instrumentation`.

use super::types::{InstrAction, X86Reg};

/// Type-erased CPU accessor passed to hooks. Methods are minimally scoped —
/// just enough to read/write registers, read/write memory, request stop, and
/// query the instruction pointer / icount. Kept non-generic so trait methods
/// that take `&mut HookCtx` don't explode into generic parameters.
pub trait CpuAccess {
    // ── Registers ─────────────────────────────────────────────────────────
    fn reg_read(&self, reg: X86Reg) -> u64;
    fn reg_write(&mut self, reg: X86Reg, val: u64);

    // ── Memory ────────────────────────────────────────────────────────────
    /// Read from guest physical memory.
    fn mem_read(&self, addr: u64, buf: &mut [u8]) -> bool;
    /// Write to guest physical memory.
    fn mem_write(&mut self, addr: u64, data: &[u8]) -> bool;
    /// Read from guest virtual memory using current CR3.
    fn virt_read(&self, vaddr: u64, buf: &mut [u8]) -> bool;
    /// Read from guest virtual memory using a specific CR3. Useful for
    /// reading user-space strings after kernel has swapped CR3 (KPTI).
    fn virt_read_with_cr3(&self, vaddr: u64, cr3: u64, buf: &mut [u8]) -> bool;

    // ── Control ───────────────────────────────────────────────────────────
    /// Request the CPU loop to stop at the next trace boundary.
    /// Analogue of Bochs `bx_pc_system.kill_bochs_request`.
    fn stop(&mut self);

    // ── Query ─────────────────────────────────────────────────────────────
    fn rip(&self) -> u64;
    fn icount(&self) -> u64;
    /// CR3 at the time the hook fires. Useful for `virt_read_with_cr3` later.
    fn cr3(&self) -> u64;
}

/// Context object passed to hook callbacks. Thin wrapper around a type-erased
/// CPU reference — the real work happens through `CpuAccess` methods.
pub struct HookCtx<'a> {
    cpu: &'a mut dyn CpuAccess,
}

impl<'a> HookCtx<'a> {
    #[inline]
    pub fn new(cpu: &'a mut dyn CpuAccess) -> Self {
        Self { cpu }
    }

    // ── Delegate everything to the inner CpuAccess impl. ───────────────────

    #[inline] pub fn reg_read(&self, reg: X86Reg) -> u64 { self.cpu.reg_read(reg) }
    #[inline] pub fn reg_write(&mut self, reg: X86Reg, val: u64) { self.cpu.reg_write(reg, val) }

    #[inline] pub fn mem_read(&self, addr: u64, buf: &mut [u8]) -> bool { self.cpu.mem_read(addr, buf) }
    #[inline] pub fn mem_write(&mut self, addr: u64, data: &[u8]) -> bool { self.cpu.mem_write(addr, data) }
    #[inline] pub fn virt_read(&self, vaddr: u64, buf: &mut [u8]) -> bool { self.cpu.virt_read(vaddr, buf) }
    #[inline] pub fn virt_read_with_cr3(&self, vaddr: u64, cr3: u64, buf: &mut [u8]) -> bool {
        self.cpu.virt_read_with_cr3(vaddr, cr3, buf)
    }

    #[inline] pub fn stop(&mut self) { self.cpu.stop() }

    #[inline] pub fn rip(&self) -> u64 { self.cpu.rip() }
    #[inline] pub fn icount(&self) -> u64 { self.cpu.icount() }
    #[inline] pub fn cr3(&self) -> u64 { self.cpu.cr3() }

    /// Read a NUL-terminated string from guest user-space memory, starting at
    /// `vaddr`, translating via `cr3`. Up to `max_len` bytes. Returns empty
    /// string on translation failure (strace convention).
    pub fn read_cstr_user(&self, vaddr: u64, cr3: u64, max_len: usize) -> alloc::string::String {
        use alloc::string::String;
        if vaddr == 0 || max_len == 0 { return String::new(); }
        let mut buf = alloc::vec![0u8; max_len];
        if !self.cpu.virt_read_with_cr3(vaddr, cr3, &mut buf) {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
}

/// Combinator: how to fold multiple hook results into one. `Stop` wins over
/// `Continue`; `Skip` wins over `Continue`; `SkipAndStop` wins over either.
impl InstrAction {
    /// Combine two actions. The more-aggressive action wins.
    #[inline]
    pub fn combine(self, other: Self) -> Self {
        use InstrAction::*;
        match (self, other) {
            (SkipAndStop, _) | (_, SkipAndStop) => SkipAndStop,
            (Skip, Stop) | (Stop, Skip) => SkipAndStop,
            (Skip, _) | (_, Skip) => Skip,
            (Stop, _) | (_, Stop) => Stop,
            _ => Continue,
        }
    }

    #[inline] pub fn is_skip(self) -> bool { matches!(self, InstrAction::Skip | InstrAction::SkipAndStop) }
    #[inline] pub fn is_stop(self) -> bool { matches!(self, InstrAction::Stop | InstrAction::SkipAndStop) }
}
