//! Public types shared by both the BOCHS-style trait API and the
//! Unicorn-style closure-based hook API.
//!
//! This module deliberately avoids any `#[repr(C)]` constraints: the
//! exposed Rust API is the source of truth. C/Python/Lua wrappers will
//! be built in a separate session on top of this clean Rust surface.

use bitflags::bitflags;

// ─────────────────────────── InstrAction ───────────────────────────

/// Return value from `pre_*` hook methods. Controls whether the CPU executes
/// the instruction architecturally, skips it (Unicorn-style intercept),
/// stops after executing, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstrAction {
    /// Execute the instruction architecturally. Default — Bochs-faithful
    /// observation. Hooks that only want to watch return this.
    #[default]
    Continue,

    /// Skip the architectural semantics. For SYSCALL this means no CS/RIP
    /// transition to MSR_LSTAR — RIP advances past the opcode bytes, all
    /// other registers untouched (except any the hook itself modified via
    /// `HookCtx::reg_write`). Use in user-mode emulation where there's no
    /// kernel to service the instruction.
    Skip,

    /// Execute the instruction architecturally, then request the CPU loop
    /// to stop at the next trace boundary. Analogue of Bochs
    /// `kill_bochs_request`.
    Stop,

    /// Skip the architectural semantics AND request stop.
    SkipAndStop,
}


// ─────────────────────────── Hook handle ───────────────────────────

#[cfg(feature = "instrumentation")]
/// Opaque identifier for a registered hook.
///
/// Returned by `Emulator::hook_add_*` methods and consumed by
/// `Emulator::hook_del`. Cannot be constructed externally — eliminates
/// a whole class of "passed wrong integer" bugs.
///
/// `#[repr(transparent)]` keeps the layout identical to `u64` so that
/// future C bindings can cast freely.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use = "HookHandle must be stored to later remove the hook, or explicitly discarded with `let _ = ...`"]
pub struct HookHandle(u64);

#[cfg(feature = "instrumentation")]
impl HookHandle {
    #[inline]
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Raw numeric value — useful for FFI bridges and serialization.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

// ─────────────────────────── HookMask ───────────────────────────

bitflags! {
    /// Bitmask used by the CPU hot path to skip hook dispatch when
    /// no hooks of that category are registered. One bit per
    /// instrumentation category. Predicted-not-taken in the common
    /// case (no instrumentation → branch predictor handles it for free).
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct HookMask: u32 {
        const EXEC         = 1 << 1; // before_execution / after_execution / repeat_iteration / opcode
        const MEM          = 1 << 2; // lin_access / phy_access
        const BRANCH       = 1 << 3; // cnear/ucnear/far_branch
        const INTERRUPT    = 1 << 4; // interrupt() (software int)
        const HW_INTERRUPT = 1 << 5; // hwinterrupt
        const EXCEPTION    = 1 << 6; // exception()
        const IO           = 1 << 7; // inp / inp2 / outp
        const TLB          = 1 << 8; // tlb_cntrl / clflush
        const CACHE        = 1 << 9; // cache_cntrl / prefetch_hint
        const HLT_MWAIT    = 1 << 10; // hlt / mwait
        const CPUID_MSR    = 1 << 11; // cpuid / wrmsr
        const VMEXIT       = 1 << 12;
        const RESET        = 1 << 13;
        const BLOCK       = 1 << 14;
        const INVALID_INSN = 1 << 15;
        const MEM_UNMAPPED = 1 << 16;
        const MEM_PERM    = 1 << 17;
    }
}

impl HookMask {
    /// Any hook category active?
    #[inline]
    pub const fn has_any(self) -> bool {
        !self.is_empty()
    }

    #[inline]
    pub const fn has_exec(self) -> bool {
        self.intersects(Self::EXEC)
    }

    #[inline]
    pub const fn has_mem(self) -> bool {
        self.intersects(Self::MEM)
    }

    #[inline]
    pub const fn has_branch(self) -> bool {
        self.intersects(Self::BRANCH)
    }

    #[inline]
    pub const fn has_interrupt(self) -> bool {
        self.intersects(Self::INTERRUPT)
    }

    #[inline]
    pub const fn has_hw_interrupt(self) -> bool {
        self.intersects(Self::HW_INTERRUPT)
    }

    #[inline]
    pub const fn has_exception(self) -> bool {
        self.intersects(Self::EXCEPTION)
    }

