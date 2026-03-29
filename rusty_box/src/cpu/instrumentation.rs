//! Bochs-style instrumentation hooks (instrumentation.txt).
//!
//! Implement the `Instrumentation` trait and install via
//! `cpu.set_instrumentation(Box::new(MyInstr))`.
//! All methods have default no-op implementations — override only what you need.
//!
//! Feature-gated by `bx_instrumentation`. When disabled, the trait and all
//! call sites compile to nothing (zero overhead).

/// CPU register snapshot passed to instrumentation callbacks.
#[derive(Debug, Clone, Copy)]
pub struct CpuSnapshot {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub eflags: u32,
    pub icount: u64,
}

/// Bochs-compatible instrumentation trait (instrument/instrumentation.txt).
///
/// All methods are optional — default implementations are no-ops.
/// Override only the hooks you need for your analysis.
#[allow(unused_variables)]
pub trait Instrumentation: Send {
    /// Called before each instruction executes.
    /// Bochs: `bx_instr_before_execution(cpu, bxInstruction_c *i)`
    fn before_execution(&mut self, rip: u64, opcode: u16, ilen: u8, snap: &CpuSnapshot) {}

    /// Called after each instruction executes.
    /// Bochs: `bx_instr_after_execution(cpu, bxInstruction_c *i)`
    fn after_execution(&mut self, rip: u64, opcode: u16, ilen: u8, snap: &CpuSnapshot) {}

    /// Called on HLT instruction.
    /// Bochs: `bx_instr_hlt(cpu)`
    fn hlt(&mut self, rip: u64) {}

    /// Called on MWAIT instruction.
    /// Bochs: `bx_instr_mwait(cpu, addr, len, flags)`
    fn mwait(&mut self, rip: u64, addr: u64, len: u32, flags: u32) {}

    /// Called on hardware interrupt delivery.
    /// Bochs: `bx_instr_hwinterrupt(cpu, vector, cs, rip)`
    fn hwinterrupt(&mut self, vector: u8, cs: u16, rip: u64) {}

    /// Called on exception.
    /// Bochs: `bx_instr_exception(cpu, vector, error_code)`
    fn exception(&mut self, vector: u8, error_code: u32) {}

    /// Called on I/O port read.
    /// Bochs: `bx_instr_inp2(addr, len, val)`
    fn inp(&mut self, port: u16, len: u8, val: u32) {}

    /// Called on I/O port write.
    /// Bochs: `bx_instr_outp(addr, len, val)`
    fn outp(&mut self, port: u16, len: u8, val: u32) {}
}