    #[inline]
    pub const fn has_io(self) -> bool {
        self.intersects(Self::IO)
    }

    #[inline]
    pub const fn has_tlb(self) -> bool {
        self.intersects(Self::TLB)
    }

    #[inline]
    pub const fn has_cache(self) -> bool {
        self.intersects(Self::CACHE)
    }

    #[inline]
    pub const fn has_hlt_mwait(self) -> bool {
        self.intersects(Self::HLT_MWAIT)
    }

    #[inline]
    pub const fn has_cpuid_msr(self) -> bool {
        self.intersects(Self::CPUID_MSR)
    }

    #[inline]
    pub const fn has_vmexit(self) -> bool {
        self.intersects(Self::VMEXIT)
    }

    #[inline]
    pub const fn has_reset(self) -> bool {
        self.intersects(Self::RESET)
    }

    #[inline]
    pub const fn has_block(self) -> bool {
        self.intersects(Self::BLOCK)
    }

    #[inline]
    pub const fn has_invalid_insn(self) -> bool {
        self.intersects(Self::INVALID_INSN)
    }

    #[inline]
    pub const fn has_mem_unmapped(self) -> bool {
        self.intersects(Self::MEM_UNMAPPED)
    }

    #[inline]
    pub const fn has_mem_perm(self) -> bool {
        self.intersects(Self::MEM_PERM)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MemPerms: u8 {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC = 1 << 2;
        const ALL = Self::READ.bits() | Self::WRITE.bits() | Self::EXEC.bits();
    }
}

// ─────────────────────────── BOCHS payload enums ───────────────────────────

/// Branch instruction category. Matches BOCHS `BX_INSTR_IS_*` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchType {
    Jmp,
    JmpIndirect,
    Call,
    CallIndirect,
    Ret,
    Iret,
    /// Software interrupt branching to handler (INT n / INT3 / INTO).
    Int,
    Syscall,
    Sysret,
    Sysenter,
    Sysexit,
}

/// TLB control operation. Replaces BOCHS's `(what, new_cr_value)` pair where
/// `new_cr_value` was undefined for several `what` variants. Each variant
/// carries exactly the data BOCHS passes for that operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlbCntrl {
    /// MOV to CR0.
    MovCr0 { new_value: u64 },
    /// MOV to CR3.
    MovCr3 { new_value: u64 },
    /// MOV to CR4.
    MovCr4 { new_value: u64 },
    /// Hardware task switch — payload is the new CR3 after the switch.
    TaskSwitch { new_cr3: u64 },
    /// SMM/VMM context switch — page-root state implicit in the new context.
    ContextSwitch,
    /// INVLPG instruction.
    Invlpg { laddr: u64 },
    /// INVEPT (VMX EPT invalidation).
    Invept { kind: InvEptType },
    /// INVVPID (VMX VPID invalidation — same numeric encoding as INVEPT).
    Invvpid { kind: InvEptType },
    /// INVPCID (PCID invalidation — separate enum because semantics differ).
    Invpcid { kind: InvPcidType },
}

/// INVEPT/INVVPID invalidation kind. Matches BOCHS `BX_INVEPT_INVVPID_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvEptType {
    /// 0 — invalidate one address.
    IndividualAddress,
    /// 1 — invalidate single EPT/VPID context.
    SingleContext,
    /// 2 — invalidate all contexts.
    AllContext,
    /// 3 — invalidate single context, keep globals.
    SingleContextNonGlobal,
}

/// INVPCID invalidation kind. Matches BOCHS `BX_INVPCID_*`. The numeric
/// values overlap with `InvEptType` but the semantics are different — keep
/// the type system honest by distinguishing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvPcidType {
    /// 0 — invalidate one address (non-global only).
    IndividualAddressNonGlobal,
    /// 1 — invalidate single PCID (non-global only).
    SingleContextNonGlobal,
    /// 2 — invalidate all PCIDs including globals.
    AllContext,
    /// 3 — invalidate all PCIDs except globals.
    AllContextNonGlobal,
}

/// Cache control operation. Matches BOCHS `BX_INSTR_INVD` / `BX_INSTR_WBINVD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheCntrl {
    Invd,
    Wbinvd,
}

/// Prefetch hint. Matches BOCHS `BX_INSTR_PREFETCH_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefetchHint {
    Nta,
    T0,
    T1,
    T2,
}

/// Memory access direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemAccessRW {
    Read,
    Write,
    /// Read-then-write (locked RMW).
    RW,
    /// Instruction fetch.
    Execute,
}

/// Memory type (MTRR/PAT). Matches BOCHS `BX_MEMTYPE_*` ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemType {
    Uc,
    Wc,
    /// Reserved value 2 in BOCHS — surfaced for round-trip fidelity.
    Reserved2,
    /// Reserved value 3 in BOCHS — surfaced for round-trip fidelity.
    Reserved3,
    Wt,
    Wp,
    Wb,
    /// Uncacheable (weak, PAT only).
    UcWeak,
    Invalid,
}

impl MemType {
    /// Convert from a BOCHS-style numeric memtype (as exposed inside the
    /// CPU memory pipeline). Unknown values map to `Invalid`.
    #[inline]
    pub const fn from_raw(v: u8) -> Self {
        match v {
            0 => Self::Uc,
            1 => Self::Wc,
            2 => Self::Reserved2,
            3 => Self::Reserved3,
            4 => Self::Wt,
            5 => Self::Wp,
            6 => Self::Wb,
            7 => Self::UcWeak,
            _ => Self::Invalid,
        }
    }
}

/// Reset cause passed to `Instrumentation::reset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetType {
    Hardware,
    Software,
}

/// Code segment operand size mode. Replaces BOCHS C++ `(is32: bool, is64: bool)`
/// pairs — three valid states, no way to accidentally encode the impossible
/// combination "both 32 and 64".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodeSize {
    Bits16,
    Bits32,
    Bits64,
}

bitflags! {
    /// MWAIT extension flags. Mirrors BOCHS `BX_MWAIT_*` constants.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MwaitFlags: u32 {
        /// Wake up on interrupt even when EFLAGS.IF=0.
        const WAKEUP_ON_EVENT_WHEN_INTERRUPT_DISABLE = 0x1;
        /// MWAITX (AMD timed MWAIT, EBX = timeout).
        const TIMED_MWAITX = 0x2;
        /// Monitorless MWAIT (no MONITOR required).
        const MONITORLESS_MWAIT = 0x4;
    }
}

// ─────────────────────────── Hook event types ───────────────────────────

#[cfg(feature = "instrumentation")]
/// Memory hook category — selects which kind of accesses fire the hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemHookType {
    Read,
    Write,
    /// Both read and write.
    ReadWrite,
    /// Instruction fetch.
    Fetch,
    /// All four (read, write, RW, execute).
    All,
}

#[cfg(feature = "instrumentation")]
impl MemHookType {
    #[inline]
    pub(crate) fn matches(self, rw: MemAccessRW) -> bool {
        match self {
            Self::All => true,
            Self::Read => matches!(rw, MemAccessRW::Read | MemAccessRW::RW),
            Self::Write => matches!(rw, MemAccessRW::Write | MemAccessRW::RW),
            Self::ReadWrite => {
                matches!(rw, MemAccessRW::Read | MemAccessRW::Write | MemAccessRW::RW)
            }
            Self::Fetch => matches!(rw, MemAccessRW::Execute),
        }
    }
}

#[cfg(feature = "instrumentation")]
/// I/O port hook category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IoHookType {
    /// IN instructions (port read).
    In,
    /// OUT instructions (port write).
    Out,
    /// Both.
    InOut,
}

#[cfg(feature = "instrumentation")]
impl IoHookType {
    #[inline]
    pub(crate) fn matches(self, rw: MemAccessRW) -> bool {
        match self {
            Self::InOut => true,
            Self::In => matches!(rw, MemAccessRW::Read),
            Self::Out => matches!(rw, MemAccessRW::Write),
        }
    }
}

/// Memory access hook event. Single-pointer payload at the call site.
#[derive(Debug, Clone, Copy)]
pub struct MemHookEvent {
    /// Direction of the access.
    pub access: MemAccessRW,
    /// Linear (virtual) address.
    pub addr: u64,
    /// Access size in bytes.
    pub size: usize,
    /// Value being written (for Write/RW); value loaded (for Read);
    /// `None` for `Execute` fetches and pre-read events with no value yet.
    pub value: Option<u64>,
    /// Physical (post-translation) address.
    pub phys_addr: u64,
    /// Cache type for the region (MTRR/PAT).
    pub memtype: MemType,
}

/// Hardware interrupt delivery event.
#[derive(Debug, Clone, Copy)]
pub struct HwInterruptEvent {
    /// Interrupt vector being delivered.
    pub vector: u8,
    /// Code segment selector at delivery.
    pub cs: u16,
    /// Instruction pointer at delivery.
    pub rip: u64,
}

/// I/O port access event.
#[derive(Debug, Clone, Copy)]
pub struct IoHookEvent {
    /// Port number.
    pub port: u16,
    /// Access size in bytes (1, 2, or 4).
    pub size: u8,
    /// Value (for Out: value being written; for In: value read).
    pub value: u32,
    /// Direction (Read = IN, Write = OUT).
    pub access: MemAccessRW,
}

/// Branch event. Tagged enum — variant determines which fields are present,
/// making invalid states (e.g. far-branch CS on a near branch) unrepresentable.
#[derive(Debug, Clone, Copy)]
pub enum BranchEvent {
    /// Conditional near branch that WAS taken (Jcc).
    CnearTaken { src_rip: u64, dst_rip: u64 },
    /// Conditional near branch that was NOT taken — `fallthrough_rip`
    /// is `src_rip + ilen`.
    CnearNotTaken { src_rip: u64, fallthrough_rip: u64 },
    /// Unconditional near branch (JMP/CALL/RET/near INT).
    Ucnear {
        kind: BranchType,
        src_rip: u64,
        dst_rip: u64,
    },
    /// Far branch (far JMP/CALL/RET, INT with segment change, IRET, SYSCALL,
    /// SYSRET, SYSENTER, SYSEXIT).
    Far {
        kind: BranchType,
        src_cs: u16,
        src_rip: u64,
        dst_cs: u16,
        dst_rip: u64,
    },
}

impl BranchEvent {
    /// Source RIP — present in every variant.
    #[inline]
    pub fn src_rip(&self) -> u64 {
        match self {
            Self::CnearTaken { src_rip, .. }
            | Self::CnearNotTaken { src_rip, .. }
            | Self::Ucnear { src_rip, .. }
            | Self::Far { src_rip, .. } => *src_rip,
        }
    }
}

// ─────────────────────────── Trait hook event structs ───────────────────────────
//
// Every multi-argument trait callback in `Instrumentation` takes an event
// struct by reference. Keeps signatures tight, makes fields self-documenting,
// and lets us extend events without breaking impls. Single-arg or
// primitive-tuple hooks (exception(vector, err), clflush(laddr, paddr), ...)
// stay positional.

use crate::cpu::decoder::Instruction;

/// Payload for `Instrumentation::opcode` — fires when the decoder produces
/// an instruction. `bytes` are the raw opcode bytes as they appeared in
/// memory; `instr` is the decoded form.
#[derive(Debug, Copy, Clone)]
pub struct OpcodeEvent<'a> {
    pub rip: u64,
    pub instr: &'a Instruction,
    pub bytes: &'a [u8],
    pub size: CodeSize,
}

/// Payload for `Instrumentation::mwait`. `len` is the monitor region size
/// hint (not a byte count of data).
#[derive(Debug, Copy, Clone)]
pub struct MwaitEvent {
    pub addr: u64,
    pub len: u32,
    pub flags: MwaitFlags,
}

/// Linear memory access event. `data` is the actual bytes touched by the
/// access — for reads it's the bytes loaded, for writes the bytes stored.
/// Length is implicit (`data.len()`).
#[derive(Debug, Copy, Clone)]
pub struct LinAccess<'a> {
    pub lin: u64,
    pub phy: u64,
    pub data: &'a [u8],
    pub memtype: MemType,
    pub rw: MemAccessRW,
}

/// Physical memory access event (e.g. page-table walks, INVLPG side effects).
/// Same shape as [`LinAccess`] but without a linear address.
#[derive(Debug, Copy, Clone)]
pub struct PhyAccess<'a> {
    pub phy: u64,
    pub data: &'a [u8],
    pub memtype: MemType,
    pub rw: MemAccessRW,
}

/// Payload for `Instrumentation::prefetch_hint`.
#[derive(Debug, Copy, Clone)]
pub struct PrefetchEvent {
    pub what: PrefetchHint,
    pub seg: u8,
    pub offset: u64,
}

/// Payload for `Instrumentation::mem_unmapped` — the access that was about
/// to fault on a not-present page. Return `true` from the hook to suppress
/// the fault.
#[derive(Debug, Copy, Clone)]
pub struct MemUnmapped {
    pub laddr: u64,
    pub size: usize,
    pub rw: MemAccessRW,
}

/// Payload for `Instrumentation::mem_perm_violation` — access denied by the
/// per-page `PagePermissions` bitmap. Return `true` to suppress the fault.
#[derive(Debug, Copy, Clone)]
pub struct MemPermViolation {
    pub laddr: u64,
    pub size: usize,
    pub rw: MemAccessRW,
    pub required: MemPerms,
}

// ─────────────────────────── CpuSnapshot ───────────────────────────

/// On-demand snapshot of CPU register state.
///
/// **Never passed to instrumentation callbacks.** Construct explicitly via
/// `Emulator::cpu_snapshot()` when you need to serialize/log state outside
/// the hot path. Callbacks should use `Emulator::reg_read` between batches
/// (or store CPU state needed by the analysis in the implementation itself).
///
/// Plain Rust struct — no `#[repr]` constraint. C/Python wrappers will
/// expose their own ABI-stable layout if needed.
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
    pub rip: u64,
    pub eflags: u32,
    pub cs: u16,
    pub ss: u16,
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    /// Current Privilege Level (0..=3).
    pub cpl: u8,
    /// Instruction count.
    pub icount: u64,
    // FPU state
    pub fpu_regs: [[u8; 10]; 8],
    pub fpu_sw: u16,
    pub fpu_cw: u16,
    pub mxcsr: u32,
    pub xmm: [[u8; 16]; 16],
}

// ─────────────────────────── X86Reg ───────────────────────────

/// All x86 registers addressable through `Emulator::reg_read` / `reg_write`.
///
/// Plain Rust enum — Rust picks the smallest discriminant. Values are not
/// stable across versions; do not rely on numeric ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum X86Reg {
    // 64-bit GPRs
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
    // 32-bit views (read zero-extended; write replaces low 32 bits and zero-extends per x86-64 rules)
    Eax,
    Ecx,
    Edx,
    Ebx,
    Esp,
    Ebp,
    Esi,
    Edi,
    R8d,
    R9d,
    R10d,
    R11d,
    R12d,
    R13d,
    R14d,
    R15d,
    // 16-bit views (write replaces low 16 bits, upper bits preserved)
    Ax,
    Cx,
    Dx,
    Bx,
    Sp,
    Bp,
    Si,
    Di,
    R8w,
    R9w,
    R10w,
    R11w,
    R12w,
    R13w,
    R14w,
    R15w,
    // 8-bit low / high views
    Al,
    Cl,
    Dl,
    Bl,
    Ah,
    Ch,
    Dh,
    Bh,
    Spl,
    Bpl,
    Sil,
    Dil,
    R8b,
    R9b,
    R10b,
    R11b,
    R12b,
    R13b,
    R14b,
    R15b,
    // Instruction pointer & flags
    Rip,
    Eip,
    Ip,
    Rflags,
    Eflags,
    Flags,
    // Segment selectors
    Cs,
    Ds,
    Es,
    Fs,
    Gs,
    Ss,
    // Segment hidden bases (writable for setup convenience)
    FsBase,
    GsBase,
    // Control registers
    Cr0,
    Cr2,
    Cr3,
    Cr4,
    Cr8,
    // Debug registers
    Dr0,
    Dr1,
    Dr2,
    Dr3,
    Dr6,
    Dr7,
    // Descriptor table registers (limit only — bases are 64-bit, exposed as `*Base`)
    GdtrBase,
    GdtrLimit,
    IdtrBase,
    IdtrLimit,
    LdtrSelector,
    TrSelector,
    // Time-stamp counter (read-only via reg_read; reg_write returns Err for now)
    Tsc,
    // EFER MSR (alias for convenience — also accessible via msr_read/msr_write)
    Efer,
    // FPU registers (x87)
    Fpr0, Fpr1, Fpr2, Fpr3, Fpr4, Fpr5, Fpr6, Fpr7,
    FpSw, FpCw, FpTag,
    // SSE/AVX/AVX-512
    Xmm0, Xmm1, Xmm2, Xmm3, Xmm4, Xmm5, Xmm6, Xmm7,
    Xmm8, Xmm9, Xmm10, Xmm11, Xmm12, Xmm13, Xmm14, Xmm15,
    Mxcsr,
    Ymm0, Ymm1, Ymm2, Ymm3, Ymm4, Ymm5, Ymm6, Ymm7,
    Ymm8, Ymm9, Ymm10, Ymm11, Ymm12, Ymm13, Ymm14, Ymm15,
    Zmm0, Zmm1, Zmm2, Zmm3, Zmm4, Zmm5, Zmm6, Zmm7,
    Zmm8, Zmm9, Zmm10, Zmm11, Zmm12, Zmm13, Zmm14, Zmm15,
    Zmm16, Zmm17, Zmm18, Zmm19, Zmm20, Zmm21, Zmm22, Zmm23,
    Zmm24, Zmm25, Zmm26, Zmm27, Zmm28, Zmm29, Zmm30, Zmm31,
    Opmask0, Opmask1, Opmask2, Opmask3, Opmask4, Opmask5, Opmask6, Opmask7,
}

// ─────────────────────────── EmuStopReason ───────────────────────────

/// Why `Emulator::emu_start` returned. Forces callers to acknowledge the
/// stop cause via exhaustive match — no silent success/failure confusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmuStopReason {
    /// RIP reached the `until` address.
    ReachedUntil,
    /// Instruction count limit reached.
    CountExhausted,
    /// Wall-clock timeout elapsed.
    TimedOut,
    /// `emu_stop()` (or `StopHandle::stop()`) was called.
    Stopped,
    /// CPU entered a halt activity state (HLT/MWAIT) and execution
    /// can't progress without an external interrupt.
    Halted,
    /// CPU entered shutdown state (triple fault).
    Shutdown,
    /// RIP reached one of the configured exit addresses.
    ReachedExit(u64),
}

// ─────────────────────────── CpuSetupMode ───────────────────────────

/// Pre-canned CPU configurations for direct binary emulation. Skip BIOS
/// boot entirely — start the guest CPU in the right mode with flat segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuSetupMode {
    /// 16-bit real mode (default after reset). Use for MBR, DOS COM/EXE,
    /// BIOS hooks, real-mode shellcode.
    RealMode,
    /// 16-bit protected mode. CR0.PE=1, segments base=0 limit=64KB.
    Protected16,
    /// 32-bit flat protected mode. CR0.PE=1, A20 enabled, segments
    /// base=0 limit=4GB, 32-bit default operand/address size.
    /// Use for PE32, ELF32, Win32 shellcode, flat firmware.
    FlatProtected32,
    /// 64-bit long mode with identity-mapped page tables. CR0.PE=1 CR0.PG=1
    /// CR4.PAE=1 EFER.LME=1 EFER.LMA=1 CS.L=1, segments base=0.
    /// Use for PE64, ELF64, x64 shellcode, kernel snapshots.
    FlatLong64,
}


// ─────────────────────────── ExitSet ───────────────────────────

/// Fixed-capacity set of exit addresses (no alloc).
#[derive(Clone)]
pub struct ExitSet {
    addrs: [u64; 64],
    len: u8,
}

impl ExitSet {
    pub const fn new() -> Self { Self { addrs: [0; 64], len: 0 } }
    pub fn set(&mut self, exits: &[u64]) {
        let n = exits.len().min(64);
        self.addrs[..n].copy_from_slice(&exits[..n]);
        self.len = n as u8;
    }
    pub fn clear(&mut self) { self.len = 0; }
    pub fn add(&mut self, addr: u64) -> bool {
        if self.len as usize >= 64 { return false; }
        if self.contains(addr) { return true; }
        self.addrs[self.len as usize] = addr;
        self.len += 1;
        true
    }
    pub fn remove(&mut self, addr: u64) -> bool {
        for i in 0..self.len as usize {
            if self.addrs[i] == addr {
                self.addrs[i] = self.addrs[self.len as usize - 1];
                self.len -= 1;
                return true;
            }
        }
        false
    }
    #[inline]
    pub fn contains(&self, addr: u64) -> bool {
        self.addrs[..self.len as usize].contains(&addr)
    }
    pub fn is_empty(&self) -> bool { self.len == 0 }
}

impl Default for ExitSet {
    fn default() -> Self { Self::new() }
}