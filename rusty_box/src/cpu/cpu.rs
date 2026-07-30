#![allow(non_snake_case, unused_variables, unused_assignments, dead_code)]
#![allow(unused_unsafe)]

use core::{cell::Cell, marker::PhantomData, ptr::NonNull};

use crate::{
    config::{BxAddress, BxPhyAddress, BxPtrEquiv},
    cpu::{
        cpuid::{SVMExtensions, VMXExtensions},
        crregs::BxEfer,
        decoder::{features::X86Feature, BxSegregs, BX_64BIT_REG_RIP},
        rusty_box::MemoryAccessType,
        smm::SMMRAM_Fields,
        tlb::{lpf_of, ppf_of, TLBEntry, Tlb},
        CpuError,
    },
    impl_eflag,
    memory::{BxMemC, CpuMemoryPolicy},
    params::CpuTopology,
};

use super::{
    apic::BxLocalApic,
    cpuid::BxCpuIdTrait,
    cpustats::BxCpuStatistics,
    crregs::{BxCr0, BxCr4, BxDr6, BxDr7, Xcr0, MSR},
    decoder::{Instruction, BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE, BX_XMM_REGISTERS},
    descriptor::{BxGlobalSegmentReg, BxSegmentReg},
    eflags::EFlags,
    i387::{BxPackedRegister, I387},
    icache::BxICache,
    lazy_flags::BxLazyflagsEntry,
    svm::VmcbCache,
    tlb::BxHostpageaddr,
    vmx::{VmcsCache, VmcsMapping, VmxCap},
    xmm::{BxMxcsr, BxZmmReg},
    Result,
};

pub(super) const BX_ASYNC_EVENT_STOP_TRACE: u32 = 1 << 31;
/// Non-architectural scheduler return reason. It must never be delivered by
/// `handle_async_event`; the emulator consumes it only after CPU execution
/// wiring has been torn down.
pub(crate) const BX_ASYNC_EVENT_SCHEDULER_BOUNDARY: u32 = 1 << 30;

// Bochs cpu.h — BX_DTLB_SIZE 2048, BX_ITLB_SIZE 1024 (direct-mapped). Matching
// the upstream sizes keeps rusty's host page-walk and direct-mapped eviction
// profile on the same curve as Bochs, the perf-parity source of truth: a larger
// DTLB changes only host miss rate (guest behaviour is identical), but that host
// divergence is exactly what the wall-clock comparison must not carry.
// CPU_TLB_PIN_DTLB_SLOTS (memory/mod.rs) mirrors BX_DTLB_SIZE and must move with it.
const BX_DTLB_SIZE: usize = 2048;
const BX_ITLB_SIZE: usize = 1024;

#[cfg(feature = "alloc")]
use super::avx::AMX;

use super::tlb::BxMemType;

// Safe register type replacing C-style union. Stores canonical u64 value;
// sub-register views are computed via inline methods. On x86 targets LLVM
// optimises from_le_bytes/to_le_bytes to the same instructions as union access.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct BxGenReg {
    value: u64,
}

impl BxGenReg {
    /// Full 64-bit value (RAX, RCX, ...).
    #[inline(always)]
    pub fn rrx(&self) -> u64 {
        self.value
    }
    #[inline(always)]
    pub fn set_rrx(&mut self, v: u64) {
        self.value = v;
    }

    /// Lower 32-bit dword (EAX, ECX, ...). Does NOT zero-extend on read.
    #[inline(always)]
    pub fn erx(&self) -> u32 {
        self.value as u32
    }
    /// Write lower 32 bits, PRESERVING upper 32 bits.
    /// Callers that need x86-64 zero-extension must also call set_hrx(0).
    #[inline(always)]
    pub fn set_erx(&mut self, v: u32) {
        self.value = (self.value & 0xFFFF_FFFF_0000_0000) | v as u64;
    }

    /// Upper 32 bits (used for zero-extension checks).
    #[inline(always)]
    pub fn hrx(&self) -> u32 {
        (self.value >> 32) as u32
    }
    #[inline(always)]
    pub fn set_hrx(&mut self, v: u32) {
        self.value = (self.value & 0x0000_0000_FFFF_FFFF) | ((v as u64) << 32);
    }

    /// Lower 16-bit word (AX, CX, ...).
    #[inline(always)]
    pub fn rx(&self) -> u16 {
        self.value as u16
    }
    /// Write lower 16 bits, preserving all other bits.
    #[inline(always)]
    pub fn set_rx(&mut self, v: u16) {
        self.value = (self.value & !0xFFFF) | v as u64;
    }

    /// Low byte (AL, CL, ...).
    #[inline(always)]
    pub fn rl(&self) -> u8 {
        self.value as u8
    }
    #[inline(always)]
    pub fn set_rl(&mut self, v: u8) {
        self.value = (self.value & !0xFF) | v as u64;
    }

    /// High byte of low word (AH, CH, ...).
    #[inline(always)]
    pub fn rh(&self) -> u8 {
        (self.value >> 8) as u8
    }
    #[inline(always)]
    pub fn set_rh(&mut self, v: u8) {
        self.value = (self.value & !0xFF00) | ((v as u64) << 8);
    }
}

impl core::fmt::Debug for BxGenReg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", self.value)
    }
}

// <TAG-INSTRUMENTATION_COMMON-BEGIN>

// possible types passed to BX_INSTR_TLB_CNTRL()
#[allow(clippy::upper_case_acronyms)]
pub(super) enum InstrTLBControl {
    MovCr0 = 10,
    MovCr3 = 11,
    MovCr4 = 12,
    TaskSwitch = 13,
    ContextSwitch = 14,
    INVLPG = 15,
    INVEPT = 16,
    INVVPID = 17,
    INVPCID = 18,
}

// possible types passed to BX_INSTR_CACHE_CNTRL()
#[allow(clippy::upper_case_acronyms)]
pub(super) enum InstrCacheControl {
    INVD = 10,
    WBINVD = 11,
}

// possible types passed to BX_INSTR_FAR_BRANCH() and BX_INSTR_UCNEAR_BRANCH()
pub(super) enum InstrBranch {
    Isjmp = 10,
    IsJmpIndirect = 11,
    IsCall = 12,
    IsCallIndirect = 13,
    IsRet = 14,
    IsIret = 15,
    IsInt = 16,
    IsSyscall = 17,
    IsSysret = 18,
    IsSysenter = 19,
    IsSysexit = 20,
    IsUIRET = 21,
}

// possible types passed to BX_INSTR_PREFETCH_HINT()
pub(super) enum InstrPrefetchHint {
    Nta = 0,
    T0 = 1,
    T1 = 2,
    T2 = 3,
    Hint4 = 4,
    Hint5 = 5,
    Hint6 = 6,
    Hint7 = 7,
}

// <TAG-INSTRUMENTATION_COMMON-END>

// passed to internal debugger together with BX_READ/BX_WRITE/BX_EXECUTE/BX_RW
#[allow(clippy::enum_variant_names)]
pub(super) enum AccessReason {
    AccessReasonNotSpecified = 0,
    Pdptr0Access = 1,
    Pdptr1Access,
    Pdptr2Access,
    Pdptr3Access,
    NestedPDPTR0Access,
    NestedPDPTR1Access,
    NestedPDPTR2Access,
    NestedPDPTR3Access,
    PTeAccess,
    PdeAccess,
    PdteAccess,
    Pml4eAccess,
    PML5EAccess,
    NestedPteAccess,
    NestedPdeAccess,
    NestedPdteAccess,
    NestedPML4EAccess,
    NestedPML5EAccess,
    EptPteAccess,
    EptPdeAccess,
    EptPdteAccess,
    EptPml4eAccess,
    EptPml5eAccess, // place holder
    EptSppPteAccess,
    EptSppPdeAccess,
    EptSppPdteaccess,
    EptSppPml4eaccess,
    VmcsAccess,
    ShadowVMCSAccess,
    MSRBitmapAccess,
    IoBitmapAccess,
    VmreadBitmapAccess,
    VmwriteBitmapAccess,
    VMXLoadMsrAccess,
    VMXStoreMsrAccess,
    VMXVAPICAccess,
    VMXPMLWrite,
    VMXPid,
    SMRAMAccess,
}

#[derive(PartialEq, Clone, Debug, Copy)]
pub enum Exception {
    /// Divide error (fault)
    De = 0,
    /// Debug (fault/trap)
    Db = 1,
    /// Breakpoint (trap)
    Bp = 3,
    /// Overflow (trap)
    Of = 4,
    /// BOUND (fault)
    Br = 5,
    Ud = 6,
    Nm = 7,
    Df = 8,
    Ts = 10,
    Np = 11,
    Ss = 12,
    Gp = 13,
    Pf = 14,
    Mf = 16,
    Ac = 17,
    Mc = 18,
    Xm = 19,
    Ve = 20,
    /// Control Protection (fault)
    Cp = 21,
    /// SVM Security Exception (fault)
    Sx = 30,
}

#[allow(clippy::upper_case_acronyms)]
pub(super) enum CpExceptionErrorCode {
    NearRet = 1,
    FarRetIret = 2,
    Endbranch = 3,
    Rstorssp = 4,
    SETSSBSY = 5,
}

pub(super) const BX_CPU_HANDLED_EXCEPTIONS: usize = 32;

#[derive(Clone, Copy)]
pub(super) enum ExceptionClass {
    Trap = 0,
    Fault = 1,
    Abort = 2,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd)]
pub(super) enum CpuMode {
    #[default]
    Ia32Real = 0, // CR0.PE=0                |
    Ia32V8086 = 1,     // CR0.PE=1, EFLAGS.VM=1   | EFER.LMA=0
    Ia32Protected = 2, // CR0.PE=1, EFLAGS.VM=0   |
    LongCompat = 3,    // EFER.LMA = 1, CR0.PE=1, CS.L=0
    Long64 = 4,        // EFER.LMA = 1, CR0.PE=1, CS.L=1
}

pub(super) const BX_MSR_MAX_INDEX: usize = 0x1000;

impl_eflag!(id, 21);
impl_eflag!(vip, 20);
impl_eflag!(vif, 19);
impl_eflag!(ac, 18);
impl_eflag!(vm, 17);
impl_eflag!(rf, 16);
impl_eflag!(nt, 14);
impl_eflag!(if, 9); // Interrupt Flag (bit 9)

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CpuActivityState {
    #[default]
    Active,
    Hlt,
    Shutdown,
    WaitForSipi,
    VmxLastActivityState,
    Mwait,
    MwaitIf,
}

impl From<CpuActivityState> for u8 {
    fn from(value: CpuActivityState) -> Self {
        match value {
            CpuActivityState::Active => 0,
            CpuActivityState::Hlt => 1,
            CpuActivityState::Shutdown => 2,
            CpuActivityState::WaitForSipi | CpuActivityState::VmxLastActivityState => 3,
            CpuActivityState::Mwait => 4,
            CpuActivityState::MwaitIf => 5,
        }
    }
}

#[allow(unused)]
//#[derive(Debug)]
pub struct BxCpuC<'c, I: BxCpuIdTrait, T: super::instrumentation::Instrumentation = ()> {
    pub(super) bx_cpuid: u32,
    pub(super) cpu_topology: CpuTopology,

    pub(super) cpuid: I,

    pub(super) ia_extensions_bitmask: [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE],

    pub(super) vmx_extensions_bitmask: Option<VMXExtensions>,

    pub(super) svm_extensions_bitmask: Option<SVMExtensions>,

    /// General register set
    /// rax: accumulator
    /// rbx: base
    /// rcx: count
    /// rdx: data
    /// rbp: base pointer
    /// rsi: source index
    /// rdi: destination index
    /// esp: stack pointer
    /// r8..r15 x86-64 extended registers
    /// rip: instruction pointer
    /// ssp: shadow stack pointer
    /// tmp: temp register
    /// nil: null register
    pub(crate) gen_reg: [BxGenReg; BX_GENERAL_REGISTERS + 4],

    //
    // 31|30|29|28| 27|26|25|24| 23|22|21|20| 19|18|17|16
    // ==|==|=====| ==|==|==|==| ==|==|==|==| ==|==|==|==
    //  0| 0| 0| 0|  0| 0| 0| 0|  0| 0|ID|VP| VF|AC|VM|RF
    //
    // 15|14|13|12| 11|10| 9| 8|  7| 6| 5| 4|  3| 2| 1| 0
    // ==|==|=====| ==|==|==|==| ==|==|==|==| ==|==|==|==
    //  0|NT| IOPL| OF|DF|IF|TF| SF|ZF| 0|AF|  0|PF| 1|CF
    //
    pub(super) eflags: super::eflags::EFlags, // x86 EFLAGS register

    /// lazy arithmetic flags state
    pub(super) oszapc: BxLazyflagsEntry,

    /// so that we can back up when handling faults, exceptions, etc.
    /// we need to store the value of the instruction pointer, before
    /// each fetch/execute cycle.
    pub(super) prev_rip: BxAddress,
    pub(super) prev_rsp: BxAddress,

    pub(super) prev_ssp: BxAddress,
    pub(super) speculative_rsp: bool,

    /// Instructions retired — Bochs cpu.h `icount` units: one per executed
    /// instruction, one per `repeat()` iteration, one per fast-REP chunk.
    /// REP elements beyond the chunk's first are time, not instructions —
    /// they are charged to [`Self::tick_surplus`] (Bochs string.cc/io.cc
    /// `BX_TICKN(count-1)`), never here.
    pub(crate) icount: u64,
    /// Virtual ticks charged beyond `icount` by fast-REP bulk transfers —
    /// the deferred form of Bochs string.cc/io.cc `BX_TICKN(count-1)`.
    /// `cpu_ticks()` (= icount + tick_surplus) is this CPU's tick-domain
    /// clock; every elapsed-time computation must use it, never raw icount.
    pub(crate) tick_surplus: u64,
    /// `cpu_ticks()` baseline captured by `mark_tick_sync`.
    pub(super) ticks_last_sync: u64,

    /// What events to inhibit at any given time.  Certain instructions
    /// inhibit interrupts, some debug exceptions and single-step traps.
    pub(super) inhibit_mask: u32,
    pub(super) inhibit_icount: u64,

    /// user segment register set
    pub(crate) sregs: [BxSegmentReg; 6],

    // system segment registers
    /// global descriptor table register
    pub(super) gdtr: BxGlobalSegmentReg,
    /// interrupt descriptor table register
    pub(super) idtr: BxGlobalSegmentReg,
    /// local descriptor table register
    pub(super) ldtr: BxSegmentReg,
    /// task register
    pub(super) tr: BxSegmentReg,

    // debug registers DR0-DR7
    /// Dr0-DR3
    pub(super) dr: [BxAddress; 5],
    pub(super) dr6: BxDr6,
    pub(super) dr7: BxDr7,

    /// holds DR6 value (16bit) to be set
    pub(super) debug_trap: u32,

    // Control registers
    pub(crate) cr0: BxCr0,
    pub(super) cr2: BxAddress,
    pub(crate) cr3: BxAddress,

    pub(super) cr4: BxCr4,
    pub(super) cr4_suppmask: u64,

    pub(super) linaddr_width: u8,

    pub(super) efer: BxEfer,
    pub(super) efer_suppmask: u32,

    /// TSC: Time Stamp Counter
    /// Instead of storing a counter and incrementing it every instruction, we
    /// remember the time in ticks that it was reset to zero.  With a little
    /// algebra, we can also support setting it to something other than zero.
    /// Don't read this directly; use get_TSC and set_TSC to access the TSC.
    pub(super) tsc_adjust: i64,

    pub(super) tsc_offset: i64,

    pub(super) xcr0: Xcr0,

    pub(super) xcr0_suppmask: u32,
    pub(super) ia32_xss_suppmask: u32,

    // protection keys
    pub(super) pkru: u32,
    pub(super) pkrs: u32,

    // unpacked protection keys to be tested together with accessBits from TLB
    // the unpacked key is stored in the accessBits format:
    //     bit 5: Execute from User   privilege is OK
    //     bit 4: Execute from System privilege is OK
    //     bit 3: Write   from User   privilege is OK
    //     bit 2: Write   from System privilege is OK
    //     bit 1: Read    from User   privilege is OK
    //     bit 0: Read    from System privilege is OK
    // But only bits 1 and 3 are relevant, all others should be set to '1
    // When protection key prevents all accesses to the page both bits 1 and 3 are cleared
    // When protection key prevents writes to the page bit 1 will be set and 3 cleared
    // When no protection keys are enabled all bits should be set for all keys
    pub(super) rd_pkey: [u32; 16],
    pub(super) wr_pkey: [u32; 16],

    pub(super) uintr: Uintr,

    pub(super) the_i387: I387,

    // Vector register set
    // vmm0-vmmN: up to 32 vector registers
    // vtmp: temp register
    pub(super) vmm: [BxZmmReg; BX_XMM_REGISTERS],
    // Note, didnt check for other features. Basically only aligment changes
    pub(super) mxcsr: BxMxcsr,
    pub(super) mxcsr_mask: u32,

    pub(super) opmask: [BxGenReg; 8],

    pub(super) monitor: MonitorAddr,

    pub(crate) lapic: BxLocalApic,

    /// SMM base register
    pub(super) smbase: u32,

    pub(super) msr: BxRegsMsr,

    pub(super) msrs: [MSR; BX_MSR_MAX_INDEX],

    // Box-allocated under `feature = "alloc"` because AMX carries 8 KiB of
    // tile-data buffers \u2014 too big to inline into every CpuC when most CPUs
    // (e.g. Skylake-X) never need it. Without an allocator (UEFI / no_std)
    // AMX is unsupported by construction; the Option degenerates to a
    // zero-sized never-Some via `Infallible`.
    #[cfg(feature = "alloc")]
    pub(super) amx: Option<alloc::boxed::Box<AMX>>,
    #[cfg(not(feature = "alloc"))]
    pub(super) amx: Option<core::convert::Infallible>,

    pub(super) in_vmx: bool,
    pub(super) in_vmx_guest: bool,
    /// save in_vmx and in_vmx_guest flags when in SMM mode
    pub(super) in_smm_vmx: bool,
    pub(super) in_smm_vmx_guest: bool,
    pub(super) vmcsptr: u64,

    pub(super) vmcs_memtype: BxMemType,

    pub(super) vmxonptr: u64,

    pub(super) vmcs: VmcsCache,
    pub(super) vmx_cap: VmxCap,
    pub(super) vmcs_map: VmcsMapping,

    pub(super) in_svm_guest: bool,
    /// global interrupt enable flag, when zero all external interrupt disabled
    pub(super) svm_gif: bool,
    pub(super) vmcbptr: BxPhyAddress,
    pub(super) vmcbhostptr: BxHostpageaddr,
    pub(super) vmcb_memtype: BxMemType,

    pub(super) vmcb: Option<VmcbCache>,

    pub(super) in_event: bool,

    /// FRED event info word (vector, type, nested, ilen)
    pub(super) fred_event_info: u32,
    /// FRED event data (e.g., CR2 for #PF, DR6 bits for #DB)
    pub(super) fred_event_data: u64,

    pub(super) nmi_unblocking_iret: bool,

    /// 1 if processing external interrupt or exception
    /// or if not related to current instruction,
    /// 0 if current CS:EIP caused exception */
    pub(super) ext: bool,

    pub activity_state: CpuActivityState,

    pub(crate) pending_event: u32,
    pub(crate) event_mask: u32,
    // keep 32-bit because of BX_ASYNC_EVENT_STOP_TRACE
    pub(crate) async_event: u32,

    pub(super) in_smm: bool,
    pub(super) cpu_mode: CpuMode,
    pub(crate) user_pl: bool,

    pub(super) ignore_bad_msrs: bool,

    /// Cached A20 address mask (set at the top of cpu_loop from BxMemC).
    pub(super) a20_mask: u64,

    pub(super) cpu_state_use_ok: u32, // format of BX_FETCH_MODE_*

    // Bochs uses jmp_buf for exception longjmp; we use CpuLoopRestart instead
    pub(super) last_exception_type: i32,

    pub(super) cpuloop_stack_anchor: Option<&'c [u8]>,

    // Perf counters (for diagnosing slowdowns)
    pub(crate) perf_icache_miss: u64,
    pub(crate) perf_prefetch: u64,
    pub(crate) perf_tlb_hit: u64,
    pub(crate) perf_tlb_miss: u64,
    pub(crate) perf_page_walk: u64,
    pub(crate) perf_instructions: u64,

    // Diagnostic counters for handle_async_event interrupt delivery
    #[cfg(debug_assertions)]
    pub(crate) diag_hae_intr_delivered: u64,
    #[cfg(debug_assertions)]
    pub(crate) diag_hae_intr_if_blocked: u64,
    #[cfg(debug_assertions)]
    pub(crate) diag_hae_intr_no_pic: u64,
    #[cfg(debug_assertions)]
    pub(crate) diag_hae_intr_pic_empty: u64,

    /// Exception counts by vector (0=DE, 6=UD, 13=GP, 14=PF, etc.)
    #[cfg(debug_assertions)]
    pub(crate) diag_exception_counts: [u64; 32],
    /// Count of IaError (decoder failures) encountered
    #[cfg(debug_assertions)]
    pub(crate) diag_ia_error_count: u64,
    /// RIP of last IaError
    #[cfg(debug_assertions)]
    pub(crate) diag_ia_error_last_rip: u64,
    /// Count of interrupt() calls by vector (0-255)
    #[cfg(debug_assertions)]
    pub(crate) diag_iac_vectors: [u64; 256],
    /// Count of inject_external_interrupt() calls (emulator-path delivery)
    #[cfg(debug_assertions)]
    pub(crate) diag_inject_ext_intr_count: u64,
    /// Vector histogram for inject_external_interrupt() calls
    #[cfg(debug_assertions)]
    pub(crate) diag_inject_ext_intr_vectors: [u64; 256],
    /// Software INT (INT nn) vector histogram — tracks BIOS calls via int_ib()
    #[cfg(debug_assertions)]
    pub(crate) diag_soft_int_vectors: [u64; 256],
    /// Software INT vector histogram for late calls (icount > 2M, after BIOS POST)
    #[cfg(debug_assertions)]
    pub(crate) diag_soft_int_vectors_late: [u64; 256],
    /// INT 10h AH subfunction histogram (late calls only)
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_ah_hist: [u64; 256],
    /// First 128 chars written via INT 10h AH=0Eh (TTY) — late calls only
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_tty_chars: [u8; 128],
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_tty_count: usize,
    /// Instruction count of first and last INT 10h call (any AH)
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_first_icount: u64,
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_last_icount: u64,
    /// Instruction count of first and last INT 10h AH=0Eh call
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_tty_first_icount: u64,
    #[cfg(debug_assertions)]
    pub(crate) diag_int10h_tty_last_icount: u64,
    /// First HLT in PM capture: icount, EAX-EDI, ESP, EBP, CS, SS, EFLAGS
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_captured: bool,
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_icount: u64,
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_regs: [u32; 8], // EAX, ECX, EDX, EBX, ESP, EBP, ESI, EDI
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_cs: u16,
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_ss: u16,
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_eflags: u32,
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_rip: u32,
    /// Stack snapshot at first PM HLT (16 dwords from ESP)
    #[cfg(debug_assertions)]
    pub(crate) diag_first_pm_hlt_stack: [u32; 16],
    /// RIP ring buffer for tracing last N instructions before HLT
    #[cfg(debug_assertions)]
    pub(super) diag_rip_ring: [u64; 8192],
    /// Opcode ring buffer (parallel to diag_rip_ring)
    #[cfg(debug_assertions)]
    pub(super) diag_opcode_ring: [u16; 256],
    #[cfg(debug_assertions)]
    pub(super) diag_rip_ring_idx: usize,
    /// Current instruction opcode being executed (for corruption detection)
    #[cfg(debug_assertions)]
    pub(super) diag_current_opcode: u16,
    /// Count of GPR64 corruption hits (to limit ring dumps)
    #[cfg(debug_assertions)]
    pub(super) diag_gpr64_corrupt_count: u64,
    /// PM→RM transition count (CR0 PE: 1→0)
    #[cfg(debug_assertions)]
    pub(crate) diag_pm_to_rm_count: u64,
    /// RM→PM transition count (CR0 PE: 0→1)
    #[cfg(debug_assertions)]
    pub(crate) diag_rm_to_pm_count: u64,
    /// Address hit counters: [addr, count] pairs for tracking specific RIP values
    #[cfg(debug_assertions)]
    pub(crate) diag_addr_hits: [(u32, u64); 8],

    /// SYSCALL ring buffer: last 32 syscalls [syscall_nr, arg0 (RDI), icount]
    #[cfg(debug_assertions)]
    pub(crate) diag_syscall_ring: [(u64, u64, u64); 32],
    #[cfg(debug_assertions)]
    pub(crate) diag_syscall_ring_idx: usize,
    #[cfg(debug_assertions)]
    pub(crate) diag_syscall_count: u64,
    /// SYSRET count — compare with diag_syscall_count to find blocked syscalls
    #[cfg(debug_assertions)]
    pub(crate) diag_sysret_count: u64,
    /// When true, log every userspace instruction to stderr (awk trace mode)
    #[cfg(debug_assertions)]
    pub(crate) diag_awk_trace_active: bool,

    // Boundaries of current code page, based on EIP
    pub(super) eip_page_bias: BxAddress,
    pub(super) eip_page_window_size: u32,
    // pub(super) eip_fetch_ptr: &'c [u8],
    pub(super) eip_fetch_ptr: Option<&'c [u8]>,
    pub(super) p_addr_fetch_page: BxPhyAddress, // Guest physical address of current instruction page

    // Boundaries of current stack page, based on ESP
    // Linear address of current stack page
    pub(super) esp_page_bias: BxAddress,
    pub(super) esp_page_window_size: u32,
    pub(super) esp_host_ptr: Option<&'c [u8]>,
    /// Guest physical address of current stack page
    pub(super) p_addr_stack_page: BxPhyAddress,

    pub(super) espPageMemtype: BxMemType,

    pub(super) esp_page_fine_granularity_mapping: u32,

    pub(super) alignment_check_mask: u32,

    pub(super) stats: BxCpuStatistics,

    #[cfg(feature = "bx_debugger")]
    pub(super) watchpoint: BxPhyAddress,
    #[cfg(feature = "bx_debugger")]
    pub(super) break_point: u8,
    #[cfg(feature = "bx_debugger")]
    pub(super) magic_break: u8,
    #[cfg(feature = "bx_debugger")]
    pub(super) stop_reason: u8,
    #[cfg(feature = "bx_debugger")]
    pub(super) trace: bool,
    #[cfg(feature = "bx_debugger")]
    pub(super) trace_reg: bool,
    #[cfg(feature = "bx_debugger")]
    pub(super) trace_mem: bool,
    #[cfg(feature = "bx_debugger")]
    pub(super) mode_break: bool,

    #[cfg(feature = "bx_debugger")]
    pub(super) vmexit_break: bool,

    #[cfg(feature = "bx_debugger")]
    pub(super) show_flag: u32,
    #[cfg(feature = "bx_debugger")]
    pub(super) guard_found: BxGuardFound,

    /// Instrumentation: monomorphized tracer + closure hooks.
    /// With `T = ()` and no closures registered, this is 4 bytes (the bitmask).
    pub(crate) instrumentation: super::instrumentation::InstrumentationRegistry<T>,

    #[cfg(feature = "instrumentation")]
    pub(crate) page_permissions: Option<crate::memory::permissions::PagePermissions>,

    pub(crate) mmio: crate::memory::mmio::MmioRegistry,

    pub(crate) dtlb: Tlb<BX_DTLB_SIZE>,
    pub(super) itlb: Tlb<BX_ITLB_SIZE>,

    pub(super) pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    pub(super) i_cache: BxICache,
    // A2 single dispatch: the former `opcode_handlers` table and per-mpool-slot
    // `i_cache_handlers` fn-ptr pool are gone — the cpu loop dispatches through
    // the canonical `execute_instruction` match, so no handler pointers are
    // cached per decoded instruction.
    pub(super) fetch_mode_mask: super::opcodes_table::FetchModeMask,

    pub(super) address_xlation: AddressXlation,

    /* Now other not so obvious fields */
    pub(super) smram_map: [u32; SMMRAM_Fields::SMRAM_FIELD_LAST as _],

    pub(super) phantom: PhantomData<I>,


    /// Used for direct memory access on TLB hits, bypassing pinned host mapping.
    /// SAFETY: Only valid during cpu_loop when memory is valid.
    pub(crate) mem_host_base: *mut u8,
    /// Usable guest RAM length (not including ROM/bogus).  Physical addresses below this
    /// (and outside VGA/MMIO ranges) can be accessed directly via mem_host_base.
    pub(crate) mem_host_len: usize,

    /// Stable all-CPU pin slice wired for one execution batch. It is a raw
    /// descriptor slice so the currently running CPU is never shared-borrowed.
    active_tlb_pins: *const crate::memory::CpuTlbPin,
    active_tlb_pin_count: usize,
    /// The externally-owned sidecar for this active memory scope. It never
    /// points at CPU storage, so allocator checks do not alias this CPU's
    /// mutable instruction-execution borrow.
    active_tlb_pin_sidecar: Option<NonNull<crate::memory::CpuTlbPin>>,
    /// True when TLB/VMCB state changed without an active external sidecar.
    /// Clean sidecars are maintained slot-by-slot and need no full rescan at
    /// the next bounded CPU memory scope.
    tlb_pin_dirty: Cell<bool>,

    /// Optional memory system pointer (MMIO/ROM handler access), wired during execution.
    ///
    /// This mirrors Bochs' v2h/getHostMemAddr model: the CPU can attempt direct host access
    /// when allowed, and fall back to handler-aware reads/writes when access is vetoed.
    ///
    /// It must only be set for the duration of a CPU execution call and cleared afterwards.
    pub(super) mem_bus: Option<NonNull<crate::memory::BxMemC<'c>>>,

    /// SMP scheduling quantum — Bochs BXPN_SMP_QUANTUM (`cpu: quantum=N`),
    /// range 1-32, default 16. Caps SMP trace length in serve_icache_miss
    /// (Bochs icache.cc) so a CPU returns to the round-robin scheduler after
    /// at most this many instructions. Ignored when cpu_count == 1.
    pub(super) smp_trace_quantum: u8,

    /// Watermark into the machine-wide SMC event queue (memory stub
    /// `smc_seq_next`): this cpu has applied every queued invalidation with a
    /// sequence number below this. Bochs icache.cc `handleSMC` flushes every
    /// cpu synchronously; the queue + watermark defer sibling flushes to the
    /// round-robin slice boundary, which no other cpu can execute before.
    pub(crate) smc_seq_seen: u64,

    /// Optional I/O bus (device port handlers), wired by the emulator during execution.
    ///
    /// This is a raw pointer to avoid borrow checker overhead in the hot path.
    /// It must only be set for the duration of a CPU execution call and cleared afterwards.
    pub(super) io_bus: Option<NonNull<crate::iodev::BxDevicesC>>,

    /// Optional PC system pointer for timer queries (getNumCpuTicksLeftNextEvent).
    /// Wired by the emulator during execution, cleared afterwards.
    pub(super) pc_system_ptr: Option<NonNull<crate::pc_system::BxPcSystemC>>,
    /// `pc_system.time_ticks()` captured when the emulator wired this CPU for
    /// the current batch/round.
    pub(super) pc_system_ticks_at_sync: u64,
    /// CPU tick clock (`cpu_ticks()`) corresponding to `pc_system_ticks_at_sync`.
    pub(super) pc_system_cpu_ticks_at_sync: u64,
    /// A value of one selects live UP time. SMP retains the captured
    /// round-start epoch until the emulator completes the round.
    pub(super) pc_system_tick_denominator: u64,

    /// Debug flags for one-time boot diagnostics (no globals).
    ///
    /// Bit 0: reported unsupported opcode
    /// Bit 1: reported real-mode IVT vector to 0000:0000
    pub(super) boot_debug_flags: u8,
}
/// Clears transient direct-memory wiring even when CPU execution exits through
/// an error path. It holds only a raw pointer to the currently borrowed CPU;
/// the guard itself never aliases CPU state and cannot outlive the call.
struct CpuMemoryWiringGuard<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> {
    cpu: *mut BxCpuC<'c, I, T>,
}

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation>
    CpuMemoryWiringGuard<'c, I, T>
{
    #[inline]
    fn new(cpu: &mut BxCpuC<'c, I, T>) -> Self {
        Self { cpu }
    }
}

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> Drop
    for CpuMemoryWiringGuard<'c, I, T>
{
    fn drop(&mut self) {
        // SAFETY: `new` receives the live CPU borrowed by its enclosing
        // cpu-loop call. The guard is local to that call and drops first.
        unsafe { (*self.cpu).clear_execution_memory_wiring() };
    }
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    pub(super) const BX_ASYNC_EVENT_STOP_TRACE: u32 = 1 << 31;
    /// Persistent sleep sentinel set by enter_sleep_state (HLT/MWAIT).
    /// Matches Bochs proc_ctrl.cc `async_event = 1` — survives the
    /// `&= ~STOP_TRACE` clearing so handle_async_event is called next
    /// outer-loop iteration to check for wake conditions.
    pub(super) const BX_ASYNC_EVENT_SLEEP: u32 = 1;

    #[cfg(feature = "std")]
    pub(crate) const fn snapshot_cpu_id(&self) -> u32 {
        self.bx_cpuid
    }

    #[inline]
    pub fn configure_smp(&mut self, cpu_id: u32, topology: CpuTopology) {
        self.bx_cpuid = cpu_id;
        self.cpu_topology = topology;
        self.lapic.set_id(cpu_id);
        self.lapic.set_bus_cpu_count(topology.cpu_count());
        if self.smp_trace_quantum == 0 {
            // Bochs config.cc BXPN_SMP_QUANTUM default.
            self.smp_trace_quantum = 16;
        }
    }

    /// Set the SMP scheduling quantum — Bochs `cpu: quantum=N`, clamped to
    /// config.h BX_SMP_QUANTUM_MIN..=BX_SMP_QUANTUM_MAX (1..=32).
    #[inline]
    pub fn set_smp_quantum(&mut self, quantum: u32) {
        self.smp_trace_quantum = quantum.clamp(1, 32) as u8;
    }

    /// Configure how the CPUID frequency leaves 0x15/0x16 are reported.
    /// Bochs `cpu: cpuid_freq=` (cpuid.cc get_freq_leaf_15/16); no-op for
    /// models without those leaves.
    #[inline]
    pub fn set_cpuid_freq(&mut self, freq: crate::cpu::cpuid::CpuidFreq, ips: u32) {
        self.cpuid.set_cpuid_freq(freq, ips);
    }

    /// This CPU's tick-domain clock: instructions retired plus the fast-REP
    /// tick surplus. Matches the per-CPU contribution to Bochs
    /// `bx_pc_system.time_ticks()` (BX_TICK1 per instruction plus BX_TICKN
    /// for bulk REP), while `icount` alone matches Bochs `icount`.
    #[inline]
    pub(crate) fn cpu_ticks(&self) -> u64 {
        self.icount.wrapping_add(self.tick_surplus)
    }

    #[inline]
    pub(crate) fn mark_tick_sync(&mut self) {
        self.ticks_last_sync = self.cpu_ticks();
    }

    #[inline]
    pub(crate) fn tick_delta_since_sync(&self) -> u64 {
        self.cpu_ticks().saturating_sub(self.ticks_last_sync)
    }

    /// Synchronize CPU-visible LAPIC INTR state and request a machine
    /// boundary for deferred LAPIC bus, timer, CPU-control, or EOI work.
    #[inline]
    pub(crate) fn sync_lapic_events(&mut self) {
        if self.lapic.intr_pending {
            self.signal_event(Self::BX_EVENT_PENDING_LAPIC_INTR);
            self.lapic.intr_pending = false;
        }
        if self.lapic.has_scheduler_work() {
            self.request_scheduler_boundary();
        }
    }

    #[inline]
    pub(crate) fn request_scheduler_boundary(&mut self) {
        self.async_event |= BX_ASYNC_EVENT_SCHEDULER_BOUNDARY;
    }

    /// Take the scheduler boundary latch after CPU execution wiring has been
    /// cleared. The CPU loop deliberately does not consume this bit.
    #[inline]
    pub(crate) fn take_scheduler_boundary_request(&mut self) -> bool {
        let requested = self.async_event & BX_ASYNC_EVENT_SCHEDULER_BOUNDARY != 0;
        self.async_event &= !BX_ASYNC_EVENT_SCHEDULER_BOUNDARY;
        requested
    }

    #[inline]
    pub fn cpu_topology(&self) -> CpuTopology {
        self.cpu_topology
    }

    // Event bit layout — matches Bochs cpu.h `BX_EVENT_*` exactly.
    // Each bit identifies a single asynchronous event in the
    // `pending_event` / `event_mask` bitmaps the cpu maintains.

    /// Bochs cpu.h `BX_EVENT_NMI`. Masked on NMI delivery, unmasked
    /// on IRET.
    pub(crate) const BX_EVENT_NMI: u32 = 1 << 0;

    /// Bochs cpu.h `BX_EVENT_SMI`. SMI enters System Management Mode.
    pub(crate) const BX_EVENT_SMI: u32 = 1 << 1;

    /// Bochs cpu.h `BX_EVENT_INIT`. INIT is used by multiprocessor
    /// startup (INIT-SIPI-SIPI); it software-resets the CPU at the
    /// next instruction boundary (event.cc handleAsyncEvent).
    pub(crate) const BX_EVENT_INIT: u32 = 1 << 2;

    /// Bochs cpu.h `BX_EVENT_VMX_MONITOR_TRAP_FLAG`. Signalled at
    /// VMENTRY when MONITOR_TRAP_FLAG ctrl is set, or by injected MTF
    /// (type=Other, vector=0); consumed after the next guest
    /// instruction to fire the MTF VMEXIT.
    pub(super) const BX_EVENT_VMX_MONITOR_TRAP_FLAG: u32 = 1 << 4;

    /// Bochs cpu.h `BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED`. Signalled
    /// by the LAPIC tick callback when the preemption-timer fire
    /// deadline is reached; consumed by `handle_async_event`.
    pub(super) const BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED: u32 = 1 << 5;

    /// Bochs cpu.h `BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING`. Signalled
    /// at VMENTRY when `INTERRUPT_WINDOW_VMEXIT` is set in proc-based
    /// controls; consumed by `handle_async_event` whenever
    /// `RFLAGS.IF=1` and external-interrupt inhibition is clear.
    pub(super) const BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING: u32 = 1 << 6;

    /// Bochs cpu.h `BX_EVENT_VMX_VIRTUAL_NMI`. Used in place of
    /// `BX_EVENT_NMI` for masking when the pin-based VIRTUAL_NMI
    /// control is set, so the host-side NMI state is independent of
    /// the guest's virtual-NMI tracking.
    pub(super) const BX_EVENT_VMX_VIRTUAL_NMI: u32 = 1 << 7;

    /// Bochs cpu.h `BX_EVENT_PENDING_VMX_VIRTUAL_INTR`. Pending
    /// virtual-interrupt request (VMX virtual-interrupt-delivery).
    /// Cleared on VMEXIT.
    #[allow(dead_code)]
    pub(super) const BX_EVENT_PENDING_VMX_VIRTUAL_INTR: u32 = 1 << 9;

    /// Bochs cpu.h `BX_EVENT_PENDING_INTR`. External interrupt pending
    /// (PIC int_pin asserted).
    pub(crate) const BX_EVENT_PENDING_INTR: u32 = 1 << 10;

    /// Bochs cpu.h `BX_EVENT_PENDING_LAPIC_INTR`. LAPIC interrupt
    /// pending.
    pub(crate) const BX_EVENT_PENDING_LAPIC_INTR: u32 = 1 << 11;

    /// Bochs cpu.h `BX_EVENT_PENDING_UINTR`. User-level interrupt
    /// pending.
    pub(super) const BX_EVENT_PENDING_UINTR: u32 = 1 << 12;

    /// Bochs cpu.h `BX_EVENT_VMX_VTPR_UPDATE`. Signalled when the
    /// virtual-TPR shadow is mutated and the next instruction
    /// boundary needs to re-evaluate TPR-threshold VMEXIT.
    /// Cleared on VMEXIT.
    #[allow(dead_code)]
    pub(super) const BX_EVENT_VMX_VTPR_UPDATE: u32 = 1 << 13;

    /// Bochs cpu.h `BX_EVENT_VMX_VEOI_UPDATE`. Signalled by virtual
    /// EOI on the virtual-APIC page; consumed to deliver the
    /// virtualized-EOI VMEXIT. Cleared on VMEXIT.
    #[allow(dead_code)]
    pub(super) const BX_EVENT_VMX_VEOI_UPDATE: u32 = 1 << 14;

    /// Bochs cpu.h `BX_EVENT_VMX_VIRTUAL_APIC_WRITE`. Signalled when
    /// a write hits the virtual-APIC page so the next instruction
    /// boundary can deliver the APIC-WRITE VMEXIT. Cleared on VMEXIT.
    #[allow(dead_code)]
    pub(super) const BX_EVENT_VMX_VIRTUAL_APIC_WRITE: u32 = 1 << 15;

    /// Returns a mutable raw pointer to the Local APIC for cross-module wiring.
    /// Used by emulator.rs to wire I/O APIC → LAPIC interrupt delivery.
    pub(crate) fn lapic_ptr_mut(&mut self) -> *mut crate::cpu::apic::BxLocalApic {
        &mut self.lapic as *mut _
    }

    /// Check LAPIC IRR/ISR for a specific vector (immutable access for diagnostics).
    pub(crate) fn lapic_vector_state(&self, vector: u8) -> (bool, bool) {
        self.lapic.vector_state(vector)
    }

    /// Check if the LAPIC has a pending interrupt (immutable access).
    pub(crate) fn lapic_has_intr(&self) -> bool {
        self.lapic.intr
    }
}


impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    #[inline]
    pub(crate) fn active_tlb_pins(&self) -> &[crate::memory::CpuTlbPin] {
        if self.active_tlb_pins.is_null() {
            &[]
        } else {
            // SAFETY: cpu_loop wires this stable slice for its duration.
            unsafe {
                core::slice::from_raw_parts(self.active_tlb_pins, self.active_tlb_pin_count)
            }
        }
    }
    /// Copy every currently valid host mapping into an external pin sidecar.
    ///
    /// Full rescans are needed only when the sidecar is created or CPU state
    /// changed outside a wired scope. Wired execution publishes each mapping
    /// install or invalidation synchronously.
    #[inline]
    pub(crate) fn refresh_tlb_pin(&self, pin: &crate::memory::CpuTlbPin) {
        pin.clear_tlb_hosts();
        for slot in 0..BX_DTLB_SIZE {
            pin.set_dtlb_host(slot, self.dtlb.pinned_host_page(slot));
        }
        for slot in 0..BX_ITLB_SIZE {
            pin.set_itlb_host(slot, self.itlb.pinned_host_page(slot));
        }
        pin.set_vmcb_host(if self.in_svm_guest {
            self.vmcbhostptr as usize
        } else {
            0
        });
        match self.eip_fetch_ptr {
            Some(slice) => pin.set_fetch_window(slice.as_ptr() as usize, slice.len()),
            None => pin.set_fetch_window(0, 0),
        }
        self.tlb_pin_dirty.set(false);
    }

    /// Refresh a sidecar only when CPU state changed outside a wired scope.
    #[inline]
    pub(crate) fn refresh_tlb_pin_if_dirty(&self, pin: &crate::memory::CpuTlbPin) {
        if self.tlb_pin_dirty.get() {
            self.refresh_tlb_pin(pin);
        }
    }
    /// Returns the sidecar owned by the current wired CPU memory scope.
    #[inline]
    fn active_tlb_pin_sidecar(&self) -> Option<&crate::memory::CpuTlbPin> {
        self.active_tlb_pin_sidecar.map(|pin| {
            // SAFETY: `wire_memory_access` stores a descriptor belonging to
            // the stable caller-provided pin slice and `clear_memory_access`
            // clears it before that scope ends.
            unsafe { pin.as_ref() }
        })
    }

    /// Publish the pin sidecar after a FULL TLB flush (`dtlb.flush()` +
    /// `itlb.flush()`), where every entry is now invalid.
    ///
    /// After a full flush `refresh_tlb_pin`'s per-slot rescan would write zero
    /// to all `BX_DTLB_SIZE + BX_ITLB_SIZE` slots (every `pinned_host_page`
    /// returns 0 for an invalid entry), so a single `clear_tlb_hosts` memset
    /// reproduces it far more cheaply. The caller must have already invalidated
    /// the prefetch queue, so the (now-empty) fetch window is correctly zeroed
    /// too. `clear_tlb_hosts` also zeros `vmcb_host`; the flush does not change
    /// SVM state, so an active guest's VMCB backing must be re-pinned — omitting
    /// this would UNDER-pin it (use-after-free). Over-pinning is always safe;
    /// under-pinning is the bug this guards against.
    #[inline]
    pub(crate) fn clear_active_tlb_pin_hosts(&self) {
        if let Some(pin) = self.active_tlb_pin_sidecar() {
            pin.clear_tlb_hosts();
            if self.in_svm_guest {
                pin.set_vmcb_host(self.vmcbhostptr as usize);
            }
            self.tlb_pin_dirty.set(false);
        } else {
            self.tlb_pin_dirty.set(true);
        }
    }

    /// Non-global TLB flush (Bochs paging.cc `TLB_flushNonGlobal`) with pin
    /// publication fused into the invalidation walk (Track B). Clears only the
    /// pin slots the walk actually invalidates, then republishes the O(1) VMCB
    /// and fetch-window pins — producing exactly the sidecar state a full
    /// `refresh_tlb_pin` rescan would, at O(entries invalidated) instead of
    /// O(`BX_DTLB_SIZE` + `BX_ITLB_SIZE`).
    ///
    /// Correctness rests on the pin invariant: a kept (global) entry's slot
    /// already equals its live host pointer because every install publishes via
    /// `sync_dtlb_pin_slot` / `sync_itlb_pin_slot`, so leaving it untouched
    /// matches the fresh rescan. This never under-pins: every slot cleared here
    /// belongs to an entry the same call just invalidated. Over-pinning is safe.
    #[inline]
    pub(crate) fn flush_non_global_and_publish_pin(&mut self) {
        match self.active_tlb_pin_sidecar {
            Some(pin_ptr) => {
                // SAFETY: the sidecar lives in the caller-owned pin slice, not
                // inside the CPU, so it is separately addressable from
                // self.dtlb/itlb. `as_ref` yields a reference decoupled from the
                // `&mut self` borrow, which is what lets the mutable TLB walks
                // below publish into it. The single-threaded CPU/memory scope
                // serializes all sidecar mutation.
                let pin: &crate::memory::CpuTlbPin = unsafe { pin_ptr.as_ref() };
                self.dtlb
                    .flush_non_global_publishing(|slot| pin.set_dtlb_host(slot, 0));
                self.itlb
                    .flush_non_global_publishing(|slot| pin.set_itlb_host(slot, 0));
                self.republish_scalar_pins(pin);
                self.tlb_pin_dirty.set(false);
            }
            None => {
                self.dtlb.flush_non_global();
                self.itlb.flush_non_global();
                self.tlb_pin_dirty.set(true);
            }
        }
    }

    /// Single-page INVLPG (Bochs paging.cc `TLB_invlpg`) with pin publication
    /// fused into the invalidation (Track B). The non-split path touches at most
    /// one DTLB and one ITLB slot; the split-large path publishes every slot its
    /// scan clears. See `flush_non_global_and_publish_pin` for the invariant.
    #[inline]
    pub(crate) fn invlpg_and_publish_pin(&mut self, laddr: BxAddress) {
        match self.active_tlb_pin_sidecar {
            Some(pin_ptr) => {
                // SAFETY: see `flush_non_global_and_publish_pin`.
                let pin: &crate::memory::CpuTlbPin = unsafe { pin_ptr.as_ref() };
                self.dtlb
                    .invlpg_publishing(laddr, |slot| pin.set_dtlb_host(slot, 0));
                self.itlb
                    .invlpg_publishing(laddr, |slot| pin.set_itlb_host(slot, 0));
                self.republish_scalar_pins(pin);
                self.tlb_pin_dirty.set(false);
            }
            None => {
                self.dtlb.invlpg(laddr);
                self.itlb.invlpg(laddr);
                self.tlb_pin_dirty.set(true);
            }
        }
    }

    /// Republish the O(1) non-TLB host pins (VMCB backing + bounded fetch
    /// window) exactly as `refresh_tlb_pin` does, so a fused flush leaves the
    /// sidecar byte-identical to a full rescan for those fields. A TLB flush
    /// does not change SVM state, but rewriting them is O(1) and removes any
    /// dependence on a pre-existing VMCB/fetch-window invariant.
    #[inline]
    fn republish_scalar_pins(&self, pin: &crate::memory::CpuTlbPin) {
        pin.set_vmcb_host(if self.in_svm_guest {
            self.vmcbhostptr as usize
        } else {
            0
        });
        match self.eip_fetch_ptr {
            Some(slice) => pin.set_fetch_window(slice.as_ptr() as usize, slice.len()),
            None => pin.set_fetch_window(0, 0),
        }
    }

    /// Publish one freshly installed DTLB host pointer without re-copying the
    /// full 5120-entry sidecar on each page walk.
    #[inline]
    pub(crate) fn sync_dtlb_pin_slot(&self, laddr: BxAddress, len: u32) {
        if let Some(pin) = self.active_tlb_pin_sidecar() {
            let slot = self.dtlb.get_index_of(laddr, len);
            pin.set_dtlb_host(slot, self.dtlb.pinned_host_page(slot));
        } else {
            self.tlb_pin_dirty.set(true);
        }
    }

    /// Discard a colliding DTLB mapping before a page walk can allocate
    /// backing, then immediately publish the removal to the eviction sidecar.
    #[inline]
    pub(crate) fn invalidate_dtlb_pin_slot(&mut self, laddr: BxAddress, len: u32) {
        self.dtlb.invalidate_slot(laddr, len);
        self.sync_dtlb_pin_slot(laddr, len);
    }

    /// Publish one freshly installed ITLB host pointer.
    #[inline]
    pub(crate) fn sync_itlb_pin_slot(&self, laddr: BxAddress, len: u32) {
        if let Some(pin) = self.active_tlb_pin_sidecar() {
            let slot = self.itlb.get_index_of(laddr, len);
            pin.set_itlb_host(slot, self.itlb.pinned_host_page(slot));
        } else {
            self.tlb_pin_dirty.set(true);
        }
    }
    /// Discard a colliding ITLB mapping before a miss can allocate backing
    /// memory, then immediately publish the removal to the eviction sidecar.
    #[inline]
    pub(crate) fn invalidate_itlb_pin_slot(&mut self, laddr: BxAddress, len: u32) {
        self.eip_fetch_ptr = None;
        self.itlb.invalidate_slot(laddr, len);
        self.sync_itlb_pin_slot(laddr, len);
        self.sync_fetch_window_pin();
    }

    /// Publish the current bounded instruction-fetch window to the eviction
    /// sidecar. `eip_fetch_ptr` (Bochs cpu.cc `eipFetchPtr`) may reference a
    /// sub-page resident block that no ITLB slot pins; without this interval
    /// a data access could evict the backing block and leave the retained
    /// fetch pointer dangling.
    #[inline]
    pub(crate) fn sync_fetch_window_pin(&self) {
        if let Some(pin) = self.active_tlb_pin_sidecar() {
            match self.eip_fetch_ptr {
                Some(slice) => pin.set_fetch_window(slice.as_ptr() as usize, slice.len()),
                None => pin.set_fetch_window(0, 0),
            }
        } else {
            self.tlb_pin_dirty.set(true);
        }
    }


    /// Publish an SVM guest/VMCB host-pointer transition.
    #[inline]
    pub(crate) fn sync_vmcb_pin(&self) {
        if let Some(pin) = self.active_tlb_pin_sidecar() {
            pin.set_vmcb_host(if self.in_svm_guest {
                self.vmcbhostptr as usize
            } else {
                0
            });
        } else {
            self.tlb_pin_dirty.set(true);
        }
    }
    pub fn is_canonical(&self, addr: BxAddress) -> bool {
        Self::is_canonical_to_width(addr, self.linaddr_width.into())
    }

    #[inline]
    pub fn is_canonical_to_width(addr: u64, width: u32) -> bool {
        // Reinterpret addr as signed, shift right (arithmetic shift),
        // add 1, cast back to unsigned and compare with 2.
        let signed = (addr as i64) >> (width - 1);
        let jumped = signed.wrapping_add(1);
        (jumped as u64) < 2
    }

    pub(super) fn bx_cpuid_support_isa_extension(&self, feature: X86Feature) -> bool {
        let feature_as_usize = feature as usize;
        (self.ia_extensions_bitmask[feature_as_usize / 32] & (1 << (feature_as_usize % 32))) != 0
    }

    pub(super) fn real_mode(&self) -> bool {
        self.cpu_mode == CpuMode::Ia32Real
    }

    /// Protected mode (NOT v8086, NOT real) — matches Bochs BX_CPU_C::protected_mode()
    /// Bochs: cpu_mode >= BX_MODE_IA32_PROTECTED (includes Protected, LongCompat, Long64)
    pub(super) fn protected_mode(&self) -> bool {
        self.cpu_mode >= CpuMode::Ia32Protected
    }

    /// Borrow the optional AMX state. Always `None` on no-alloc builds: the
    /// `Option` storage degenerates to `Option<Infallible>` so this returns
    /// `None` without any cfg-gating at the call site.
    #[cfg(feature = "alloc")]
    #[inline]
    pub(super) fn amx_ref(&self) -> Option<&crate::cpu::avx::AMX> {
        self.amx.as_deref()
    }
    #[cfg(not(feature = "alloc"))]
    #[inline]
    pub(super) fn amx_ref(&self) -> Option<&crate::cpu::avx::AMX> {
        None
    }

    /// Mutable counterpart of [`amx_ref`]. Always `None` on no-alloc builds.
    #[cfg(feature = "alloc")]
    #[inline]
    pub(super) fn amx_mut(&mut self) -> Option<&mut crate::cpu::avx::AMX> {
        self.amx.as_deref_mut()
    }
    #[cfg(not(feature = "alloc"))]
    #[inline]
    pub(super) fn amx_mut(&mut self) -> Option<&mut crate::cpu::avx::AMX> {
        None
    }

    pub(super) fn bx_write_opmask(&mut self, index: usize, val_64: u64) {
        self.opmask[index].set_rrx(val_64);
    }

    // ── Debug trap bits (DR6 bits set by CPU) ──
    // Bochs crregs.h
    pub(super) const BX_DEBUG_TRAP_HIT: u32 = 1 << 12; // internal "a breakpoint fired" flag
    pub(super) const BX_DEBUG_SINGLE_STEP_BIT: u32 = 1 << 14; // BS flag in DR6 (bit 14)
    pub(super) const BX_DEBUG_TRAP_TASK_SWITCH_BIT: u32 = 0x8000; // BT flag in DR6

    // ── Hardware debug (DR7 R/W field) breakpoint type ──
    // Bochs cpu.h enum: instruction-execution breakpoint (R/W == 00b).
    // Data (01b/11b) and I/O watchpoints are not wired into the memory
    // access paths yet, so only the instruction type is defined here.
    pub(super) const BX_HW_DEBUG_INSTRUCTION: u32 = 0x00;

    // ── DR7 local breakpoint enable bits mask ──
    // Bits L0(0), L1(2), L2(4), L3(6), LE(8) = 0x155
    pub(super) const DR7_LOCAL_ENABLE_MASK: u32 = 0x0000_0155;

    // ── Interrupt inhibition (MOV SS / POP SS) ──
    // Bochs cpu.h
    pub(super) const BX_INHIBIT_INTERRUPTS: u32 = 0x01;
    pub(super) const BX_INHIBIT_DEBUG: u32 = 0x02;
    pub(super) const BX_INHIBIT_INTERRUPTS_BY_MOVSS: u32 = 0x01 | 0x02;

    /// Set interrupt inhibition mask for the next instruction boundary.
    /// Bochs event.cc: prevents double MOV SS from extending the window.
    pub(super) fn inhibit_interrupts(&mut self, mask: u32) {
        // Bochs guard: if mask is MOVSS and we're already inhibiting by MOVSS,
        // don't reset the window. A second MOV SS doesn't extend inhibition.
        if mask != Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS
            || !self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS_BY_MOVSS)
        {
            self.inhibit_mask = mask;
            self.inhibit_icount = self.icount + 1;
        }
    }

    /// Check if interrupts of the given type are currently inhibited.
    /// Bochs event.cc: `(inhibit_mask & mask) == mask` — ALL bits must match.
    pub(crate) fn interrupts_inhibited(&self, mask: u32) -> bool {
        self.icount <= self.inhibit_icount && (self.inhibit_mask & mask) == mask
    }
}

#[derive(Debug, Default, Copy, Clone)]
pub(crate) struct AddressXlation {
    /// The address offset after resolution
    pub(crate) rm_addr: BxPhyAddress,
    /// physical address after translation of 1st len1 bytes of data
    pub(crate) paddress1: BxPhyAddress,
    /// physical address after translation of 2nd len2 bytes of data
    pub(crate) paddress2: BxPhyAddress,
    /// Number of bytes in page 1
    pub(crate) len1: u32,
    // Number of bytes in page 2
    pub(crate) len2: u32,
    /// Number of pages access spans (1 or 2).  Also used
    /// for the case when a native host pointer is
    /// available for the R-M-W instructions.  The host
    /// pointer is stuffed here.  Since this field has
    /// to be checked anyways (and thus cached), if it
    /// is greated than 2 (the maximum possible for
    /// normal cases) it is a native pointer and is used
    /// for a direct write access.
    pub(crate) pages: BxPtrEquiv,
    /// memory type of the page 1
    pub(crate) memtype1: BxMemType,
    /// memory type of the page 1
    pub(crate) memtype2: BxMemType,
}

impl AddressXlation {
    /// Write a `u8` directly via the cached host pointer in `pages`.
    ///
    /// # Safety contract (encapsulated)
    /// Caller guarantees `self.pages > 2`, meaning it holds a valid host pointer
    /// set during the read phase of a read-modify-write TLB hit.
    #[inline(always)]
    pub(super) fn write_pages_u8(&mut self, val: u8) {
        unsafe { *(self.pages as *mut u8) = val };
    }

    /// Write a `u16` (unaligned) directly via the cached host pointer in `pages`.
    #[inline(always)]
    pub(super) fn write_pages_u16(&mut self, val: u16) {
        unsafe { (self.pages as *mut u16).write_unaligned(val) };
    }

    /// Write a `u32` (unaligned) directly via the cached host pointer in `pages`.
    #[inline(always)]
    pub(super) fn write_pages_u32(&mut self, val: u32) {
        unsafe { (self.pages as *mut u32).write_unaligned(val) };
    }
}

#[derive(Debug, Default)]
pub(super) struct PdptrCache {
    pub(crate) entry: [u64; 4],
}

#[derive(Debug, Default)]
pub struct BxRegsMsr {
    pub(crate) apicbase: BxPhyAddress,

    // SYSCALL/SYSRET instruction msr's
    pub(crate) star: u64,

    pub(crate) lstar: u64,
    pub(crate) cstar: u64,
    pub(crate) fmask: u32,
    pub(crate) kernelgsbase: u64,
    pub(crate) tsc_aux: u32,

    // SYSENTER/SYSEXIT instruction msr's
    pub(crate) sysenter_cs_msr: u32,
    pub(crate) sysenter_esp_msr: BxAddress,
    pub(crate) sysenter_eip_msr: BxAddress,

    pub(crate) pat: BxPackedRegister,
    pub(crate) mtrrphys: [u64; 16],
    pub(crate) mtrrfix64k: BxPackedRegister,
    pub(crate) mtrrfix16k: [BxPackedRegister; 2],
    pub(crate) mtrrfix4k: [BxPackedRegister; 8],
    pub(crate) mtrr_deftype: u32,

    pub(crate) ia32_feature_ctrl: u32,

    pub(crate) svm_vm_cr: u32,
    pub(crate) svm_hsave_pa: u64,

    pub(crate) ia32_xss: u64,

    pub(crate) ia32_cet_control: [u64; 2], // indexed by CPL==3
    pub(crate) ia32_pl_ssp: [u64; 4],
    pub(crate) ia32_interrupt_ssp_table: u64,

    // FRED MSRs
    pub(crate) ia32_fred_rsp: [u64; 4], // RSP0-RSP3
    pub(crate) ia32_fred_ssp: [u64; 4], // SSP0-SSP3 (CET)
    pub(crate) ia32_fred_stack_levels: u64,
    pub(crate) ia32_fred_cfg: u64,

    pub(crate) ia32_umwait_ctrl: u32,
    pub(crate) ia32_spec_ctrl: u32, // SCA
}

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {
    /* CPL == 3 */
    #[inline]
    pub(super) fn user_pl(&self) -> bool {
        self.user_pl
    }

    pub(super) fn v8086_mode(&self) -> bool {
        self.cpu_mode == CpuMode::Ia32V8086
    }

    fn bx_write_32bit_regz(&mut self, index: usize, val: u32) {
        self.gen_reg[index].set_rrx(val as _);
    }

    fn bx_write_64bit_reg(&mut self, index: usize, val: u64) {
        self.gen_reg[index].set_rrx(val);
    }
    pub(super) fn bx_clear_64bit_high(&mut self, index: usize) {
        self.gen_reg[index].set_hrx(0);
    }

    #[inline]
    pub(super) fn get_laddr32(&self, seg: usize, offset: u32) -> u32 {
        (self.sregs[seg].cache.u.segment_base() + u64::from(offset)) as u32
    }

    /// Get linear address in 64-bit mode (matching Bochs get_laddr64 — cpu.h)
    /// In 64-bit mode, ES/CS/SS/DS bases are forced to 0 per Intel SDM.
    /// Only FS and GS may have non-zero bases (loaded via MSR).
    #[inline]
    pub(super) fn get_laddr64(&self, seg: usize, offset: u64) -> u64 {
        // BxSegregs: ES=0, CS=1, SS=2, DS=3, FS=4, GS=5
        if seg < 4 {
            // ES, CS, SS, DS — base is always 0 in 64-bit mode
            offset
        } else {
            // FS, GS — use actual segment base from MSR
            self.sregs[seg].cache.u.segment_base().wrapping_add(offset)
        }
    }

    /// Read 64-bit qword from memory (matching mem_read_qword)
    pub(super) fn mem_read_qword(&self, laddr: u64) -> u64 {
        // Read 8 bytes from memory
        let bytes = [
            self.mem_read_byte(laddr),
            self.mem_read_byte(laddr + 1),
            self.mem_read_byte(laddr + 2),
            self.mem_read_byte(laddr + 3),
            self.mem_read_byte(laddr + 4),
            self.mem_read_byte(laddr + 5),
            self.mem_read_byte(laddr + 6),
            self.mem_read_byte(laddr + 7),
        ];
        u64::from_le_bytes(bytes)
    }

    /// Write 64-bit qword to memory (matching mem_write_qword)
    pub(super) fn mem_write_qword(&mut self, paddr: u64, value: u64) {
        // Write 8 bytes to memory
        let bytes = value.to_le_bytes();
        self.mem_write_byte(paddr, bytes[0]);
        self.mem_write_byte(paddr + 1, bytes[1]);
        self.mem_write_byte(paddr + 2, bytes[2]);
        self.mem_write_byte(paddr + 3, bytes[3]);
        self.mem_write_byte(paddr + 4, bytes[4]);
        self.mem_write_byte(paddr + 5, bytes[5]);
        self.mem_write_byte(paddr + 6, bytes[6]);
        self.mem_write_byte(paddr + 7, bytes[7]);
    }
}

#[derive(Debug, Default)]
pub struct MonitorAddr {
    pub(super) monitor_addr: BxPhyAddress,
    pub(crate) armed_by: u32,
}

pub(super) const BX_MONITOR_NOT_ARMED: u32 = 0;
pub(super) const BX_MONITOR_ARMED_BY_MONITOR: u32 = 1;
pub(super) const BX_MONITOR_ARMED_BY_MONITORX: u32 = 2;
pub(super) const BX_MONITOR_ARMED_BY_UMONITOR: u32 = 3;

impl MonitorAddr {
    const CACHE_LINE_SIZE: u64 = 64;

    pub fn arm(&mut self, addr: BxPhyAddress, by: u32) {
        // align to cache line
        self.monitor_addr = addr & !(Self::CACHE_LINE_SIZE - 1);
        self.armed_by = by;
    }

    pub fn reset_monitor(&mut self) {
        self.armed_by = BX_MONITOR_NOT_ARMED;
    }

    pub fn reset_umonitor(&mut self) {
        if self.armed_by == BX_MONITOR_ARMED_BY_UMONITOR {
            self.armed_by = BX_MONITOR_NOT_ARMED;
        }
    }

    pub fn reset_monitorx(&mut self) {
        if self.armed_by == BX_MONITOR_ARMED_BY_MONITORX {
            self.armed_by = BX_MONITOR_NOT_ARMED;
        }
    }

    pub fn armed(&self) -> bool {
        self.armed_by != BX_MONITOR_NOT_ARMED
    }

    pub fn armed_by_monitor(&self) -> bool {
        self.armed_by == BX_MONITOR_ARMED_BY_MONITOR
    }

    pub fn armed_by_monitorx(&self) -> bool {
        self.armed_by == BX_MONITOR_ARMED_BY_MONITORX
    }

    pub fn armed_by_umonitor(&self) -> bool {
        self.armed_by == BX_MONITOR_ARMED_BY_UMONITOR
    }
}

#[derive(Debug, Default)]
pub(super) struct Uintr {
    pub(super) ui_handler: BxAddress,
    pub(super) stack_adjust: u64,
    /// user interrupt notification vector, actually 8 bit
    pub(super) uinv: u32,
    /// user interrupt target table size
    pub(super) uitt_size: u32,
    /// user interrupt target table address
    pub(super) uitt_addr: BxAddress,
    /// user posted-interrupt descriptor address
    pub(super) upid_addr: BxAddress,
    /// user-interrupt request register
    pub(super) uirr: u64,
    /// if UIF=0 user interrupt cannot be delivered
    pub(super) uif: bool,
}

impl Uintr {
    fn senduipi_enabled(&self) -> bool {
        (self.uitt_addr & 0x1) != 0
    }
}

#[cfg(feature = "bx_debugger")]
#[derive(Debug, Default)]
pub(super) struct BxDbgGuardState {
    /// cs:eip and linear addr of instruction at guard point
    cs: u32,
    eip: BxAddress,
    laddr: BxAddress,
    // 00 - 16 bit, 01 - 32 bit, 10 - 64-bit, 11 - illegal
    code_32_64: u32, // CS seg size at guard point
}

#[cfg(feature = "bx_debugger")]
#[derive(Debug, Default)]
pub(super) struct BxGuardFound {
    guard_found: u32,
    icount_max: u64, // stop after completing this many instructions
    iaddr_index: u32,
    guard_state: BxDbgGuardState,
}

/// Type alias for instruction handler function pointer
pub(super) type InstructionHandler<I, T> = fn(&mut BxCpuC<'_, I, T>, &Instruction) -> Result<()>;

impl<'c, I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'c, I, T> {
    /// Bochs `signal_event()`: set event bit and force async check.
    /// Called by PIC (via raw pointer) when master int_pin asserts.
    #[inline]
    pub(crate) fn signal_event(&mut self, event: u32) {
        // Bochs cpu.h: pending_event |= event (event IS the bitmask, not a bit index)
        self.pending_event |= event;
        // Bochs cpu.h: if (! is_masked_event(event)) async_event = 1;
        // is_masked_event returns (event & event_mask) != 0
        // So only set async_event when event is NOT masked
        if (event & self.event_mask) == 0 {
            self.async_event = 1;
        }
    }

    /// Bochs `clear_event()`: clear event bit.
    /// Called by PIC (via raw pointer) when master int_pin deasserts.
    #[inline]
    pub(crate) fn clear_event(&mut self, event: u32) {
        // Bochs cpu.h: pending_event &= ~event (event IS the bitmask)
        self.pending_event &= !event;
    }

    /// Bochs `mask_event()`: add event bits to event_mask so they won't fire.
    /// Used by handleInterruptMaskChange when IF is cleared — external
    /// interrupts stay pending but are blocked until IF is re-enabled.
    /// Matches Bochs cpu.h
    #[inline]
    pub(crate) fn mask_event(&mut self, event_bits: u32) {
        self.event_mask |= event_bits;
    }

    /// Bochs `unmask_event()`: remove event bits from event_mask.
    /// When IF is set, PENDING_INTR is unmasked. If a pending event
    /// exists, async_event is set to trigger delivery at next boundary.
    /// Matches Bochs cpu.h
    #[inline]
    pub(crate) fn unmask_event(&mut self, event_bits: u32) {
        self.event_mask &= !event_bits;
        // If any of the newly-unmasked events are pending, force async check
        if (self.pending_event & event_bits) != 0 {
            self.async_event = 1;
        }
    }

    /// Bochs `is_unmasked_event_pending()`: check if event is both pending
    /// and not masked. Matches Bochs cpu.h
    #[inline]
    pub(crate) fn is_unmasked_event_pending(&self, event_bits: u32) -> bool {
        (self.pending_event & !self.event_mask & event_bits) != 0
    }

    #[inline]
    pub(crate) fn set_io_bus_ptr(&mut self, io: NonNull<crate::iodev::BxDevicesC>) {
        self.io_bus = Some(io);
    }

    #[inline]
    pub(crate) fn clear_io_bus(&mut self) {
        self.io_bus = None;
    }

    // ── Instrumentation helpers (no-op when `instrumentation` feature disabled) ──

    /// Fire the `repeat_iteration` hook for string/IO REP instructions.
    /// Invoked at each iteration; compiles to nothing without the feature.
    #[inline(always)]
    pub(crate) fn on_repeat_iteration(
        &mut self,
        #[cfg_attr(not(feature = "instrumentation"), allow(unused_variables))]
        instr: &super::decoder::Instruction,
    ) {
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_exec() {
            let rip = self.prev_rip;
            self.instrumentation.fire_repeat_iteration(rip, instr);
        }
    }

    /// Conditional near branch (16-bit). Fires `cnear_branch_taken`/
    /// `cnear_branch_not_taken` hooks. RIP is already at fallthrough when this
    /// helper is called (cpu_loop increments RIP before execute).
    #[inline(always)]
    pub(crate) fn conditional_branch16(&mut self, taken: bool, new_ip: u16) -> Result<()> {
        if taken {
            self.branch_near16(new_ip)?;
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_branch() {
                let src = self.prev_rip;
                self.instrumentation.fire_branch(
                    &super::instrumentation::BranchEvent::CnearTaken {
                        src_rip: src,
                        dst_rip: new_ip as u64,
                    },
                );
            }
        } else {
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_branch() {
                let src = self.prev_rip;
                let fall = self.rip();
                self.instrumentation.fire_branch(
                    &super::instrumentation::BranchEvent::CnearNotTaken {
                        src_rip: src,
                        fallthrough_rip: fall,
                    },
                );
            }
        }
        Ok(())
    }

    /// Conditional near branch (32-bit). See `conditional_branch16`.
    #[inline(always)]
    pub(crate) fn conditional_branch32(&mut self, taken: bool, new_eip: u32) -> Result<()> {
        if taken {
            self.branch_near32(new_eip)?;
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_branch() {
                let src = self.prev_rip;
                self.instrumentation.fire_branch(
                    &super::instrumentation::BranchEvent::CnearTaken {
                        src_rip: src,
                        dst_rip: new_eip as u64,
                    },
                );
            }
        } else {
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_branch() {
                let src = self.prev_rip;
                let fall = self.rip();
                self.instrumentation.fire_branch(
                    &super::instrumentation::BranchEvent::CnearNotTaken {
                        src_rip: src,
                        fallthrough_rip: fall,
                    },
                );
            }
        }
        Ok(())
    }

    /// Conditional near branch (64-bit). See `conditional_branch16`.
    /// Takes `&Instruction` because `branch_near64` extracts the displacement
    /// from the instruction itself.
    #[inline(always)]
    pub(crate) fn conditional_branch64(
        &mut self,
        taken: bool,
        instr: &super::decoder::Instruction,
    ) -> Result<()> {
        if taken {
            self.branch_near64(instr)?;
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_branch() {
                let src = self.prev_rip;
                let dst = self.rip();
                self.instrumentation.fire_branch(
                    &super::instrumentation::BranchEvent::CnearTaken {
                        src_rip: src,
                        dst_rip: dst,
                    },
                );
            }
        } else {
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_branch() {
                let src = self.prev_rip;
                let fall = self.rip();
                self.instrumentation.fire_branch(
                    &super::instrumentation::BranchEvent::CnearNotTaken {
                        src_rip: src,
                        fallthrough_rip: fall,
                    },
                );
            }
        }
        Ok(())
    }

    /// Fire an unconditional near branch hook (JMP/CALL/RET/LOOP).
    /// Call AFTER the branch_near* sets the new IP.
    #[inline(always)]
    pub(crate) fn on_ucnear_branch(
        &mut self,
        what: super::instrumentation::BranchType,
        new_rip: u64,
    ) {
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_branch() {
            let src = self.prev_rip;
            self.instrumentation
                .fire_branch(&super::instrumentation::BranchEvent::Ucnear {
                    kind: what,
                    src_rip: src,
                    dst_rip: new_rip,
                });
        }
        #[cfg(not(feature = "instrumentation"))]
        {
            let _ = (what, new_rip);
        }
    }

    /// Fire a far branch hook (inter-segment). Call AFTER the new CS:IP is set.
    #[inline(always)]
    pub(crate) fn on_far_branch(
        &mut self,
        what: super::instrumentation::BranchType,
        prev_cs: u16,
        new_cs: u16,
        new_rip: u64,
    ) {
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_branch() {
            let src_rip = self.prev_rip;
            self.instrumentation
                .fire_branch(&super::instrumentation::BranchEvent::Far {
                    kind: what,
                    src_cs: prev_cs,
                    src_rip,
                    dst_cs: new_cs,
                    dst_rip: new_rip,
                });
        }
        #[cfg(not(feature = "instrumentation"))]
        {
            let _ = (what, prev_cs, new_cs, new_rip);
        }
    }

    /// Fire the BOCHS `lin_access` hook. No-op when the feature is disabled
    /// or no memory hooks are registered.
    #[inline(always)]
    pub(crate) fn on_lin_access(
        &mut self,
        laddr: u64,
        paddr: u64,
        data: &[u8],
        rw: super::instrumentation::MemAccessRW,
    ) {
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_mem() {
            let ev = super::instrumentation::LinAccess {
                lin: laddr,
                phy: paddr,
                data,
                memtype: super::instrumentation::MemType::Wb,
                rw,
            };
            self.instrumentation.fire_lin_access(&ev);
        }
        #[cfg(not(feature = "instrumentation"))]
        {
            let _ = (laddr, paddr, data, rw);
        }
    }

    /// Initialize a BxCpuC on a pre-allocated, zeroed buffer.
    ///
    /// # Safety
    /// `ptr` must point to a zeroed buffer of at least `size_of::<BxCpuC<I, T>>()`
    /// bytes, properly aligned, exclusively owned, and valid for `'static`.
    pub unsafe fn init_on_ptr(ptr: *mut Self)
    where
        T: Default,
    {
        core::ptr::addr_of_mut!((*ptr).cpuid).write(I::new());
        core::ptr::addr_of_mut!((*ptr).ignore_bad_msrs).write(true);
        core::ptr::addr_of_mut!((*ptr).a20_mask).write(0xFFFF_FFFF_FFFF_FFFF);
        core::ptr::addr_of_mut!((*ptr).last_exception_type).write(-1);
        core::ptr::addr_of_mut!((*ptr).instrumentation)
            .write(super::instrumentation::InstrumentationRegistry::with_tracer(T::default()));
        core::ptr::addr_of_mut!((*ptr).dtlb).write(super::tlb::Tlb::new());
        core::ptr::addr_of_mut!((*ptr).itlb).write(super::tlb::Tlb::new());
    }

    #[inline]
    pub fn set_pc_system_ptr(&mut self, ps: NonNull<crate::pc_system::BxPcSystemC>) {
        self.set_pc_system_ptr_with_tick_denominator(ps, 1);
    }

    #[inline]
    pub fn set_pc_system_ptr_with_tick_denominator(
        &mut self,
        ps: NonNull<crate::pc_system::BxPcSystemC>,
        tick_denominator: u64,
    ) {
        self.pc_system_ptr = Some(ps);
        // SAFETY: PcSystem pointer is valid for the duration of the CPU batch.
        self.pc_system_ticks_at_sync = unsafe { ps.as_ref().time_ticks() };
        self.pc_system_cpu_ticks_at_sync = self.cpu_ticks();
        self.pc_system_tick_denominator = tick_denominator.max(1);
    }

    #[inline]
    pub fn clear_pc_system(&mut self) {
        self.pc_system_ptr = None;
    }

    // ── Safe accessor methods for NonNull device pointers ──────────────
    // Each centralizes the single `unsafe` deref so all call sites are safe.

    #[inline(always)]
    pub(super) fn io_bus_mut(&mut self) -> Option<&mut crate::iodev::BxDevicesC> {
        self.io_bus.map(|mut p| unsafe { p.as_mut() })
    }

    #[inline(always)]
    pub(super) fn pc_system_mut(&mut self) -> Option<&mut crate::pc_system::BxPcSystemC> {
        self.pc_system_ptr.map(|mut p| unsafe { p.as_mut() })
    }

    #[inline(always)]
    pub(super) fn pc_system_ref(&self) -> Option<&crate::pc_system::BxPcSystemC> {
        self.pc_system_ptr.map(|p| unsafe { p.as_ref() })
    }


    /// Snapshot the CPU state handler-aware memory needs before it is
    /// mutably borrowed. `addr` is already A20-adjusted so MONITOR observes
    /// exactly the same physical page as the memory mapping decision.
    #[inline]
    pub(crate) fn memory_access_policy(&self, addr: BxPhyAddress) -> CpuMemoryPolicy {
        CpuMemoryPolicy::new(self.smm_mode(), self.is_monitor(addr & !0xfff, 0xfff))
    }

    /// Snapshot memory-access policy and borrow the external memory bus.
    ///
    /// # Safety
    ///
    /// The caller must be in the exclusive wired CPU-memory scope: no other
    /// reference to this memory bus may be live, and the returned borrow must
    /// not outlive that scope.  `wire_memory_access`/`clear_memory_access`
    /// establish these bounds for CPU execution.
    #[inline(always)]
    pub(super) unsafe fn mem_bus_with_policy(
        &self,
        addr: BxPhyAddress,
    ) -> Option<(CpuMemoryPolicy, &mut crate::memory::BxMemC<'c>)> {
        let mem_bus = self.mem_bus?;
        let a20_addr = unsafe { mem_bus.as_ref().a20_addr(addr) };
        let policy = self.memory_access_policy(a20_addr);
        let mem = unsafe { &mut *mem_bus.as_ptr() };
        // HPET registers convert emulated time inside the memory handler
        // (Bochs hpet.cc reads bx_pc_system.time_nsec() there); stamp this
        // access with the CPU's live clock so mid-batch counter reads are
        // exact. Range-gated so ordinary slow-path accesses pay one compare.
        if (crate::iodev::hpet::HPET_BASE
            ..crate::iodev::hpet::HPET_BASE + crate::iodev::hpet::HPET_LEN)
            .contains(&a20_addr)
        {
            let ips = self
                .pc_system_ref()
                .map(|ps| ps.ips())
                .unwrap_or(0);
            mem.stamp_hpet_access_clock(self.system_ticks(), ips);
        }
        Some((policy, mem))
    }

    /// Apply final I/O state after a port dispatch.
    ///
    /// I/O runs through raw device pointers while an instruction is executing.
    /// The producer collapses PIC edge activity to a final physical level and
    /// separately latches scheduler-owned work; this consumer makes both
    /// visible only after the raw I/O borrow ends.
    #[inline]
    pub(super) fn sync_io_events(&mut self) {
        let (pic_intr_level, hrq_level, scheduler_boundary_requested) =
            if let Some(io) = self.io_bus_mut() {
                (
                    io.take_pic_intr_level(),
                    io.take_hrq_level(),
                    io.take_scheduler_boundary_requested(),
                )
            } else {
                (None, None, false)
            };

        if let Some(level) = pic_intr_level {
            if level {
                self.signal_event(Self::BX_EVENT_PENDING_INTR);
            } else {
                self.clear_event(Self::BX_EVENT_PENDING_INTR);
            }
        }
        if let Some(level) = hrq_level {
            // Bochs pc_system.cc set_HRQ: `HRQ = val; if (val)
            // BX_CPU(0)->async_event = 1;` — the OUT that unmasked a pending
            // DRQ makes HRQ visible at this CPU's very next instruction
            // boundary, where handle_async_event services HLDA.
            if let Some(ps) = self.pc_system_mut() {
                ps.set_hrq(level);
            }
            if level {
                self.async_event |= 1;
            }
        }
        if scheduler_boundary_requested {
            self.request_scheduler_boundary();
        }
    }

    /// Check HRQ (DMA Hold Request) state from pc_system.
    /// Matches Bochs `BX_HRQ` macro (pc_system.h) which reads
    /// `bx_pc_system.HRQ`. Returns false if pc_system is not wired.
    #[inline]
    pub(super) fn get_hrq(&self) -> bool {
        if let Some(ps) = self.pc_system_ref() {
            ps.get_hrq()
        } else {
            false
        }
    }

    /// Bochs `bx_pc_system.getNumCpuTicksLeftNextEvent()` — caps FastRep transfer counts
    /// so that timers fire on schedule.
    #[inline]
    pub(super) fn ticks_left_next_event(&self) -> u32 {
        if let Some(ps) = self.pc_system_ref() {
            ps.get_num_cpu_ticks_left_next_event()
        } else {
            u32::MAX // no cap when not wired (tests)
        }
    }

    /// Probe pc_system countdown during FastRep bulk operations.
    ///
    /// When countdown would expire, sets STOP_TRACE to force a trace break so
    /// the outer emulator loop can advance pc_system time exactly once and fire
    /// `countdown_event()`.
    #[inline]
    pub(super) fn tickn_fastrep(&mut self, n: usize) {
        if let Some(ps) = self.pc_system_ref() {
            if ps.countdown_would_expire_after(n as u32) {
                self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
            }
        }
    }

    #[inline]
    pub fn set_mem_bus_ptr(&mut self, mem: NonNull<crate::memory::BxMemC<'c>>) {
        self.mem_bus = Some(mem);
    }
    /// Wire memory and the complete stable machine pin set for a bounded
    /// CPU-side operation. `current_pin` is the externally-owned descriptor
    /// for this CPU; it is refreshed before CPU execution can mutate TLB/VMCB
    /// state and remains valid through the scope.
    #[inline]
    pub(crate) fn wire_memory_access(
        &mut self,
        mem: NonNull<crate::memory::BxMemC<'c>>,
        pins: &[crate::memory::CpuTlbPin],
        current_pin: &crate::memory::CpuTlbPin,
    ) {
        debug_assert!(pins.iter().any(|pin| core::ptr::eq(pin, current_pin)));
        self.refresh_tlb_pin_if_dirty(current_pin);
        self.active_tlb_pin_sidecar = Some(NonNull::from(current_pin));
        self.set_mem_bus_ptr(mem);
        self.active_tlb_pins = pins.as_ptr();
        self.active_tlb_pin_count = pins.len();
    }

    /// Tear down a `wire_memory_access` scope.
    #[inline]
    pub(crate) fn clear_memory_access(&mut self) {
        self.active_tlb_pins = core::ptr::null();
        self.active_tlb_pin_count = 0;
        self.active_tlb_pin_sidecar = None;
        self.clear_mem_bus();
    }
    #[inline]
    fn clear_execution_memory_wiring(&mut self) {
        self.mem_host_base = core::ptr::null_mut();
        self.mem_host_len = 0;
        self.clear_memory_access();
    }


    #[inline]
    pub fn clear_mem_bus(&mut self) {
        self.mem_bus = None;
    }

    /// Check whether a direct bulk write would overlap cached guest code.
    ///
    /// The probe is intentionally non-mutating: callers can abandon the bulk
    /// path and let the scalar RMW access perform Bochs-ordered invalidation
    /// before the corresponding external side effect.
    #[inline]
    pub(crate) fn smc_range_has_cached_code(&self, p_addr: BxPhyAddress, len: u32) -> bool {
        let Some(mem_bus) = self.mem_bus else {
            return true;
        };
        // SAFETY: mem_bus is wired for the duration of CPU execution and this
        // shared probe does not mutate memory or alias a mutable borrow.
        let mem = unsafe { mem_bus.as_ref() };
        mem.smc_range_has_stamps(p_addr, len)
    }

    /// Bochs icache.h `bxPageWriteStampTable::decWriteStamp` + icache.cc
    /// `handleSMC`: check a store against the machine-wide write-stamp table.
    /// On a hit the event is queued for every cpu; this (writing) cpu applies
    /// it immediately below — flush affected traces and stop the current
    /// trace — exactly Bochs's synchronous behavior for the writer. Sibling
    /// cpus are flushed by the emulator's drain before their next slice,
    /// which the single-threaded round-robin scheduler guarantees runs first.
    #[inline]
    pub(crate) fn smc_write_check(&mut self, p_addr: BxPhyAddress, len: u32) {
        let Some(mem_bus) = self.mem_bus else { return };
        // SAFETY: mem_bus is wired for the duration of cpu execution (same
        // invariant as every other mem_bus access); BxCpuC and BxMemC are
        // distinct objects, so this temporary &mut never aliases self.
        let mem = unsafe { &mut *mem_bus.as_ptr() };
        mem.smc_dec_write_stamp(p_addr, len);
        if mem.smc_seq_next() > self.smc_seq_seen {
            self.smc_apply_pending(mem, true);
        }
    }

    /// Apply queued SMC invalidations this cpu has not seen yet (watermark).
    /// The per-cpu body of Bochs icache.cc `handleSMC`'s all-processors loop:
    /// flush affected traces and (when `stop_trace`) set
    /// BX_ASYNC_EVENT_STOP_TRACE so the currently-running trace is abandoned.
    /// Non-consuming — events stay queued until the emulator's drain has
    /// applied them to every cpu.
    #[cold]
    pub(crate) fn smc_apply_pending(&mut self, mem: &crate::memory::BxMemC, stop_trace: bool) {
        let (needs_full_flush, events) = mem.smc_pending_since(self.smc_seq_seen);
        if needs_full_flush {
            // Pending-queue overflow dropped events — conservative full flush.
            self.i_cache.flush_all();
        } else {
            for ev in events {
                self.i_cache.handle_smc_scan(ev.p_addr, ev.mask);
            }
        }
        self.smc_seq_seen = mem.smc_seq_next();
        if stop_trace {
            self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
        }
    }

    /// After a handler-aware physical write (`write_physical_page`) issued
    /// from cpu context, apply any SMC invalidation it queued to THIS cpu
    /// immediately — Bochs icache.cc `handleSMC` flushes the writer
    /// synchronously at the store.
    #[inline]
    pub(crate) fn smc_sync_after_phys_write(&mut self) {
        let Some(mem_bus) = self.mem_bus else { return };
        // SAFETY: same mem_bus wiring invariant as smc_write_check.
        let mem = unsafe { &*mem_bus.as_ptr() };
        if mem.smc_seq_next() > self.smc_seq_seen {
            self.smc_apply_pending(mem, true);
        }
    }

    #[inline]
    pub(crate) fn debug_putc(&mut self, ch: u8) {
        let current_ticks = self.system_ticks();
        let dispatched = if let Some(io) = self.io_bus_mut() {
            io.outp(0x00E9, ch as u32, 1, current_ticks);
            true
        } else {
            false
        };
        if dispatched {
            self.sync_io_events();
        }
    }

    #[inline]
    pub(crate) fn debug_puts(&mut self, s: &[u8]) {
        for &b in s {
            self.debug_putc(b);
        }
    }

    #[inline]
    fn debug_put_hex_u8(&mut self, v: u8) {
        #[inline]
        fn nybble(n: u8) -> u8 {
            match n & 0x0f {
                0..=9 => b'0' + (n & 0x0f),
                10..=15 => b'a' + ((n & 0x0f) - 10),
                _ => b'?',
            }
        }
        self.debug_putc(nybble(v >> 4));
        self.debug_putc(nybble(v));
    }

    #[inline]
    fn debug_put_hex_u16(&mut self, v: u16) {
        self.debug_put_hex_u8((v >> 8) as u8);
        self.debug_put_hex_u8(v as u8);
    }

    /// Inject an external (hardware) interrupt vector into the CPU.
    ///
    /// This is used by the outer emulator loop to deliver PIC interrupts and
    /// wake the CPU from `HLT`, mirroring Bochs' event/timer driven flow.
    ///
    /// Note: callers must ensure the memory bus is wired (`mem_bus` set) so that
    /// stack pushes and IVT/IDT reads work correctly.
    /// Inject an external interrupt via the unified interrupt() dispatch.
    /// Based on Bochs event.cc HandleExtInterrupt (lines 133-184).
    ///
    /// Sets EXT=1, uses the unified interrupt() for proper inhibit_mask clearing,
    /// speculative_rsp, and BadVector recovery, then commits prev_rip.
    pub(crate) fn inject_external_interrupt(&mut self, vector: u8) -> Result<()> {
        #[cfg(debug_assertions)]
        {
            self.diag_inject_ext_intr_count += 1;
            self.diag_inject_ext_intr_vectors[vector as usize] += 1;
        }

        // BOCHS BX_INSTR_HWINTERRUPT(cpu_id, vector, cs, eip)
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_hw_interrupt() {
            let cs = self.sregs[super::decoder::BxSegregs::Cs as usize]
                .selector
                .value;
            let rip = self.rip();
            let ev = super::instrumentation::HwInterruptEvent { vector, cs, rip };
            self.instrumentation.fire_hwinterrupt(&ev);
        }

        // Wake from halt/wait state.
        self.activity_state = CpuActivityState::Active;
        // Clear stop-trace and sleep sentinel so execution can resume.
        // BX_ASYNC_EVENT_SLEEP (bit 0) must be cleared here because this path
        // bypasses handle_async_event's tail which normally clears async_event.
        self.async_event &= !(BX_ASYNC_EVENT_STOP_TRACE | Self::BX_ASYNC_EVENT_SLEEP);

        // Mark as external interrupt (EXT=1) — affects error codes pushed
        // during any exception that occurs during interrupt delivery.
        // Based on Bochs event.cc
        self.ext = true;

        // Use unified interrupt() dispatch which handles:
        // - inhibit_mask clearing
        // - speculative_rsp setup/commit
        // - BadVector → exception() recovery
        // - mode dispatch (real vs protected)
        // soft_int=false, no error code for external IRQs
        let result = self.interrupt(
            vector,
            super::exception::InterruptType::ExternalInterrupt,
            false,
            false,
            0,
        );

        // Commit prev_rip after successful delivery (Bochs event.cc)
        if result.is_ok() {
            self.prev_rip = self.rip();
        }

        // CpuLoopRestart is expected from interrupt() — convert to Ok for external callers
        match result {
            Err(super::error::CpuError::CpuLoopRestart) => Ok(()),
            other => other,
        }
    }

    /// True if the CPU is halted or waiting for an event.
    ///
    /// We use this to decide when the outer emulator loop should inject
    /// PIC interrupts (wake-from-HLT), matching Bochs' wait-for-event flow.
    pub(crate) fn is_waiting_for_event(&self) -> bool {
        !matches!(self.activity_state, CpuActivityState::Active)
    }

    /// True if the CPU has triple-faulted and entered shutdown state.
    ///
    /// The emulator run loop should stop when this is true to avoid spinning.
    pub fn is_in_shutdown(&self) -> bool {
        matches!(self.activity_state, CpuActivityState::Shutdown)
    }

    /// Execute CPU loop with an attached I/O bus (port handlers).
    ///
    /// This sets the bus, pc_system, pic, and dma pointers for the duration of the call
    /// and clears them afterwards.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn cpu_loop_n_with_io(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
        current_pin: &crate::memory::CpuTlbPin,
        max_instructions: u64,
        strict_instruction_budget: bool,
        pc_tick_denominator: u64,
        io: NonNull<crate::iodev::BxDevicesC>,
        pc_system: NonNull<crate::pc_system::BxPcSystemC>,
        pic: Option<&mut crate::pic::BxPicC>,
        dma: Option<&mut crate::dma::BxDmaC>,
    ) -> super::Result<u64> {
        self.set_io_bus_ptr(io);
        self.set_pc_system_ptr_with_tick_denominator(pc_system, pc_tick_denominator);
        let result = if strict_instruction_budget {
            self.cpu_loop_n_impl::<false, true>(
                mem,
                cpus,
                current_pin,
                max_instructions,
                pic,
                dma,
            )
        } else {
            self.cpu_loop_n_impl::<false, false>(
                mem,
                cpus,
                current_pin,
                max_instructions,
                pic,
                dma,
            )
        };
        self.clear_io_bus();
        self.clear_pc_system();
        result
    }

    /// Execute exactly one icache trace with an attached I/O bus, then return.
    ///
    /// Bochs cpu.cc `cpu_run_trace`: handle pending async events, execute one
    /// trace, and return so the SMP scheduler can switch to the next CPU.
    /// Exceptions also end the slice (Bochs main.cc `bx_begin_simulation`
    /// setjmp lands back in the scheduler loop). `max_instructions` remains a
    /// safety cap only — the icache already caps SMP traces at the quantum.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn cpu_run_trace_with_io(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
        current_pin: &crate::memory::CpuTlbPin,
        max_instructions: u64,
        strict_instruction_budget: bool,
        pc_tick_denominator: u64,
        io: NonNull<crate::iodev::BxDevicesC>,
        pc_system: NonNull<crate::pc_system::BxPcSystemC>,
        pic: Option<&mut crate::pic::BxPicC>,
        dma: Option<&mut crate::dma::BxDmaC>,
    ) -> super::Result<u64> {
        self.set_io_bus_ptr(io);
        self.set_pc_system_ptr_with_tick_denominator(pc_system, pc_tick_denominator);
        let result = if strict_instruction_budget {
            self.cpu_loop_n_impl::<true, true>(
                mem,
                cpus,
                current_pin,
                max_instructions,
                pic,
                dma,
            )
        } else {
            self.cpu_loop_n_impl::<true, false>(
                mem,
                cpus,
                current_pin,
                max_instructions,
                pic,
                dma,
            )
        };
        self.clear_io_bus();
        self.clear_pc_system();
        result
    }

    pub(crate) fn cpu_loop(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
        current_pin: &crate::memory::CpuTlbPin,
    ) -> super::Result<()> {
        let _stack_anchor = 0;

        self.cpuloop_stack_anchor = None;

        // Bochs uses setjmp here; we use CpuLoopRestart via Rust error propagation.
        // We get here either by a normal function call, or by a CpuLoopRestart
        // back from an exception() call.  In either case, commit the
        // new EIP/ESP, and set up other environmental fields.  This code
        // mirrors similar code below, after the interrupt() call.

        self.prev_rip = self.rip();
        self.speculative_rsp = false;

        if self.in_vmx_guest {
            let vm = &mut self.vmcs;

            if vm.shadow_stack_prematurely_busy {
                return Err(CpuError::ShadowStackPrematurelyBusy);
            }
            vm.shadow_stack_prematurely_busy = false; // for safety
        }

        // Execute instructions in a loop. Use unsafe to work around lifetime issues with
        // the mem borrow across loop iterations (each call is independent but compiler
        // doesn't see it due to the 'c lifetime binding).
        //
        // SAFETY: We cast mem to a shorter-lived reference for each loop iteration.
        // Each call to get_icache_entry is independent and completes before the next iteration.

        self.cpu_loop_n(mem, cpus, current_pin, 1_000_000, None, None)?;
        Ok(())
    }

    /// Execute CPU loop with a maximum instruction count.
    ///
    /// Returns Ok(instructions_executed) when limit is reached or async event occurs.
    pub(crate) fn cpu_loop_n(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
        current_pin: &crate::memory::CpuTlbPin,
        max_instructions: u64,
        pic: Option<&mut crate::pic::BxPicC>,
        dma: Option<&mut crate::dma::BxDmaC>,
    ) -> super::Result<u64> {
        self.cpu_loop_n_impl::<false, false>(
            mem,
            cpus,
            current_pin,
            max_instructions,
            pic,
            dma,
        )
    }

    /// Shared body of `cpu_loop_n` / `cpu_run_trace_with_io`.
    ///
    /// With `stop_after_one_trace` set this behaves like Bochs cpu.cc
    /// `cpu_run_trace`: the call returns at the first trace boundary, when an
    /// async event breaks the trace, or when an exception restarts the loop
    /// (Bochs main.cc setjmp returns control to the SMP scheduler).
    fn cpu_loop_n_impl<
        const STOP_AFTER_ONE_TRACE: bool,
        const STRICT_INSTRUCTION_BUDGET: bool,
    >(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
        current_pin: &crate::memory::CpuTlbPin,
        max_instructions: u64,
        mut pic: Option<&mut crate::pic::BxPicC>,
        mut dma: Option<&mut crate::dma::BxDmaC>,
    ) -> super::Result<u64> {
        // Wire the memory system pointer for the duration of this execution call.
        // This enables Bochs-style "host-pointer-or-fallback" access in mem_read/mem_write.
        // Reborrow `mem` so we don't move the `&mut` binding.
        self.a20_mask = mem.a20_mask();
        self.wire_memory_access(NonNull::from(&mut *mem), cpus, current_pin);
        let _memory_wiring = CpuMemoryWiringGuard::new(self);

        // Direct pointers are available only for a complete, identity-backed
        // guest RAM mapping. All other accesses use the wired memory bus.
        let (host_base, host_len) = mem.identity_guest_base();
        self.mem_host_base = host_base;
        self.mem_host_len = host_len;

        let mut iteration = 0u64;
        // `iteration` is the batch budget counter (one per handler dispatch).
        // The RETURN VALUE is the icount delta — Bochs cpu.h icount units:
        // slow `repeat()` loops retire one icount per iteration inside the
        // handler (Bochs cpu.cc repeat), which `iteration` cannot see.
        // Fast-REP element surpluses are ticks, not instructions, and land
        // in `tick_surplus` (Bochs string.cc BX_TICKN(count-1)), so they
        // appear in neither counter.
        let icount_start = self.icount;
        #[cfg(feature = "profiling")]
        let mut prof_assign_ns = 0u64;
        #[cfg(feature = "profiling")]
        let mut prof_exec_ns = 0u64;
        #[cfg(feature = "profiling")]
        let mut prof_icache_ns = 0u64;

        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        tracing::trace!(
            "CPU loop starting at CS:IP = {:04X}:{:08X}",
            self.cs_selector_value(),
            self.rip()
        );

        let mut outer_loop_count = 0u64;
        let result = 'cpu_loop: loop {
            outer_loop_count += 1;
            // Spin diagnostics are debug-only: the periodic trace and its
            // modulo are compiled out of release so the hot per-trace path
            // stays minimal. The infinite-loop bailout stays in release — a
            // single comparison per trace is negligible — as a safety net.
            #[cfg(debug_assertions)]
            if outer_loop_count.is_multiple_of(100_000) {
                tracing::trace!(
                    "[cpu_loop-spin] outer={} iter={}/{} RIP={:#010x} async={} activity={:?}",
                    outer_loop_count,
                    iteration,
                    max_instructions,
                    self.rip(),
                    self.async_event,
                    self.activity_state,
                );
            }
            if outer_loop_count > 50_000_000 {
                tracing::error!("[cpu_loop] BAILOUT after {} outer loops", outer_loop_count);
                break Ok(iteration);
            }

            // Cooperative stop request (Bochs kill_bochs_request analogue):
            // any hook may set `stop_request` to break out of the batch. Latency
            // is at most one trace (~10-20 instructions).
            if self.instrumentation.stop_request {
                self.instrumentation.stop_request = false;
                break Ok(iteration);
            }

            // Safety limit - pause when instruction limit is reached
            // Use >= so each batch runs exactly max_instructions, not max_instructions+1.
            if iteration >= max_instructions {
                #[cfg(feature = "profiling")]
                tracing::trace!(
                    "CPU-LOOP-STATS: {} instr, icache={}ms assign={}ms exec={}ms",
                    iteration,
                    prof_icache_ns / 1_000_000,
                    prof_assign_ns / 1_000_000,
                    prof_exec_ns / 1_000_000
                );
                #[cfg(feature = "profiling")]
                {
                    prof_icache_ns = 0;
                    prof_assign_ns = 0;
                    prof_exec_ns = 0;
                }
                #[cfg(feature = "profiling")]
                {
                    self.perf_icache_miss = 0;
                    self.perf_prefetch = 0;
                }
                // Clear STOP_TRACE (trace-boundary hint only; served its purpose).
                // BX_ASYNC_EVENT_SLEEP (bit 0) intentionally survives: if HLT was the
                // last instruction in this batch, the next batch sees SLEEP set, calls
                // handle_async_event → handle_wait_for_event, and correctly returns Ok(0)
                // while waiting for an interrupt. This matches Bochs enter_sleep_state
                // behavior (proc_ctrl.cc: async_event = 1).
                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                break Ok(iteration);
            }

            // check on events which occurred for previous instructions (traps)
            // and ones which are asynchronous to the CPU (hardware interrupts)
            // Matches Bochs cpu.cc
            if self.async_event != 0 {
                // A machine boundary is not an architectural event. Return
                // before `handle_async_event`, whose normal tail may clear
                // async_event, and leave the latch for run_cpu_batch alone.
                if self.async_event & BX_ASYNC_EVENT_SCHEDULER_BOUNDARY != 0 {
                    break Ok(iteration);
                }
                // Fast path: if only STOP_TRACE is set and CPU is still active,
                // just clear it without calling handle_async_event(). This is the
                // common case after a taken branch — no real events to process.
                if self.async_event == BX_ASYNC_EVENT_STOP_TRACE
                    && matches!(self.activity_state, CpuActivityState::Active)
                {
                    self.async_event = 0;
                } else if self.handle_async_event(
                    pic.as_deref_mut(),
                    dma.as_deref_mut(),
                    Some(mem),
                    cpus,
                ) {
                    // Slow path: real async event (interrupt, HLT, shutdown, etc.)
                    break Ok(iteration);
                }
            }

            // Get raw pointer to mem before the loop to work around borrow checker
            // SAFETY: We'll use this raw pointer to create new references after borrows are released
            let mem_ptr: *mut BxMemC<'c> = mem;

            // SAFETY: We extend the lifetime of mem temporarily for this call only.
            // The borrow is released at the end of the expression.
            #[cfg(feature = "profiling")]
            let _t0 = std::time::Instant::now();
            // SAFETY: mem_ptr valid for duration of cpu_loop; reborrow is non-overlapping
            let (mut instr_idx, mut trace_end) = unsafe {
                let mem_extended: &'c mut BxMemC<'c> = &mut *mem_ptr;
                match self.get_icache_entry(mem_extended, cpus) {
                    Ok((start, tlen)) => (start, start + tlen),
                    Err(crate::cpu::CpuError::CpuLoopRestart) => {
                        // Bochs setjmp handler (cpu.cc): icount++, then
                        // line 154: prev_rip = RIP; speculative_rsp = false;
                        self.icount += 1;
                        iteration += 1;
                        self.prev_rip = self.rip();
                        self.speculative_rsp = false;
                        self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                        if STOP_AFTER_ONE_TRACE {
                            // Bochs main.cc: the SMP-loop setjmp ends this
                            // CPU's turn on an exception.
                            break 'cpu_loop Ok(iteration);
                        }
                        continue 'cpu_loop;
                    }
                    Err(e) => break 'cpu_loop Err(e),
                }
            };
            if STRICT_INSTRUCTION_BUDGET {
                let trace_budget = usize::try_from(max_instructions.saturating_sub(iteration))
                    .unwrap_or(usize::MAX);
                trace_end = trace_end.min(instr_idx.saturating_add(trace_budget));
            }
            #[cfg(feature = "profiling")]
            {
                prof_icache_ns += _t0.elapsed().as_nanos() as u64;
            }
            let is_real = self.real_mode();

            // Unicorn-inspired: fire block hook at trace (basic block) start
            #[cfg(feature = "instrumentation")]
            if self.instrumentation.active.has_block() {
                let block_rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
                let block_len = (trace_end - instr_idx) as u16;
                self.instrumentation.fire_block_start(block_rip, block_len);
            }

            'trace: loop {
                // Bochs-style: pointer to mpool slot — no 24-byte copy per instruction.
                // The raw pointer decouples the mpool borrow from the `&mut self`
                // execute_instruction call below (they never alias — see SAFETY).
                //
                // instr_idx is always a valid mpool index: it starts at a get_icache_entry
                // mpool_start_idx and only advances within [start, start+tlen), and
                // serve_icache_miss guarantees start + BX_MAX_TRACE_LENGTH + 1 <= mpool length.
                // The debug_assert enforces that invariant in debug/test builds; release keeps
                // the per-instruction bounds check elided.
                debug_assert!(
                    instr_idx < self.i_cache.mpool.len(),
                    "mpool index {} out of bounds (len {})",
                    instr_idx,
                    self.i_cache.mpool.len()
                );
                // SAFETY: instr_idx is in-bounds (invariant above, checked in debug builds).
                // execute_instruction only mutates CPU registers and guest memory, never
                // i_cache.mpool; serve_icache_miss (the sole mpool writer) runs only inside
                // get_icache_entry, not during execution — so the slot is stable for this call.
                let i_ptr: *const Instruction =
                    unsafe { self.i_cache.mpool.get_unchecked(instr_idx) as *const Instruction };
                // SAFETY: i_ptr points into self.i_cache.mpool which is stable for this
                // iteration — execute_instruction never writes to mpool, and serve_icache_miss
                // is only called from get_icache_entry, not during execution.
                let instr_ref = || -> &Instruction { unsafe { &*i_ptr } };

                // Bochs cpu.cc sets prev_rip AFTER execution (not before ilen).
                // prev_rip is set below, after execute_instruction returns Ok(()).

                // Advance RIP before execution (handlers may read RIP and expect it advanced)
                // SAFETY: gen_reg is initialized during CPU init; BX_64BIT_REG_RIP is always valid.
                let ilen_val = instr_ref().ilen();
                // ilen=0 is valid ONLY for InsertedOpcode (trace boundary marker).
                // Debug-only sanity check — Bochs does not validate ilen in the hot loop,
                // and the decoder already guarantees 1..=15 (or 0 for InsertedOpcode).
                #[cfg(debug_assertions)]
                if ilen_val == 0 || ilen_val > 15 {
                    let oc = instr_ref().get_ia_opcode();
                    assert!(
                        ilen_val == 0 && oc == super::decoder::Opcode::InsertedOpcode,
                        "Invalid ilen={} opcode={:?} at RIP={:#x}",
                        ilen_val,
                        oc,
                        self.gen_reg[BX_64BIT_REG_RIP].rrx()
                    );
                }
                self.gen_reg[BX_64BIT_REG_RIP]
                    .set_rrx(self.gen_reg[BX_64BIT_REG_RIP].rrx() + ilen_val as u64);
                if is_real {
                    self.gen_reg[BX_64BIT_REG_RIP]
                        .set_rrx(self.gen_reg[BX_64BIT_REG_RIP].rrx() & 0xFFFF);
                }

                // Execute instruction (matching C++ BX_CPU_CALL_METHOD)
                let opcode = instr_ref().get_ia_opcode();
                #[cfg(debug_assertions)]
                {
                    self.diag_current_opcode = opcode as u16;
                }

                // Bochs BX_INSTR_BEFORE_EXECUTION(cpu_id, i)
                #[cfg(feature = "instrumentation")]
                if self.instrumentation.active.has_exec() {
                    let rip_before = self.prev_rip;
                    self.instrumentation
                        .fire_before_execution(rip_before, instr_ref());
                }

                // A2 single dispatch (unmeasured): the canonical
                // `execute_instruction` match is the sole path. The former
                // parallel `i_cache_handlers` fn-ptr pool cached exactly the
                // arm the opcode selects here (matching handlers == the arm;
                // excluded opcodes already fell back to this match), so this is
                // behaviorally identical while dropping the 4.6 MB pool and the
                // per-instruction Option check. Dispatch mechanism is not
                // guest-observable, so the match is parity-safe.
                let execution_result = self.execute_instruction(instr_ref());
                match execution_result {
                    Ok(()) => {}
                    Err(crate::cpu::CpuError::CpuLoopRestart) => {
                        // Exception delivery during execution: restart decode (Bochs longjmp).
                        // Bochs setjmp handler (cpu.cc): icount++, prev_rip = RIP,
                        // speculative_rsp = false, then continue outer loop.
                        self.icount += 1;
                        iteration += 1;
                        self.prev_rip = self.rip();
                        self.speculative_rsp = false;
                        // If triple fault set Shutdown, exit cleanly instead of restarting.
                        if matches!(self.activity_state, CpuActivityState::Shutdown) {
                            tracing::trace!("CPU shutdown — exiting cpu_loop");
                            break 'cpu_loop Ok(iteration);
                        }
                        self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                        if STOP_AFTER_ONE_TRACE {
                            // Bochs main.cc: the SMP-loop setjmp ends this
                            // CPU's turn on an exception.
                            break 'cpu_loop Ok(iteration);
                        }
                        continue 'cpu_loop;
                    }
                    Err(e) => {
                        // Fatal errors still leave through the loop result so
                        // `CpuMemoryWiringGuard` tears down every CPU-memory
                        // pointer before the error reaches the scheduler.
                        break 'cpu_loop match self.handle_execution_error(e, instr_ref()) {
                            Ok(()) => Err(crate::cpu::CpuError::CpuNotInitialized),
                            Err(error) => Err(error),
                        };
                    }
                }

                // Bochs BX_INSTR_AFTER_EXECUTION(cpu_id, i)
                // Use prev_rip BEFORE updating it — that's the address of the instruction
                // we just executed (matches BOCHS semantics).
                #[cfg(feature = "instrumentation")]
                if self.instrumentation.active.has_exec() {
                    let executed_rip = self.prev_rip;
                    self.instrumentation
                        .fire_after_execution(executed_rip, instr_ref());
                }

                // Bochs cpu.cc — prev_rip = RIP AFTER execution ("commit new RIP")
                self.prev_rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
                // Bochs cpu.cc — icount++
                self.icount += 1;
                #[cfg(feature = "profiling")]
                {
                    self.perf_instructions += 1;
                }

                iteration += 1;

                // Check async events (matching C++ line 215: if (async_event) break;)
                // When async_event is set (branch taken, exception, HLT, etc.), we MUST
                // break out of the trace because RIP has changed and the next sequential
                // instruction in the trace is wrong. The outer loop will handle the event
                // and fetch a new trace for the updated RIP.
                if self.async_event != 0 {
                    // Bochs ctrl_xfer*.cc BX_LINK_TRACE → cpu.cc linkTrace:
                    // when the ONLY pending event is the taken branch's own
                    // STOP_TRACE and the branch is a direct near transfer,
                    // continue straight into the cached target trace without
                    // returning to the outer loop or re-hashing the icache.
                    // Guard composition mirrors linkTrace: real async events
                    // never link (the equality test), SMP never links
                    // (STOP_AFTER_ONE_TRACE), and `iteration < max` is the
                    // ticks-left guard — UP batches are pre-capped at the
                    // next pc_system deadline, so a linked chain cannot run
                    // past a timer any more than the existing trace-end
                    // chaining can. Cooperative stop keeps trace latency.
                    // BENCHMARK-ONLY (temporary): link-rate diagnostics —
                    // 498 = STOP_TRACE breaks, 499 = other async breaks,
                    // 500 = guard passed, 501 = link followed.
                    if self.async_event == BX_ASYNC_EVENT_STOP_TRACE {
                        crate::vec_diag::count(498);
                    } else {
                        crate::vec_diag::count(499);
                    }
                    if !STOP_AFTER_ONE_TRACE
                        && self.async_event == BX_ASYNC_EVENT_STOP_TRACE
                        && iteration < max_instructions
                        && matches!(self.activity_state, CpuActivityState::Active)
                        && !self.instrumentation.stop_request
                        && super::icache::is_linkable_opcode(opcode)
                    {
                        crate::vec_diag::count(500);
                        if let Some((start, tlen)) = self.try_link_trace(instr_idx) {
                            crate::vec_diag::count(501);
                            self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                            instr_idx = start;
                            trace_end = start + tlen;
                            if STRICT_INSTRUCTION_BUDGET {
                                let trace_budget = usize::try_from(
                                    max_instructions.saturating_sub(iteration),
                                )
                                .unwrap_or(usize::MAX);
                                trace_end =
                                    trace_end.min(instr_idx.saturating_add(trace_budget));
                            }
                            #[cfg(feature = "instrumentation")]
                            if self.instrumentation.active.has_block() {
                                let block_rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
                                let block_len = (trace_end - instr_idx) as u16;
                                self.instrumentation.fire_block_start(block_rip, block_len);
                            }
                            continue 'trace;
                        }
                    }
                    break 'trace;
                }

                // Matching C++ line 217: if (++i == last) { get new trace }
                instr_idx += 1;
                if instr_idx >= trace_end {
                    // Bochs cpu.cc cpu_run_trace executes exactly one trace
                    // per call — in an SMP slice, return to the scheduler at
                    // the trace boundary instead of chaining.
                    if STOP_AFTER_ONE_TRACE {
                        break 'cpu_loop Ok(iteration);
                    }
                    // Check instruction limit at trace boundary (not per-instruction)
                    if iteration >= max_instructions {
                        break 'cpu_loop Ok(iteration);
                    }
                    // Chain to new trace without breaking to outer loop
                    // (matching C++ line 218-220: entry=getICacheEntry; i=entry->i; last=...)
                    // SAFETY: mem_ptr valid for duration of cpu_loop; reborrow is non-overlapping
                    let (start, tlen) = unsafe {
                        let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
                        match self.get_icache_entry(mem_reborrowed, cpus) {
                            Ok(v) => v,
                            Err(crate::cpu::CpuError::CpuLoopRestart) => {
                                // Bochs setjmp handler: icount++, prev_rip = RIP
                                self.icount += 1;
                                iteration += 1;
                                self.prev_rip = self.rip();
                                self.speculative_rsp = false;
                                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                                continue 'cpu_loop;
                            }
                            Err(e) => break 'cpu_loop Err(e),
                        }
                    };
                    instr_idx = start;
                    trace_end = start + tlen;
                    if STRICT_INSTRUCTION_BUDGET {
                        let trace_budget =
                            usize::try_from(max_instructions.saturating_sub(iteration))
                                .unwrap_or(usize::MAX);
                        trace_end = trace_end.min(instr_idx.saturating_add(trace_budget));
                    }

                    // Unicorn-inspired: fire block hook at trace (basic block) start
                    #[cfg(feature = "instrumentation")]
                    if self.instrumentation.active.has_block() {
                        let block_rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
                        let block_len = (trace_end - instr_idx) as u16;
                        self.instrumentation.fire_block_start(block_rip, block_len);
                    }
                }
            }

            // A scheduler request stopped this trace. Preserve the distinct
            // latch and return before STOP_TRACE continuation or async handling.
            if self.async_event & BX_ASYNC_EVENT_SCHEDULER_BOUNDARY != 0 {
                break 'cpu_loop Ok(iteration);
            }
            // Clear stop trace magic indication (matching C++ line 226).
            self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
            // Bochs cpu.cc cpu_run_trace returns after the trace was broken
            // by an async event; the SMP scheduler switches CPUs here.
            if STOP_AFTER_ONE_TRACE {
                break 'cpu_loop Ok(iteration);
            }
        };

        // Bochs icount units: report retired instructions, which includes
        // slow-repeat iterations charged inside handlers that `iteration`
        // (the dispatch counter carried by the Ok breaks) does not count.
        result.map(|_| self.icount.wrapping_sub(icount_start))
    }

    /// Cold path: handle fatal errors from instruction execution.
    /// Separated from the hot inner loop to keep the hot path small for better
    /// instruction cache utilization.
    #[cold]
    #[inline(never)]
    fn handle_execution_error(
        &self,
        e: crate::cpu::CpuError,
        instr: &Instruction,
    ) -> super::Result<()> {
        use crate::cpu::CpuError;
        match e {
            CpuError::CpuNotInitialized => {
                // Silent — CPU shutting down
            }
            CpuError::UnimplementedOpcode { ref opcode } => {
                let rip = self.prev_rip; // prev_rip was the RIP before advancement
                let cs_base = self.sregs[BxSegregs::Cs as usize].cache.u.segment_base();
                let laddr = cs_base + rip;
                let cs_value = self.cs_selector_value();
                let instr_bytes: [u8; 16] = if let Some(fetch_ptr) = &self.eip_fetch_ptr {
                    let page_base = cs_base + self.eip_page_bias;
                    let offset = (rip.wrapping_sub(page_base)) as usize;
                    let ilen = instr.ilen() as usize;
                    if offset < fetch_ptr.len() && offset + ilen <= fetch_ptr.len() {
                        let mut buf = [0u8; 16];
                        let copy_len = ilen.min(16);
                        buf[..copy_len].copy_from_slice(&fetch_ptr[offset..offset + copy_len]);
                        buf
                    } else {
                        [0u8; 16]
                    }
                } else {
                    [0u8; 16]
                };
                let ilen = instr.ilen() as usize;
                tracing::error!(
                    "UNIMPLEMENTED OPCODE: {:?} at RIP={:#x} CS:IP={:#x}:{:#x} laddr={:#x} bytes={:02x?}",
                    opcode, rip, cs_value, rip, laddr, &instr_bytes[..ilen.min(16)]
                );
            }
            _ => {
                let rip = self.prev_rip;
                let cs_value = self.cs_selector_value();
                let opcode = instr.get_ia_opcode();
                tracing::error!(
                    "CPU ERROR at icount={} RIP={:#x} CS={:#x} opcode={:?}: {}",
                    self.icount,
                    rip,
                    cs_value,
                    opcode,
                    e
                );
                tracing::error!(
                    "  EAX={:#x} ECX={:#x} EDX={:#x} EBX={:#x} ESP={:#x} EBP={:#x} ESI={:#x} EDI={:#x}",
                    self.get_gpr32(0), self.get_gpr32(1), self.get_gpr32(2), self.get_gpr32(3),
                    self.get_gpr32(4), self.get_gpr32(5), self.get_gpr32(6), self.get_gpr32(7)
                );
            }
        }
        Err(e)
    }

    fn fetch_next_instruction(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
    ) -> Result<Instruction> {
        let mem_ptr: *mut BxMemC<'c> = mem;
        // SAFETY: mem_ptr valid for duration of cpu_loop; reborrow is non-overlapping
        let (mpool_start_idx, _tlen) = unsafe {
            let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
            self.get_icache_entry(mem_reborrowed, cpus)?
        };
        Ok(self.i_cache.mpool[mpool_start_idx])
    }

    /// Bochs cpu.cc `linkTrace`, loop-continuation form: after a taken direct
    /// near branch, try to continue at the branch target's cached trace
    /// without returning to the outer loop or re-hashing per branch.
    ///
    /// Hit-only, exactly like Bochs (`entry != NULL`): on any doubt —
    /// icache miss, target outside the current prefetch window (Bochs
    /// prefetch()es here; refusing is a pure link-rate reduction), or a
    /// stale/mismatched stored link — return `None` and let the outer loop's
    /// full `get_icache_entry` path (prefetch, SMC watermark, miss service)
    /// handle it. Every refusal is behavior-invisible.
    ///
    /// Links die on `break_links`/`flush_all` via the timestamp bump; CPU
    /// stores invalidate synchronously (`handle_smc_scan` → `break_links`)
    /// and device writes only land at scheduler boundaries where the batch
    /// ends, so a mid-batch link can never bypass a pending invalidation
    /// that `get_icache_entry`'s SMC watermark would have applied.
    ///
    /// `expected_rip` is stored/checked so a stale link can only be followed
    /// when the branch target genuinely resolves to the same mapping —
    /// stronger than Bochs, which follows the stored target unconditionally.
    #[inline]
    fn try_link_trace(&mut self, branch_idx: usize) -> Option<(usize, usize)> {
        let rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
        let stamp = self.i_cache.trace_link_time_stamp;
        // Fast path — Bochs instr.h getNextTrace.
        if let Some(target) = self.i_cache.trace_links[branch_idx].target(stamp, rip) {
            return Some(target);
        }
        // Store path — Bochs linkTrace's find_entry hit case.
        let eip_biased = (rip as i64).wrapping_add(self.eip_page_bias as i64) as u32;
        if self.eip_page_window_size == 0 || eip_biased >= self.eip_page_window_size {
            return None;
        }
        let p_addr: BxPhyAddress = self.p_addr_fetch_page.wrapping_add(u64::from(eip_biased));
        let hash_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;
        let entry = &self.i_cache.entry[hash_idx];
        if entry.p_addr != p_addr {
            return None;
        }
        let start = entry.mpool_start_idx;
        let tlen = entry.tlen as usize;
        self.i_cache.trace_links[branch_idx] =
            super::icache::TraceLink::store(stamp, start, tlen, rip);
        Some((start, tlen))
    }

    /// Look up the instruction cache for the current RIP.
    /// Returns (mpool_start_idx, tlen) to avoid cloning BxICacheEntry on the hot path.
    /// Matching Bochs cpu.cc getICacheEntry().
    #[inline]
    fn get_icache_entry(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[crate::memory::CpuTlbPin],
    ) -> Result<(usize, usize)> {
        // Apply machine-wide SMC invalidations this cpu has not seen before
        // consulting the icache — device DMA (Bochs memory.cc
        // dmaWritePhysicalPage) and non-store cpu write paths queue events
        // between this cpu's slices/stores. Bochs flushes every cpu
        // synchronously in icache.cc handleSMC; the watermark makes this a
        // single compare per trace lookup. No STOP_TRACE: there is no
        // running trace at a lookup boundary.
        if mem.smc_seq_next() > self.smc_seq_seen {
            self.smc_apply_pending(mem, false);
        }
        // Check if we need to prefetch a new page (matching C++ lines 289-292)
        let needs_prefetch = self.eip_page_window_size == 0 || {
            let eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;
            eip_biased >= self.eip_page_window_size
        };
        // Get raw pointer to mem before calling prefetch() to work around borrow checker
        // SAFETY: addr_of_mut avoids creating intermediate reference; pointer valid for fn scope
        let mem_ptr: *mut BxMemC<'c> = unsafe { core::ptr::addr_of_mut!(*mem) };
        let mut eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;

        if needs_prefetch {
            #[cfg(feature = "profiling")]
            {
                self.perf_prefetch += 1;
            }
            let mut retry_count = 0;
            loop {
                // SAFETY: mem_ptr valid for duration of cpu_loop; reborrow is non-overlapping
                let mem_reborrowed: &'c mut BxMemC<'c> = unsafe { &mut *mem_ptr };
                self.prefetch(mem_reborrowed, cpus)?;

                if self.eip_page_window_size == 0 || self.eip_fetch_ptr.is_none() {
                    retry_count += 1;
                    if retry_count > 10 {
                        tracing::error!("prefetch retry limit exceeded, RIP={:#x}", self.rip());
                        return Err(crate::cpu::CpuError::CpuNotInitialized);
                    }
                    tracing::trace!(
                        "prefetch queue invalidated after exception, retrying (attempt {})",
                        retry_count
                    );
                    continue;
                }

                eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;

                if eip_biased >= self.eip_page_window_size {
                    tracing::trace!("eip_biased ({}) >= eip_page_window_size ({}) after prefetch, RIP={:#x}, retrying",
                        eip_biased, self.eip_page_window_size, self.rip());
                    self.eip_fetch_ptr = None;
                    self.eip_page_window_size = 0;
                    retry_count += 1;
                    if retry_count > 10 {
                        tracing::error!("prefetch eip_biased retry limit exceeded");
                        return Err(crate::cpu::CpuError::CpuNotInitialized);
                    }
                    continue;
                }

                break;
            }
        }

        // Physical address for this instruction
        let p_addr: BxPhyAddress = self
            .p_addr_fetch_page
            .wrapping_add(u64::from(eip_biased));

        // Direct icache lookup without cloning BxICacheEntry.
        // We only need mpool_start_idx and tlen from the entry.
        let hash_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;
        let entry = &self.i_cache.entry[hash_idx];

        // Bochs find_entry (icache.h): a hit trusts the stored physical address
        // and the machine-wide write-stamp invalidation path. Invalid entries carry
        // the all-ones sentinel, so a real p_addr never false-hits.
        if entry.p_addr == p_addr {
            return Ok((entry.mpool_start_idx, entry.tlen as usize));
        }

        // Cache miss path
        #[cfg(feature = "profiling")]
        {
            self.perf_icache_miss += 1;
        }

        // SAFETY: prefetch() borrow is released before serve_icache_miss is called
        let miss_entry = unsafe {
            let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
            self.serve_icache_miss(eip_biased, p_addr, mem_reborrowed, cpus)?
        };
        Ok((miss_entry.mpool_start_idx, miss_entry.tlen as usize))
    }

    pub(super) fn get_gpr32(&self, idx: usize) -> u32 {
        // Must handle indices 0-15 (R8D-R15D via REX in 64-bit mode)
        // Matches set_gpr32() which uses direct array access
        self.gen_reg[idx].erx()
    }

    /// Write 32-bit GPR with zero-extension to 64 bits (Bochs BX_WRITE_32BIT_REGZ)
    /// Handles all 16 GPRs (0-7 = EAX-EDI, 8-15 = R8D-R15D)
    pub(super) fn set_gpr32(&mut self, idx: usize, val: u32) {
        // SAFETY: gen_reg union always valid for erx/hrx (32-bit) write access
        unsafe {
            self.gen_reg[idx].set_erx(val);
            self.gen_reg[idx].set_hrx(0);
        }
    }

    pub(super) fn update_flags_add32(&mut self, op1: u32, op2: u32, res: u32) {
        // Bochs SET_FLAGS_OSZAPC_ADD_32: works for ADD and ADC
        // (result already includes the carry-in from ADC).
        self.oszapc.set_oszapc_add_32(op1, op2, res);
    }

    pub(super) fn update_flags_sub32(&mut self, op1: u32, op2: u32, res: u32) {
        // Bochs SET_FLAGS_OSZAPC_SUB_32: works for SUB and SBB
        // (result already includes the borrow-in from SBB).
        self.oszapc.set_oszapc_sub_32(op1, op2, res);
    }

    // execute_instruction() is in dispatcher.rs
    // Moved 2026-02-27: ~2000-line opcode dispatch match extracted to keep cpu.rs focused on CPU loop

    // 8-bit flag updates
    pub(super) fn update_flags_add8(&mut self, op1: u8, op2: u8, result: u8) {
        self.oszapc.set_oszapc_add_8(op1, op2, result);
    }

    pub(super) fn update_flags_add16(&mut self, op1: u16, op2: u16, result: u16) {
        self.oszapc.set_oszapc_add_16(op1, op2, result);
    }

    pub(super) fn update_flags_sub8(&mut self, op1: u8, op2: u8, result: u8) {
        self.oszapc.set_oszapc_sub_8(op1, op2, result);
    }

    pub(super) fn update_flags_sub16(&mut self, op1: u16, op2: u16, result: u16) {
        self.oszapc.set_oszapc_sub_16(op1, op2, result);
    }

    pub(super) fn update_flags_logic8(&mut self, result: u8) {
        // Bochs SET_FLAGS_OSZAPC_LOGIC_8 clears OF/CF/AF, sets SF/ZF/PF from result.
        self.oszapc.set_oszapc_logic_8(result);
    }

    pub(super) fn update_flags_logic16(&mut self, result: u16) {
        self.oszapc.set_oszapc_logic_16(result);
    }

    /// Get segment base address safely
    pub(super) fn get_segment_base(&self, seg: super::decoder::BxSegregs) -> BxAddress {
        self.sregs[seg as usize].cache.u.segment_base()
    }

    /// Get segment limit safely
    pub(super) fn get_segment_limit(&self, seg: super::decoder::BxSegregs) -> u32 {
        self.sregs[seg as usize].cache.u.segment_limit_scaled()
    }

    /// Get segment d_b flag safely
    pub(super) fn get_segment_d_b(&self, seg: super::decoder::BxSegregs) -> bool {
        self.sregs[seg as usize].cache.u.segment_d_b()
    }

    /// Set segment base address safely
    pub(super) fn set_segment_base(&mut self, seg: super::decoder::BxSegregs, base: BxAddress) {
        self.sregs[seg as usize].cache.u.set_segment_base(base);
    }

    pub(super) fn update_flags_logic32(&mut self, result: u32) {
        self.oszapc.set_oszapc_logic_32(result);
    }

    // ── Bochs lazy-flag bridge (cpu.h, lazy_flags.h) ────────────────
    // These methods are the interface for lazy flag evaluation.
    // Wire individual call sites to these as you migrate from eflags.
    // See docs/future-plans/lazy-flags-read-side.md for the plan.

    /// Read a single arithmetic flag from the lazy `oszapc` store.
    #[inline]
    pub(super) fn getb_cf(&self) -> u32 {
        self.oszapc.getb_cf()
    }
    #[inline]
    pub(super) fn getb_pf(&self) -> u32 {
        self.oszapc.getb_pf()
    }
    #[inline]
    pub(super) fn getb_af(&self) -> u32 {
        self.oszapc.getb_af()
    }
    #[inline]
    pub(super) fn getb_zf(&self) -> u32 {
        self.oszapc.getb_zf()
    }
    #[inline]
    pub(super) fn getb_sf(&self) -> u32 {
        self.oszapc.getb_sf()
    }
    #[inline]
    pub(super) fn getb_of(&self) -> u32 {
        self.oszapc.getb_of()
    }

    /// Write a single arithmetic flag into the lazy `oszapc` store.
    #[inline]
    pub(super) fn set_cf(&mut self, val: bool) {
        self.oszapc.set_cf(val)
    }
    #[inline]
    pub(super) fn set_pf(&mut self, val: bool) {
        self.oszapc.set_pf(val)
    }
    #[inline]
    pub(super) fn set_af(&mut self, val: bool) {
        self.oszapc.set_af(val)
    }
    #[inline]
    pub(super) fn set_zf(&mut self, val: bool) {
        self.oszapc.set_zf(val)
    }
    #[inline]
    pub(super) fn set_sf(&mut self, val: bool) {
        self.oszapc.set_sf(val)
    }
    #[inline]
    pub(super) fn set_of(&mut self, val: bool) {
        self.oszapc.set_of(val)
    }

    /// Materialize all six arithmetic flags from `oszapc` into `eflags`.
    /// Bochs `force_flags()`. Call before any code that reads the full
    /// `eflags` register (PUSHF, LAHF, interrupt delivery, etc.).
    pub(super) fn force_flags(&mut self) {
        let new = self.oszapc.getb_cf()
            | (self.oszapc.getb_pf() << 2)
            | (self.oszapc.getb_af() << 4)
            | (self.oszapc.getb_zf() << 6)
            | (self.oszapc.getb_sf() << 7)
            | (self.oszapc.getb_of() << 11);
        let mask = EFlags::OSZAPC.bits();
        self.eflags = EFlags::from_bits_retain((self.eflags.bits() & !mask) | (new & mask));
    }

    /// Materialize lazy flags then return the full `eflags` value.
    /// Bochs `read_eflags()`.
    #[inline]
    pub(crate) fn read_eflags(&mut self) -> u32 {
        self.force_flags();
        self.eflags.bits()
    }

    /// Compute the full eflags value without mutating state.
    /// Use when only `&self` is available (snapshot, API reads).
    #[inline]
    pub(crate) fn eflags_materialized(&self) -> u32 {
        let new = self.oszapc.getb_cf()
            | (self.oszapc.getb_pf() << 2)
            | (self.oszapc.getb_af() << 4)
            | (self.oszapc.getb_zf() << 6)
            | (self.oszapc.getb_sf() << 7)
            | (self.oszapc.getb_of() << 11);
        let mask = EFlags::OSZAPC.bits();
        (self.eflags.bits() & !mask) | (new & mask)
    }

    /// Sync raw `eflags` arithmetic bits into `oszapc`.
    /// Bochs `setEFlagsOSZAPC()`. Call after any code that writes
    /// raw arithmetic bits into `eflags` (POPF, SAHF, IRET, etc.).
    pub(super) fn set_eflags_oszapc(&mut self, flags32: u32) {
        self.oszapc.set_of((flags32 >> 11) & 1 != 0);
        self.oszapc.set_sf((flags32 >> 7) & 1 != 0);
        self.oszapc.set_zf((flags32 >> 6) & 1 != 0);
        self.oszapc.set_af((flags32 >> 4) & 1 != 0);
        self.oszapc.set_pf((flags32 >> 2) & 1 != 0);
        self.oszapc.set_cf(flags32 & 1 != 0);
    }

    fn before_execution(&mut self, _cpu_id: u32) {
        // Populate RIP ring buffer for post-mortem analysis.
        // Cheap: one array write per instruction, no I/O.
        #[cfg(debug_assertions)]
        {
            let idx = self.diag_rip_ring_idx % 256;
            self.diag_rip_ring[idx] = self.rip();
            self.diag_rip_ring_idx += 1;
        }
    }

    // boundaries of consideration:
    //
    //  * physical memory boundary: 1024k (1Megabyte) (increments of...)
    //  * A20 boundary:             1024k (1Megabyte)
    //  * page boundary:            4k
    //  * ROM boundary:             2k (dont care since we are only reading)
    //  * segment boundary:         any
    pub(super) fn prefetch(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        pins: &[crate::memory::CpuTlbPin],
    ) -> Result<()> {
        let laddr: BxAddress;
        let page_offset;

        if self.long64_mode() {
            if !self.is_canonical_access(self.rip(), MemoryAccessType::Execute, self.user_pl()) {
                tracing::error!("prefetch: #GP(0): RIP crossed canonical boundary");
                self.exception(Exception::Gp, 0)?;
            }

            // linear address is equal to RIP in 64-bit long mode
            page_offset = super::tlb::page_offset(self.eip());
            laddr = self.rip();

            // Calculate RIP at the beginning of the page.
            self.eip_page_bias = u64::from(page_offset).wrapping_sub(self.rip());
            self.eip_page_window_size = 4096;
        } else {
            if self.user_pl()
                && self.get_vip() != 0
                && self.get_vif() != 0
                && (self.cr4.pvi() || (self.v8086_mode() && self.cr4.vme()))
            {
                tracing::error!("prefetch: inconsistent VME state");
                self.exception(Exception::Gp, 0)?;
            }

            self.bx_clear_64bit_high(BX_64BIT_REG_RIP); /* avoid 32-bit EIP wrap */

            // In real mode, EIP is 16-bit - mask it to prevent overflow
            // Matching behavior: ensure EIP doesn't exceed 16-bit range in real mode
            let eip_raw = self.eip();
            let eip = if self.real_mode() {
                // In real mode, EIP is effectively 16-bit (though stored as 32-bit)
                // Mask to 16 bits to match original behavior
                eip_raw & 0xFFFF
            } else {
                eip_raw
            };

            // If EIP was masked, update it (matching C++ vm8086.cc: EIP = new_eip & 0xffff)
            if self.real_mode() && eip != eip_raw {
                self.set_eip(eip);
            }

            laddr = BxAddress::from(self.get_laddr32(BxSegregs::Cs as _, eip));
            let cs_base = self.sregs[BxSegregs::Cs as usize].cache.u.segment_base();
            tracing::trace!(
                "prefetch: CS.base={:#x}, EIP={:#x}, laddr={:#x}",
                cs_base,
                eip,
                laddr
            );
            page_offset = super::tlb::page_offset(laddr);

            // Calculate RIP at the beginning of the page.
            let eip_page_bias_calc = BxAddress::from(page_offset.wrapping_sub(eip));

            let limit: u32 = self.sregs[BxSegregs::Cs as usize]
                .cache
                .u
                .segment_limit_scaled();
            if eip > limit {
                // Matching C++ cpu.cc - raise exception (does not return normally)
                tracing::error!("prefetch: EIP [{eip:#x}] > CS.limit [{limit:#x}]",);
                // In C++, exception() uses setjmp/longjmp and doesn't return here
                // In Rust, exception() returns Ok(()), but control was transferred to handler
                self.eip_page_bias = 0; // Reset to prevent using stale value
                self.exception(Exception::Gp, 0)?;
                // After exception handler runs, check if the new EIP is valid
                // If not, we're in a loop (exception handler also has invalid EIP)
                let new_eip = self.eip();
                let new_limit: u32 = self.sregs[BxSegregs::Cs as usize]
                    .cache
                    .u
                    .segment_limit_scaled();
                if new_eip > new_limit {
                    // Exception handler set invalid EIP - this would cause double-fault in real hardware
                    tracing::error!("prefetch: exception handler set invalid EIP [{new_eip:#x}] > CS.limit [{new_limit:#x}] - double-fault condition");
                    // Return error to stop infinite loop - this is a serious error condition
                    return Err(crate::cpu::CpuError::CpuNotInitialized);
                }
                // Control was transferred - abort prefetch and let retry logic handle it
                return Ok(());
            }

            // Only set eip_page_bias if limit check passed (matching C++ order)
            self.eip_page_bias = eip_page_bias_calc;

            self.eip_page_window_size = 4096;

            // Check if segment limit constrains the fetch window to less than 4096 bytes.
            // Use u64 to avoid u32 overflow when limit is 0xFFFFFFFF (flat 4GB segment).
            // Matches Bochs cpu.cc — but Bochs relies on C unsigned wrapping which
            // coincidentally produces the right behavior in most cases because the resulting
            // large eipPageWindowSize still allows eip_biased (a page offset) through.
            // We must be precise here because Rust bounds-checks the fetch buffer.
            if (limit as u64) + (self.eip_page_window_size as u64) < 4096 {
                self.eip_page_window_size = (u64::from(limit) + self.eip_page_bias + 1) as u32;
            }
        }
        // skip the
        // '''cpp
        // '#if BX_X86_DEBUGGER
        // '''
        self.clear_rf();
        let lpf = lpf_of(laddr);

        // Check TLB entry - extract values to avoid holding mutable borrow
        let (tlb_hit, tlb_ppf, tlb_host_addr) = {
            let tlb_entry = self.itlb.get_entry_of(laddr, 0);
            let hit = (tlb_entry.lpf == lpf)
                && (tlb_entry.access_bits & (1 << u32::from(self.user_pl))) != 0;
            (hit, tlb_entry.ppf, tlb_entry.host_page_addr)
        };

        // Track whether translate_linear succeeded so we can populate the iTLB afterward.
        let mut itlb_should_update = false;

        let fetch_ptr_option = if tlb_hit {
            self.p_addr_fetch_page = tlb_ppf;
            // Bochs cpu/cpu.cc prefetch path does NOT speculatively
            // populate the DTLB on an ITLB hit. The earlier rusty_box
            // workaround that called `translate_data_read(laddr)` here
            // could synchronously raise #PF (mutating CR2 + pushing the
            // exception frame) for an unrelated data access, then
            // swallow the `CpuLoopRestart` and continue prefetching
            // from a stale RIP. Removed to match Bochs.
            Some(tlb_host_addr)
        } else {
            // The direct-mapped slot may still pin an unrelated resident
            // page.  Its mapping and prefetch slice must be gone before the
            // page walk or direct mapping can ask the block allocator to
            // replace it.
            self.invalidate_itlb_pin_slot(laddr, 0);
            // TLB miss - need to walk page tables
            // Get a20_mask before borrowing mem mutably
            let a20_mask = mem.a20_mask();
            // Create a dummy TLB entry (not actually used for page walk)
            let dummy_tlb_entry = TLBEntry::default();
            match self.translate_linear(
                &dummy_tlb_entry,
                laddr,
                self.user_pl,
                MemoryAccessType::Execute,
                a20_mask,
                mem,
            ) {
                Ok(p_addr) => {
                    self.p_addr_fetch_page = ppf_of(p_addr);
                    itlb_should_update = true;
                    tracing::trace!(
                        "prefetch: translate_linear OK, p_addr={:#x}, p_addr_fetch_page={:#x}",
                        p_addr,
                        self.p_addr_fetch_page
                    );
                    // Bochs `BX_CPU_C::prefetch` (cpu.cc) does NOT
                    // populate the DTLB after an ITLB miss — only the
                    // ITLB entry it just walked. The earlier rusty_box
                    // workaround that called `translate_data_read(laddr)`
                    // here could synchronously raise #PF (mutating CR2 +
                    // pushing the exception frame) for an unrelated data
                    // access, then swallow the `CpuLoopRestart` and
                    // continue prefetching from a stale RIP. Removed to
                    // match Bochs.
                    None
                }
                Err(e) => {
                    // Page fault or other exception occurred during page walk.
                    // The exception handler has already pushed the exception frame
                    // and changed RIP. Propagate the error (CpuLoopRestart) so the
                    // CPU loop restarts execution at the exception handler.
                    // Previously this was silently swallowed, causing boundary_fetch
                    // to continue with stale eip_page_window_size=0 and panic.
                    return Err(e);
                }
            }
        };

        let mut direct_page_mapping = false;
        if let Some(fetch_ptr) = fetch_ptr_option {
            let fetch_ptr_as_ptr =
                // SAFETY: an ITLB entry is installed only for a complete
                // contiguous host page (see below).
                unsafe { super::access::host_slice_u8(fetch_ptr as *const u8, 4096) };
            self.eip_fetch_ptr = Some(fetch_ptr_as_ptr);
            direct_page_mapping = true;
        } else {
            let mem_len = mem.get_memory_len();
            let page_base = self.p_addr_fetch_page;
            let current_p_addr = page_base.wrapping_add(u64::from(page_offset));
            let page_policy = self.memory_access_policy(mem.a20_addr(page_base));

            let mem_ptr: *mut BxMemC<'c> = mem;
            let page_mapping = unsafe {
                (&mut *mem_ptr)
                    .get_host_mem_addr_pinned(
                        page_base,
                        MemoryAccessType::Execute,
                        pins,
                        page_policy,
                    )
                    .map(|mapping| mapping.map(|slice| (slice.as_mut_ptr(), slice.len())))
            };
            match page_mapping {
                Ok(Some((fetch_ptr, fetch_len))) if fetch_len >= 4096 => {
                    // An ITLB host page is necessarily a full contiguous page.
                    self.eip_fetch_ptr = Some(unsafe {
                        super::access::host_slice_u8(fetch_ptr as *const u8, 4096)
                    });
                    direct_page_mapping = true;
                }
                Ok(Some(_)) => {
                    // Guest blocks may be smaller than a page and may live in
                    // unrelated host slots.  Fetch from the current byte only,
                    // cap the prefetch window at that resident block, and let
                    // the next lookup refill at its boundary.  It must never
                    // create an ITLB page mapping from this short slice.
                    let current_policy =
                        self.memory_access_policy(mem.a20_addr(current_p_addr));
                    let current_mapping = unsafe {
                        (&mut *mem_ptr)
                            .get_host_mem_addr_pinned(
                                current_p_addr,
                                MemoryAccessType::Execute,
                                pins,
                                current_policy,
                            )
                            .map(|mapping| {
                                mapping.map(|slice| (slice.as_mut_ptr(), slice.len()))
                            })
                    };
                    match current_mapping {
                        Ok(Some((fetch_ptr, available))) => {
                            let page_remaining = self
                                .eip_page_window_size
                                .saturating_sub(page_offset) as usize;
                            let fetch_len = available.min(page_remaining);
                            if fetch_len != 0 {
                                self.p_addr_fetch_page = current_p_addr;
                                self.eip_page_bias = 0u64.wrapping_sub(self.rip());
                                self.eip_page_window_size = fetch_len
                                    .try_into()
                                    .expect("resident block length exceeds u32");
                                self.eip_fetch_ptr = Some(unsafe {
                                    super::access::host_slice_u8(
                                        fetch_ptr as *const u8,
                                        fetch_len,
                                    )
                                });
                            } else {
                                self.eip_fetch_ptr = None;
                            }
                        }
                        Ok(None) | Err(_) => {
                            self.eip_fetch_ptr = None;
                        }
                    }
                }
                Ok(None) => {
                    self.eip_fetch_ptr = None;
                }
                Err(error) => {
                    tracing::trace!("Failed to get host mem addr for fetch: {error:?}");
                    self.eip_fetch_ptr = None;
                }
            }
            // Only complete page mappings can be placed in the ITLB.  In
            // particular, sub-page guest blocks are not host-contiguous.
            if itlb_should_update && direct_page_mapping {
                if let Some(fp) = self.eip_fetch_ptr {
                    let host_page_ptr = fp.as_ptr() as super::tlb::BxHostpageaddr;
                    let ppf = self.p_addr_fetch_page;
                    let access_bits = 1u32 << (self.user_pl as u32);
                    let tlb_entry = self.itlb.get_entry_of(lpf, 0);
                    tlb_entry.lpf = lpf;
                    tlb_entry.ppf = ppf;
                    tlb_entry.access_bits = access_bits;
                    tlb_entry.lpf_mask = 0xFFF;
                    tlb_entry.host_page_addr = host_page_ptr;
                }
                self.sync_itlb_pin_slot(lpf, 0);
            }
            let eip_biased =
                (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;
            let p_addr = self.p_addr_fetch_page.wrapping_add(u64::from(eip_biased));
            if self.eip_fetch_ptr.is_none() && p_addr >= mem_len.try_into()? {
                // Address is beyond available memory - set to no direct access
                tracing::trace!("prefetch: address {p_addr:#x} beyond memory limit {mem_len:#x} and no ROM mapping");
                self.eip_fetch_ptr = None;
            }
        }

        // Publish the final fetch window from every arm above, including
        // full-page windows installed without a matching ITLB slot.
        self.sync_fetch_window_pin();

        Ok(())
    }

    pub(super) fn long64_mode(&self) -> bool {
        self.cpu_mode == CpuMode::Long64
    }

    /// Returns true when CPU is in long mode (either 64-bit or compatibility sub-mode).
    /// Matches Bochs `long_mode()` which checks `EFER.LMA == 1`.
    pub(super) fn long_mode(&self) -> bool {
        self.cpu_mode == CpuMode::Long64 || self.cpu_mode == CpuMode::LongCompat
    }

    pub(crate) fn smm_mode(&self) -> bool {
        self.in_smm
    }

    // =========================================================================
    // Error handlers matching original C++ BxError, BxNoFPU, etc.
    // =========================================================================

    /// BxError - Invalid instruction handler
    /// Matches BX_CPU_C::BxError from proc_ctrl.cc
    /// Raises #UD (Undefined Instruction) exception
    pub(super) fn bx_error(&mut self, instr: &Instruction) -> Result<()> {
        let opcode = instr.get_ia_opcode();

        if opcode == crate::cpu::decoder::Opcode::IaError {
            tracing::trace!("BxError: Encountered an unknown instruction (signalling #UD)");
        } else {
            tracing::trace!("{:?}: instruction not supported - signalling #UD", opcode);
        }

        // Boot diagnostic: report the first unsupported opcode via port 0xE9.
        // If BIOS hits #UD early, it may vector to 0000:0000 and appear to “do nothing”.
        if (self.boot_debug_flags & 0x01) == 0 {
            self.boot_debug_flags |= 0x01;
            self.debug_puts(b"[UD]\n");
        }

        // Unicorn-inspired: give hooks a chance to suppress #UD for unrecognized opcodes
        #[cfg(feature = "instrumentation")]
        if self.instrumentation.active.has_invalid_insn() {
            if self.instrumentation.fire_invalid_instruction(self.prev_rip) {
                return Ok(());
            }
        }

        self.exception(Exception::Ud, 0)?;
        Ok(())
    }

    /// BxNoFPU - FPU not available handler
    /// Matches BX_CPU_C::BxNoFPU from proc_ctrl.cc
    /// Raises #NM (Device Not Available) if CR0.EM or CR0.TS is set
    pub(super) fn bx_no_fpu(&mut self, _instr: &Instruction) -> Result<()> {
        let cr0 = self.cr0.get32();
        let cr0_em = (cr0 & (1 << 2)) != 0; // CR0.EM bit 2
        let cr0_ts = (cr0 & (1 << 3)) != 0; // CR0.TS bit 3

        if cr0_em || cr0_ts {
            self.exception(Exception::Nm, 0)?;
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoFPU: FPU instruction executed but FPU not available");
        Ok(())
    }

    /// BxNoMMX - MMX not available handler
    /// Matches BX_CPU_C::BxNoMMX from proc_ctrl.cc
    /// Raises #UD if CR0.EM is set, #NM if CR0.TS is set
    pub(super) fn bx_no_mmx(&mut self, _instr: &Instruction) -> Result<()> {
        let cr0 = self.cr0.get32();
        let cr0_em = (cr0 & (1 << 2)) != 0; // CR0.EM bit 2
        let cr0_ts = (cr0 & (1 << 3)) != 0; // CR0.TS bit 3

        if cr0_em {
            self.exception(Exception::Ud, 0)?;
        }

        if cr0_ts {
            self.exception(Exception::Nm, 0)?;
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoMMX: MMX instruction executed but MMX not available");
        Ok(())
    }

    /// BxNoSSE - SSE not available handler
    /// Matches BX_CPU_C::BxNoSSE from proc_ctrl.cc
    /// Only available if CPU_LEVEL >= 6
    /// Raises #UD if CR0.EM is set or CR4.OSFXSR is clear, #NM if CR0.TS is set
    pub(super) fn bx_no_sse(&mut self, instr: &Instruction) -> Result<()> {
        let cr0 = self.cr0.get32();
        let cr4 = self.cr4.get32();
        let cr0_em = (cr0 & (1 << 2)) != 0; // CR0.EM bit 2
        let cr0_ts = (cr0 & (1 << 3)) != 0; // CR0.TS bit 3
        let cr4_osfxsr = (cr4 & (1 << 9)) != 0; // CR4.OSFXSR bit 9

        if cr0_em || !cr4_osfxsr {
            self.exception(Exception::Ud, 0)?;
        }

        if cr0_ts {
            self.exception(Exception::Nm, 0)?;
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoSSE: SSE instruction executed but SSE not available");
        Ok(())
    }

    /// BxNoAVX - AVX not available handler
    /// Matches BX_CPU_C::BxNoAVX from proc_ctrl.cc
    /// Only available if BX_SUPPORT_AVX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    pub(super) fn bx_no_avx(&mut self, _instr: &Instruction) -> Result<()> {
        self.prepare_avx()
    }

    /// BxNoOpMask - Opmask not available handler
    /// Matches BX_CPU_C::BxNoOpMask from proc_ctrl.cc
    /// Only available if BX_SUPPORT_EVEX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    pub(super) fn bx_no_opmask(&mut self, instr: &Instruction) -> Result<()> {
        // Check if in protected mode (CR0.PE = 1)
        let cr0 = self.cr0.get32();
        let cr0_pe = (cr0 & (1 << 0)) != 0; // CR0.PE bit 0
        if !cr0_pe {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        let cr4 = self.cr4.get32();
        let cr4_osxsave = (cr4 & (1 << 18)) != 0; // CR4.OSXSAVE bit 18

        if !cr4_osxsave {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        // Check XCR0 for SSE, YMM, and OPMASK masks
        let xcr0 = self.xcr0.get32();
        const XCR0_SSE_MASK: u32 = 1 << 0;
        const XCR0_YMM_MASK: u32 = 1 << 2;
        const XCR0_OPMASK_MASK: u32 = 1 << 5;
        if (xcr0 & (XCR0_SSE_MASK | XCR0_YMM_MASK | XCR0_OPMASK_MASK))
            != (XCR0_SSE_MASK | XCR0_YMM_MASK | XCR0_OPMASK_MASK)
        {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        let cr0_ts = (cr0 & (1 << 3)) != 0; // CR0.TS bit 3
        if cr0_ts {
            self.exception(Exception::Nm, 0)?;
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoOpMask: Opmask instruction executed but Opmask not available");
        Ok(())
    }

    /// BxNoEVEX - EVEX not available handler
    /// Matches BX_CPU_C::BxNoEVEX from proc_ctrl.cc
    /// Only available if BX_SUPPORT_EVEX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    pub(super) fn bx_no_evex(&mut self, instr: &Instruction) -> Result<()> {
        // Check if in protected mode (CR0.PE = 1)
        let cr0 = self.cr0.get32();
        let cr0_pe = (cr0 & (1 << 0)) != 0; // CR0.PE bit 0
        if !cr0_pe {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        let cr4 = self.cr4.get32();
        let cr4_osxsave = (cr4 & (1 << 18)) != 0; // CR4.OSXSAVE bit 18

        if !cr4_osxsave {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        // Check XCR0 for SSE, YMM, OPMASK, ZMM_HI256, and HI_ZMM masks
        let xcr0 = self.xcr0.get32();
        const XCR0_SSE_MASK: u32 = 1 << 0;
        const XCR0_YMM_MASK: u32 = 1 << 2;
        const XCR0_OPMASK_MASK: u32 = 1 << 5;
        const XCR0_ZMM_HI256_MASK: u32 = 1 << 6;
        const XCR0_HI_ZMM_MASK: u32 = 1 << 7;
        if (xcr0
            & (XCR0_SSE_MASK
                | XCR0_YMM_MASK
                | XCR0_OPMASK_MASK
                | XCR0_ZMM_HI256_MASK
                | XCR0_HI_ZMM_MASK))
            != (XCR0_SSE_MASK
                | XCR0_YMM_MASK
                | XCR0_OPMASK_MASK
                | XCR0_ZMM_HI256_MASK
                | XCR0_HI_ZMM_MASK)
        {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        let cr0_ts = (cr0 & (1 << 3)) != 0; // CR0.TS bit 3
        if cr0_ts {
            self.exception(Exception::Nm, 0)?;
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoEVEX: EVEX instruction executed but EVEX not available");
        Ok(())
    }

    /// BxNoAMX - AMX not available handler
    /// Matches BX_CPU_C::BxNoAMX from proc_ctrl.cc
    /// Only available if BX_SUPPORT_AMX
    /// Raises #UD if not in long64 mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    pub(super) fn bx_no_amx(&mut self, instr: &Instruction) -> Result<()> {
        if !self.long64_mode() {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        let cr4 = self.cr4.get32();
        let cr4_osxsave = (cr4 & (1 << 18)) != 0; // CR4.OSXSAVE bit 18

        if !cr4_osxsave {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        // Check XCR0 for XTILECFG and XTILEDATA masks
        let xcr0 = self.xcr0.get32();
        const XCR0_XTILECFG_MASK: u32 = 1 << 17;
        const XCR0_XTILEDATA_MASK: u32 = 1 << 18;
        if (xcr0 & (XCR0_XTILECFG_MASK | XCR0_XTILEDATA_MASK))
            != (XCR0_XTILECFG_MASK | XCR0_XTILEDATA_MASK)
        {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoAMX: AMX instruction executed but AMX not available");
        Ok(())
    }

    // =========================================================================
    // Handler assignment (assign_handler) matching original C++ assignHandler
    // =========================================================================

    /// Assign handler function for instruction execution
    ///
    /// This function selects the appropriate handler function for an instruction based on:
    /// - The instruction opcode
    /// - Whether it's a memory form (modC0 == false) or register form (modC0 == true)
    /// - Special cases (e.g., MOV with SS segment override)
    /// - Feature availability (FPU, MMX, SSE, AVX, EVEX, OPMASK, AMX)
    /// - EVEX-specific rules (broadcast, SAE)
    ///
    /// Matching C++ `BX_CPU_C::assignHandler` in fetchdecode32.cc
    ///
    /// # Parameters
    /// - `instr`: The instruction to assign a handler for
    /// - `fetch_mode_mask`: Bitmask indicating which features are currently available
    ///
    /// # Returns
    /// - `Ok((should_stop_trace, handler_opt))`:
    ///   - `should_stop_trace`: `true` if trace should end (TRACE_END flag set or error handler assigned)
    ///   - `handler_opt`: The selected handler function, or `None` if opcode not in table
    ///
    /// # Special Cases
    /// - MOV with SS segment override uses MOV32S handlers (stack_read_dword/stack_write_dword)
    /// - Instructions requiring unavailable features get error handlers (BxNoFPU, BxNoMMX, etc.)
    /// - EVEX instructions with invalid broadcast/SAE get BxError handler
    pub(crate) fn assign_handler(
        &mut self,
        instr: &mut Instruction,
        fetch_mode_mask: super::opcodes_table::FetchModeMask,
    ) -> Result<(bool, Option<InstructionHandler<I, T>>)> {
        use super::opcodes_table::{get_opcode_entry, FetchModeMask, OpFlags};
        use crate::cpu::decoder::Opcode;

        let ia_opcode = instr.get_ia_opcode();
        let opcode_entry = get_opcode_entry(ia_opcode);

        // Get opflags from table entry, or use empty if not in table yet
        let op_flags = opcode_entry
            .as_ref()
            .map(|e| e.opflags)
            .unwrap_or(OpFlags::empty());

        // Check modC0 (register form vs memory form)
        let is_reg_form = instr.mod_c0();

        // Handler assignment logic (matching original lines 2045-2061)
        let mut selected_handler: Option<InstructionHandler<I, T>> = None;
        let mut is_bx_error = false; // Track if BxError handler was assigned

        if let Some(entry) = &opcode_entry {
            // Handler assignment from table
            if !is_reg_form {
                // Memory form: use execute1 from table (matching line 2046)
                selected_handler = Some(entry.execute1);

                // Special case: MOV with SS segment override (matching lines 2049-2056)
                if ia_opcode == Opcode::MovOp32GdEd && instr.seg() == BxSegregs::Ss as u8 {
                    // Use MOV32S_GdEdM handler (matching C++ line 2051)
                    use super::opcodes_table::mov32s_gd_ed_m_wrapper;
                    selected_handler = Some(mov32s_gd_ed_m_wrapper);
                }
                if ia_opcode == Opcode::MovOp32EdGd && instr.seg() == BxSegregs::Ss as u8 {
                    // Use MOV32S_EdGdM handler (matching C++ line 2055)
                    use super::opcodes_table::mov32s_ed_gd_m_wrapper;
                    selected_handler = Some(mov32s_ed_gd_m_wrapper);
                }
            } else {
                // Register form: use execute2 from table as execute1 (matching line 2059)
                if let Some(execute2) = entry.execute2 {
                    selected_handler = Some(execute2);
                } else {
                    // No register form handler - fall back to execute_instruction
                    return Ok((false, None));
                }
            }
        } else {
            // Opcode not in table yet - will use execute_instruction match statement
            return Ok((false, None));
        }

        // EVEX-specific checks (matching lines 2067-2084)
        // These checks assign BxError IMMEDIATELY if EVEX rules are violated
        {
            if op_flags.contains(OpFlags::PREPARE_EVEX) {
                if instr.get_evex_b() != 0 {
                    if !is_reg_form {
                        // Memory form: check NO_BROADCAST
                        if op_flags.contains(OpFlags::PREPARE_EVEX_NO_BROADCAST) {
                            tracing::trace!(
                                "{:?}: broadcast is not supported for this instruction",
                                ia_opcode
                            );
                            // Matching C++ line 2073: assign BxError immediately
                            selected_handler = Some(super::opcodes_table::bx_error_wrapper);
                            is_bx_error = true;
                        }
                    } else {
                        // Register form: check NO_SAE
                        if op_flags.contains(OpFlags::PREPARE_EVEX_NO_SAE) {
                            tracing::trace!(
                                "{:?}: EVEX.b in reg form is not allowed for instructions which cannot cause floating point exception",
                                ia_opcode
                            );
                            // Matching C++ line 2079: assign BxError immediately
                            selected_handler = Some(super::opcodes_table::bx_error_wrapper);
                            is_bx_error = true;
                        }
                    }
                }
            }
        }

        // Feature availability checks (matching lines 2086-2133)
        // These checks only assign error handlers if execute1 != BxError (matching C++ lines 2088, 2092, etc.)
        // Check FPU/MMX availability
        if !fetch_mode_mask.contains(FetchModeMask::FPU_MMX_OK) {
            if op_flags.contains(OpFlags::PREPARE_FPU) {
                // Matching C++ line 2088: only assign if execute1 != BxError
                if !is_bx_error {
                    use super::opcodes_table::bx_no_fpu_wrapper;
                    selected_handler = Some(bx_no_fpu_wrapper);
                }
                return Ok((true, selected_handler)); // Stop trace
            }
            if op_flags.contains(OpFlags::PREPARE_MMX) {
                // Matching C++ line 2092: only assign if execute1 != BxError
                if !is_bx_error {
                    use super::opcodes_table::bx_no_mmx_wrapper;
                    selected_handler = Some(bx_no_mmx_wrapper);
                }
                return Ok((true, selected_handler)); // Stop trace
            }
        }

        // Check SSE availability (CPU_LEVEL >= 6)
        {
            if !fetch_mode_mask.contains(FetchModeMask::SSE_OK) {
                if op_flags.contains(OpFlags::PREPARE_SSE) {
                    // Matching C++ line 2099: only assign if execute1 != BxError
                    if !is_bx_error {
                        use super::opcodes_table::bx_no_sse_wrapper;
                        selected_handler = Some(bx_no_sse_wrapper);
                    }
                    return Ok((true, selected_handler)); // Stop trace
                }
            }
        }

        // Check AVX availability
        {
            if !fetch_mode_mask.contains(FetchModeMask::AVX_OK) {
                if op_flags.contains(OpFlags::PREPARE_AVX) {
                    // Matching C++ line 2106: only assign if execute1 != BxError
                    if !is_bx_error {
                        use super::opcodes_table::bx_no_avx_wrapper;
                        selected_handler = Some(bx_no_avx_wrapper);
                    }
                    return Ok((true, selected_handler)); // Stop trace
                }
            }
        }

        // Check OPMASK availability
        {
            if !fetch_mode_mask.contains(FetchModeMask::OPMASK_OK) {
                if op_flags.contains(OpFlags::PREPARE_OPMASK) {
                    // Matching C++ line 2113: only assign if execute1 != BxError
                    if !is_bx_error {
                        use super::opcodes_table::bx_no_opmask_wrapper;
                        selected_handler = Some(bx_no_opmask_wrapper);
                    }
                    return Ok((true, selected_handler)); // Stop trace
                }
            }
        }

        // Check EVEX availability
        {
            if !fetch_mode_mask.contains(FetchModeMask::EVEX_OK) {
                if op_flags.contains(OpFlags::PREPARE_EVEX) {
                    // Matching C++ line 2119: only assign if execute1 != BxError
                    if !is_bx_error {
                        use super::opcodes_table::bx_no_evex_wrapper;
                        selected_handler = Some(bx_no_evex_wrapper);
                    }
                    return Ok((true, selected_handler)); // Stop trace
                }
            }
        }

        // Check AMX availability
        {
            if !fetch_mode_mask.contains(FetchModeMask::AMX_OK)
                && op_flags.contains(OpFlags::PREPARE_AMX)
            {
                // Matching C++ line 2126: only assign if execute1 != BxError
                if !is_bx_error {
                    use super::opcodes_table::bx_no_amx_wrapper;
                    selected_handler = Some(bx_no_amx_wrapper);
                }
                return Ok((true, selected_handler)); // Stop trace
            }
        }

        // Check if trace should end (matching line 2135)
        // Original: if ((op_flags & BX_TRACE_END) != 0 || i->execute1 == &BX_CPU_C::BxError)
        if op_flags.contains(OpFlags::TRACE_END) || is_bx_error {
            return Ok((true, selected_handler)); // Stop trace
        }

        // Return handler for execution
        Ok((false, selected_handler))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cpu::{
            builder::BxCpuBuilder,
            cpudb::intel::core_i7_skylake::Corei7SkylakeX,
            decoder::BxSegregs,
            crregs::{BxCr0, BxCr4},
        },
        memory::{BxMemC, BxMemoryStubC, CpuTlbPin},
        params::BxParams,
        pc_system::{BxPcSystemC, TimerOwner},
    };

    #[test]
    fn fetch_window_pin_cleared_by_itlb_invalidation() {
        // Bochs cpu.cc prefetch: `eipFetchPtr` remains valid until the next
        // refill. The publisher must pin its exact host span and the ITLB
        // invalidation path must clear both the pointer and the pin.
        static CODE: [u8; 0x80] = [0x90; 0x80];

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        let pins = core::slice::from_ref(&pin);
        cpu.wire_memory_access(NonNull::from(&mut mem), pins, &pin);

        cpu.eip_fetch_ptr = Some(&CODE);
        cpu.sync_fetch_window_pin();
        let start = CODE.as_ptr() as usize;
        assert!(pin.is_range_pinned(start, start + CODE.len()));
        // Only the exact window is pinned, not neighbouring ranges.
        assert!(!pin.is_range_pinned(start + CODE.len(), start + 2 * CODE.len()));

        cpu.invalidate_itlb_pin_slot(0, 0);
        assert!(cpu.eip_fetch_ptr.is_none());
        assert!(!pin.is_range_pinned(start, start + CODE.len()));

        // The full-rescan recovery path republishes a live window.
        cpu.eip_fetch_ptr = Some(&CODE);
        cpu.refresh_tlb_pin(&pin);
        assert!(pin.is_range_pinned(start, start + CODE.len()));

        cpu.clear_memory_access();
    }

    #[test]
    fn page_walk_direct_ad_writes_invalidate_dword_and_qword_cached_code() {
        const CODE_PAGE: u64 = 0xc000;
        const LEGACY_DIRECTORY: u64 = 0x1000;
        const LEGACY_TABLE: u64 = 0x2000;
        const PAE_DIRECTORY: u64 = 0x3000;
        const PAE_TABLE: u64 = 0x4000;
        const PAGE_ENTRY: u64 = CODE_PAGE | 0x3; // present + writable

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.linaddr_width = 48;
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let mem_ptr: *mut BxMemC<'_> = &mut mem;
        let pin = CpuTlbPin::new(&cpu);
        let pins = core::slice::from_ref(&pin);
        cpu.wire_memory_access(NonNull::from(&mut mem), pins, &pin);
        let (host_base, host_len) = mem.identity_guest_base();
        cpu.mem_host_base = host_base;
        cpu.mem_host_len = host_len;

        // The leaf PTE itself starts as executable code.  Cache it through the
        // normal prefetch/decode path while paging is off, then make a real
        // legacy system write walk update that physical cache line.
        mem.write_ram(
            pins,
            LEGACY_DIRECTORY,
            &(LEGACY_TABLE | 0x3).to_le_bytes(),
        )
        .unwrap();
        mem.write_ram(pins, LEGACY_TABLE, &PAGE_ENTRY.to_le_bytes())
            .unwrap();
        cpu.cpu_mode = CpuMode::Long64;
        cpu.cr0 = BxCr0::empty();
        cpu.cr4 = BxCr4::empty();
        cpu.set_rip(LEGACY_TABLE);
        cpu.prev_rip = LEGACY_TABLE;
        let (legacy_mpool, _) = unsafe { cpu.get_icache_entry(&mut *mem_ptr, pins) }.unwrap();
        let legacy_mode = cpu.fetch_mode_mask.bits().into();
        assert!(cpu.i_cache.find_entry(LEGACY_TABLE, legacy_mode).is_some());
        assert!(mem.smc_range_has_stamps(LEGACY_TABLE, 4));

        cpu.invalidate_prefetch_q();
        cpu.cpu_mode = CpuMode::Ia32Protected;
        cpu.cr0 = BxCr0::PE | BxCr0::PG;
        cpu.cr3 = LEGACY_DIRECTORY;
        cpu.async_event = 0;
        assert_eq!(
            cpu.translate_linear_system_write(0).unwrap(),
            CODE_PAGE,
            "the legacy direct system-write walk must resolve the leaf"
        );
        let mut legacy_pte = [0; 4];
        mem.read_ram(pins, LEGACY_TABLE, &mut legacy_pte).unwrap();
        assert_eq!(
            u32::from_le_bytes(legacy_pte) & 0x60,
            0x60,
            "the legacy direct dword writer must persist A+D"
        );
        assert!(
            cpu.i_cache.find_entry(LEGACY_TABLE, legacy_mode).is_none(),
            "the live cached legacy trace must not survive the PTE write"
        );
        assert_ne!(cpu.async_event, 0);

        // Re-enter through the CPU cache lookup and cached dispatch path.  A
        // stale primary cache entry would make this a hit on the pre-write
        // trace instead of decoding the changed PTE bytes.
        cpu.async_event = 0;
        cpu.invalidate_prefetch_q();
        cpu.cpu_mode = CpuMode::Long64;
        cpu.cr0 = BxCr0::empty();
        cpu.set_rip(LEGACY_TABLE);
        cpu.prev_rip = LEGACY_TABLE;
        let (legacy_reloaded, _) =
            unsafe { cpu.get_icache_entry(&mut *mem_ptr, pins) }.unwrap();
        assert_ne!(legacy_reloaded, legacy_mpool);
        let legacy_instr = cpu.i_cache.mpool[legacy_reloaded];
        cpu.execute_instruction(&legacy_instr).unwrap();

        // Repeat with the PAE direct qword walker.  Its PTE line is likewise
        // decoded into a live trace before the architectural A/D write.
        mem.write_ram(pins, PAE_DIRECTORY, &(PAE_TABLE | 0x3).to_le_bytes())
            .unwrap();
        mem.write_ram(pins, PAE_TABLE, &PAGE_ENTRY.to_le_bytes())
            .unwrap();
        cpu.invalidate_prefetch_q();
        cpu.set_rip(PAE_TABLE);
        cpu.prev_rip = PAE_TABLE;
        let (pae_mpool, _) = unsafe { cpu.get_icache_entry(&mut *mem_ptr, pins) }.unwrap();
        let pae_mode = cpu.fetch_mode_mask.bits().into();
        assert!(cpu.i_cache.find_entry(PAE_TABLE, pae_mode).is_some());
        assert!(mem.smc_range_has_stamps(PAE_TABLE, 8));

        cpu.invalidate_prefetch_q();
        cpu.cpu_mode = CpuMode::Ia32Protected;
        cpu.cr0 = BxCr0::PE | BxCr0::PG;
        cpu.cr4 = BxCr4::PAE;
        cpu.pdptrcache.entry[0] = PAE_DIRECTORY | 0x1;
        cpu.async_event = 0;
        assert_eq!(
            cpu.translate_linear_system_write(0).unwrap(),
            CODE_PAGE,
            "the PAE direct system-write walk must resolve the leaf"
        );
        let mut pae_pte = [0; 8];
        mem.read_ram(pins, PAE_TABLE, &mut pae_pte).unwrap();
        assert_eq!(
            u64::from_le_bytes(pae_pte) & 0x60,
            0x60,
            "the PAE direct qword writer must persist A+D"
        );
        assert!(
            cpu.i_cache.find_entry(PAE_TABLE, pae_mode).is_none(),
            "the live cached PAE trace must not survive the PTE write"
        );
        assert_ne!(cpu.async_event, 0);

        cpu.async_event = 0;
        cpu.invalidate_prefetch_q();
        cpu.cpu_mode = CpuMode::Long64;
        cpu.cr0 = BxCr0::empty();
        cpu.cr4 = BxCr4::empty();
        cpu.set_rip(PAE_TABLE);
        cpu.prev_rip = PAE_TABLE;
        let (pae_reloaded, _) =
            unsafe { cpu.get_icache_entry(&mut *mem_ptr, pins) }.unwrap();
        assert_ne!(pae_reloaded, pae_mpool);
        let pae_instr = cpu.i_cache.mpool[pae_reloaded];
        cpu.execute_instruction(&pae_instr).unwrap();


        cpu.clear_memory_access();
    }

    #[test]
    fn page_split_trace_is_invalidated_by_tlb_break_links() {
        const PAGE_DIRECTORY: u64 = 0x1000;
        const PAGE_TABLE: u64 = 0x2000;
        const FIRST_CODE_PAGE: u64 = 0x3000;
        const OLD_SECOND_CODE_PAGE: u64 = 0x4000;
        const NEW_SECOND_CODE_PAGE: u64 = 0x5000;
        const SPLIT_RIP: u64 = 0x0ffe;

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.linaddr_width = 48;
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let mem_ptr: *mut BxMemC<'_> = &mut mem;
        let pin = CpuTlbPin::new(&cpu);
        let pins = core::slice::from_ref(&pin);
        cpu.wire_memory_access(NonNull::from(&mut mem), pins, &pin);
        let (host_base, host_len) = mem.identity_guest_base();
        cpu.mem_host_base = host_base;
        cpu.mem_host_len = host_len;

        // Flat 32-bit protected code with two virtual pages.  The five-byte
        // MOV begins two bytes before the boundary, so `get_icache_entry`
        // must decode a genuine page-split trace whose immediate comes from
        // the second mapping.
        let cs = &mut cpu.sregs[BxSegregs::Cs as usize];
        cs.cache.u.set_segment_base(0);
        cs.cache.u.set_segment_limit_scaled(u32::MAX);
        cs.cache.u.set_segment_d_b(true);
        cs.cache.u.set_segment_l(false);
        cpu.cpu_mode = CpuMode::Ia32Protected;
        cpu.cr0 = BxCr0::PE | BxCr0::PG;
        cpu.cr4 = BxCr4::empty();
        cpu.cr3 = PAGE_DIRECTORY;
        cpu.update_fetch_mode_mask();

        mem.write_ram(pins, PAGE_DIRECTORY, &(PAGE_TABLE | 0x3).to_le_bytes())
            .unwrap();
        mem.write_ram(pins, PAGE_TABLE, &(FIRST_CODE_PAGE | 0x3).to_le_bytes())
            .unwrap();
        mem.write_ram(
            pins,
            PAGE_TABLE + 4,
            &(OLD_SECOND_CODE_PAGE | 0x3).to_le_bytes(),
        )
        .unwrap();
        mem.write_ram(pins, FIRST_CODE_PAGE + 0x0ffe, &[0xb8, 0x78])
            .unwrap();
        mem.write_ram(pins, OLD_SECOND_CODE_PAGE, &[0x56, 0x34, 0x12])
            .unwrap();
        mem.write_ram(pins, NEW_SECOND_CODE_PAGE, &[0x99, 0x88, 0x77])
            .unwrap();

        cpu.set_rip(SPLIT_RIP);
        cpu.prev_rip = SPLIT_RIP;
        let (old_mpool, _) = unsafe { cpu.get_icache_entry(&mut *mem_ptr, pins) }.unwrap();
        let fetch_mode = cpu.fetch_mode_mask.bits().into();
        assert!(
            cpu.i_cache.find_entry(FIRST_CODE_PAGE + 0x0ffe, fetch_mode).is_some(),
            "normal lookup must commit the primary page-split trace"
        );
        let old_instr = cpu.i_cache.mpool[old_mpool];
        cpu.execute_instruction(&old_instr).unwrap();
        assert_eq!(
            cpu.get_gpr32(0),
            0x1234_5678,
            "the initial split trace must consume bytes from the old second page"
        );

        // Remap only the second virtual page and perform the architectural TLB
        // link break.  No write touches the first-page code line: correctness
        // depends specifically on invalidating the primary live split entry.
        mem.write_ram(
            pins,
            PAGE_TABLE + 4,
            &(NEW_SECOND_CODE_PAGE | 0x3).to_le_bytes(),
        )
        .unwrap();
        cpu.tlb_flush();
        assert!(
            cpu.i_cache
                .find_entry(FIRST_CODE_PAGE + 0x0ffe, fetch_mode)
                .is_none(),
            "TLB break_links must invalidate the live primary split trace"
        );

        cpu.async_event = 0;
        cpu.set_rip(SPLIT_RIP);
        cpu.prev_rip = SPLIT_RIP;
        let (new_mpool, _) = unsafe { cpu.get_icache_entry(&mut *mem_ptr, pins) }.unwrap();
        assert_ne!(
            new_mpool, old_mpool,
            "a primary cache hit after remapping would reuse stale split code"
        );
        let new_instr = cpu.i_cache.mpool[new_mpool];
        cpu.execute_instruction(&new_instr).unwrap();
        assert_eq!(
            cpu.get_gpr32(0),
            0x7788_9978,
            "continued decode/dispatch must execute bytes from the remapped second page"
        );

        cpu.clear_memory_access();
    }

    #[test]
    fn smp_fastrep_uses_pc_system_deadline_probes() {
        let topology = BxParams::default()
            .with_topology(1, 2, 1)
            .unwrap()
            .cpu_topology();
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.configure_smp(1, topology);

        let mut pc = BxPcSystemC::new();
        pc.initialize(1_000_000);
        pc.register_timer(TimerOwner::PciIdeCh0, 1000, false, true, "fastrep")
            .unwrap();
        cpu.set_pc_system_ptr_with_tick_denominator(NonNull::from(&mut pc), 2);

        assert_eq!(cpu.ticks_left_next_event(), 1000);

        cpu.tickn_fastrep(1000);

        assert_ne!(cpu.async_event & BX_ASYNC_EVENT_STOP_TRACE, 0);
    }

    #[test]
    fn scheduler_boundary_request_is_distinct_and_taken_explicitly() {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.async_event = BX_ASYNC_EVENT_STOP_TRACE;

        cpu.request_scheduler_boundary();

        assert_ne!(
            cpu.async_event & BX_ASYNC_EVENT_SCHEDULER_BOUNDARY,
            0,
            "boundary work must not be encoded as STOP_TRACE"
        );
        assert_ne!(cpu.async_event & BX_ASYNC_EVENT_STOP_TRACE, 0);
        assert!(cpu.take_scheduler_boundary_request());
        assert_eq!(cpu.async_event, BX_ASYNC_EVENT_STOP_TRACE);
        assert!(!cpu.take_scheduler_boundary_request());
    }

    #[test]
    fn fatal_execution_error_tears_down_memory_wiring() {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        cpu.wire_memory_access(
            NonNull::from(&mut mem),
            core::slice::from_ref(&pin),
            &pin,
        );
        let (host_base, host_len) = mem.identity_guest_base();
        cpu.mem_host_base = host_base;
        cpu.mem_host_len = host_len;

        let result: super::Result<()> = (|| {
            let _memory_wiring = CpuMemoryWiringGuard::new(&mut cpu);
            cpu.handle_execution_error(CpuError::CpuNotInitialized, &Instruction::default())?;
            Ok(())
        })();

        assert!(matches!(result, Err(CpuError::CpuNotInitialized)));
        assert!(cpu.mem_host_base.is_null());
        assert_eq!(cpu.mem_host_len, 0);
        assert!(cpu.mem_bus.is_none());
        assert!(cpu.active_tlb_pins().is_empty());
        assert!(cpu.active_tlb_pin_sidecar.is_none());
    }

    #[test]
    fn unwired_tlb_mutation_refreshes_pin_before_next_memory_scope() {
        const TARGET: u64 = 0x4000;

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        let host_ptr = mem.identity_guest_base().0 as usize + TARGET as usize;
        let slot = cpu.dtlb.get_index_of(TARGET, 0);
        let entry = &mut cpu.dtlb.entries[slot];
        entry.lpf = TARGET;
        entry.host_page_addr = host_ptr as _;
        entry.access_bits = 1;

        cpu.sync_dtlb_pin_slot(TARGET, 0);
        assert!(!pin.is_range_pinned(host_ptr, host_ptr + 0x1000));

        cpu.wire_memory_access(
            NonNull::from(&mut mem),
            core::slice::from_ref(&pin),
            &pin,
        );

        assert!(pin.is_range_pinned(host_ptr, host_ptr + 0x1000));
        cpu.clear_memory_access();
    }

    #[test]
    fn dtlb_miss_releases_colliding_pin_before_page_walk() {
        const TARGET: u64 = 0x4000;
        const OLD_LPF: u64 = 0x8000;

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        let pins = core::slice::from_ref(&pin);
        cpu.wire_memory_access(NonNull::from(&mut mem), pins, &pin);
        let (host_base, host_len) = mem.identity_guest_base();
        cpu.mem_host_base = host_base;
        cpu.mem_host_len = host_len;
        cpu.cpu_mode = CpuMode::Ia32Protected;
        cpu.cr0 = BxCr0::PE | BxCr0::PG;
        cpu.cr4 = BxCr4::empty();
        cpu.cr3 = 0;

        let host_ptr = host_base as usize;
        let slot = cpu.dtlb.get_index_of(TARGET, 0);
        let entry = &mut cpu.dtlb.entries[slot];
        entry.lpf = OLD_LPF;
        entry.host_page_addr = host_ptr as _;
        entry.access_bits = 1;
        cpu.sync_dtlb_pin_slot(TARGET, 0);
        assert!(pin.is_range_pinned(host_ptr, host_ptr + 0x1000));

        assert!(cpu.translate_data_read(TARGET).is_err());
        assert!(!pin.is_range_pinned(host_ptr, host_ptr + 0x1000));
        cpu.clear_execution_memory_wiring();
    }

    #[test]
    fn full_tlb_flush_clears_pin_hosts_like_a_full_rescan() {
        // Track B: `tlb_flush` publishes the pin sidecar via a memset
        // (`clear_active_tlb_pin_hosts`) instead of the per-slot rescan. Every
        // pinned DTLB and ITLB host pointer must be gone afterward — exactly
        // what `refresh_tlb_pin` produces once every entry is invalid.
        const DTLB_TARGET: u64 = 0x4000;
        const ITLB_TARGET: u64 = 0x8000;

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        let host_base = mem.identity_guest_base().0 as usize;
        let dtlb_host = host_base + DTLB_TARGET as usize;
        let itlb_host = host_base + ITLB_TARGET as usize;

        let dslot = cpu.dtlb.get_index_of(DTLB_TARGET, 0);
        {
            let e = &mut cpu.dtlb.entries[dslot];
            e.lpf = DTLB_TARGET;
            e.host_page_addr = dtlb_host as _;
            e.access_bits = 1;
        }
        let islot = cpu.itlb.get_index_of(ITLB_TARGET, 0);
        {
            let e = &mut cpu.itlb.entries[islot];
            e.lpf = ITLB_TARGET;
            e.host_page_addr = itlb_host as _;
            e.access_bits = 1;
        }

        cpu.wire_memory_access(NonNull::from(&mut mem), core::slice::from_ref(&pin), &pin);
        cpu.sync_dtlb_pin_slot(DTLB_TARGET, 0);
        cpu.sync_itlb_pin_slot(ITLB_TARGET, 0);
        assert!(pin.is_range_pinned(dtlb_host, dtlb_host + 0x1000));
        assert!(pin.is_range_pinned(itlb_host, itlb_host + 0x1000));

        cpu.tlb_flush();
        assert!(!pin.is_range_pinned(dtlb_host, dtlb_host + 0x1000));
        assert!(!pin.is_range_pinned(itlb_host, itlb_host + 0x1000));

        cpu.clear_memory_access();
    }

    #[test]
    fn full_tlb_flush_keeps_the_vmcb_pin_in_an_svm_guest() {
        // The full-flush memset zeros `vmcb_host`, but the flush does not change
        // SVM state — under-pinning the VMCB backing would be a use-after-free.
        // `clear_active_tlb_pin_hosts` re-publishes it while in an SVM guest.
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        let vmcb_host = mem.identity_guest_base().0 as usize + 0x1_0000;
        cpu.in_svm_guest = true;
        cpu.vmcbhostptr = vmcb_host as _;

        cpu.wire_memory_access(NonNull::from(&mut mem), core::slice::from_ref(&pin), &pin);
        cpu.sync_vmcb_pin();
        assert!(pin.is_range_pinned(vmcb_host, vmcb_host + 0x1000));

        cpu.tlb_flush();
        assert!(
            pin.is_range_pinned(vmcb_host, vmcb_host + 0x1000),
            "a full flush must not drop the active SVM guest's VMCB pin"
        );

        cpu.clear_memory_access();
    }

    #[test]
    fn incremental_tlb_pins_match_a_fresh_rescan_after_every_op() {
        // Track B property test. The pin sidecar is maintained incrementally:
        // installs publish one slot, `flush_non_global_and_publish_pin` and
        // `invlpg_and_publish_pin` fuse pin removal into the invalidation walk,
        // and `tlb_flush` memsets. After every operation the sidecar must be
        // byte-identical to a fresh `refresh_tlb_pin` full rescan — the oracle —
        // for both an SVM-off and an SVM-on host, or the fused walks would
        // under-pin (use-after-free) or over-pin (a stale eviction block).
        const MIB: usize = 1024 * 1024;

        for &svm in &[false, true] {
            let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
            let mut mem = BxMemC::new(
                BxMemoryStubC::create_and_init(4 * MIB, MIB, MIB).unwrap(),
                false,
            );
            let host_base = mem.identity_guest_base().0 as usize;

            if svm {
                cpu.in_svm_guest = true;
                cpu.vmcbhostptr = (host_base + 0x2_0000) as _;
            }

            let pin = CpuTlbPin::new(&cpu);
            let oracle = CpuTlbPin::new(&cpu);
            cpu.wire_memory_access(NonNull::from(&mut mem), core::slice::from_ref(&pin), &pin);
            if svm {
                cpu.sync_vmcb_pin();
            }

            // Deterministic xorshift64 — Date/rand are unavailable in tests.
            let mut state: u64 = 0x9E37_79B9_7F4A_7C15 ^ (svm as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            // Page-aligned linear address spanning 8192 pages, forcing slot
            // collisions in both the 1024-entry ITLB and the larger DTLB.
            let laddr_of = |n: u64| ((n & 0x1FFF) << 12) as u64;

            for _ in 0..2000 {
                match next() % 6 {
                    0 | 1 => {
                        let laddr = laddr_of(next());
                        let host = host_base + laddr as usize;
                        let global = (next() & 1) != 0;
                        let large = (next() & 7) == 0;
                        let slot = cpu.dtlb.get_index_of(laddr, 0);
                        {
                            let e = &mut cpu.dtlb.entries[slot];
                            e.lpf = laddr;
                            e.host_page_addr = host as _;
                            // bit31 == TLB_GLOBAL_PAGE (tlb.rs); low bit is a
                            // normal access-permission bit marking the entry live.
                            e.access_bits = 1 | if global { 0x8000_0000 } else { 0 };
                            e.lpf_mask = if large { 0x1F_FFFF } else { 0xFFF };
                        }
                        if large {
                            cpu.dtlb.split_large = true;
                        }
                        cpu.sync_dtlb_pin_slot(laddr, 0);
                    }
                    2 => {
                        let laddr = laddr_of(next());
                        let host = host_base + laddr as usize;
                        let global = (next() & 1) != 0;
                        let slot = cpu.itlb.get_index_of(laddr, 0);
                        {
                            let e = &mut cpu.itlb.entries[slot];
                            e.lpf = laddr;
                            e.host_page_addr = host as _;
                            e.access_bits = 1 | if global { 0x8000_0000 } else { 0 };
                            e.lpf_mask = 0xFFF;
                        }
                        cpu.sync_itlb_pin_slot(laddr, 0);
                    }
                    3 => {
                        let laddr = laddr_of(next());
                        cpu.invlpg_and_publish_pin(laddr);
                    }
                    4 => cpu.flush_non_global_and_publish_pin(),
                    _ => cpu.tlb_flush(),
                }

                cpu.refresh_tlb_pin(&oracle);
                assert!(
                    pin.state_matches(&oracle),
                    "incremental pin diverged from a fresh rescan (svm={svm})"
                );
            }

            cpu.clear_memory_access();
        }
    }

    #[test]
    fn tlb_flushes_disarm_the_monitor_like_bochs() {
        // Bochs paging.cc calls wakeup_monitor() in TLB_flush / TLB_flushNonGlobal
        // / TLB_invlpg: a flush can change the monitored page's translation, so
        // the monitor is disarmed and any MWAIT sleep is woken to ACTIVE. The
        // host-side rewire invalidate_host_memory_mappings is NOT a guest flush
        // and must preserve the (possibly just-restored) monitor.
        use super::{BX_MONITOR_ARMED_BY_MONITOR, CpuActivityState};

        // Full flush also wakes an MWAIT sleep to ACTIVE.
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.monitor.arm(0x1000, BX_MONITOR_ARMED_BY_MONITOR);
        cpu.activity_state = CpuActivityState::Mwait;
        assert!(cpu.monitor.armed());
        cpu.tlb_flush();
        assert!(!cpu.monitor.armed(), "tlb_flush must disarm the monitor");
        assert_eq!(
            cpu.activity_state,
            CpuActivityState::Active,
            "tlb_flush must wake an MWAIT sleep"
        );

        // Non-global flush disarms too.
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.monitor.arm(0x1000, BX_MONITOR_ARMED_BY_MONITOR);
        cpu.tlb_flush_non_global();
        assert!(!cpu.monitor.armed(), "tlb_flush_non_global must disarm the monitor");

        // Single-page invlpg disarms too.
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.monitor.arm(0x1000, BX_MONITOR_ARMED_BY_MONITOR);
        cpu.tlb_invlpg(0x2000);
        assert!(!cpu.monitor.armed(), "tlb_invlpg must disarm the monitor");

        // Host-side rewire must PRESERVE the monitor across its internal flush.
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        cpu.monitor.arm(0x1000, BX_MONITOR_ARMED_BY_MONITOR);
        cpu.invalidate_host_memory_mappings();
        assert!(
            cpu.monitor.armed(),
            "invalidate_host_memory_mappings must not disarm the guest monitor"
        );
    }

    #[test]
    fn itlb_miss_releases_colliding_pin_before_block_replacement() {
        const MIB: usize = 1024 * 1024;
        const TARGET: u64 = 4 * MIB as u64;

        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(5 * MIB, MIB, MIB).unwrap(),
            false,
        );
        mem.set_a20_mask(u64::MAX);
        let pin = CpuTlbPin::new(&cpu);
        let pins = core::slice::from_ref(&pin);
        cpu.wire_memory_access(NonNull::from(&mut mem), pins, &pin);

        mem.write_ram(&[], 0, &[0x5a]).unwrap();
        let host_ptr = mem
            .get_host_mem_addr_pinned(
                0,
                MemoryAccessType::Read,
                &[],
                crate::memory::CpuMemoryPolicy::default(),
            )
            .unwrap()
            .unwrap()
            .as_ptr() as usize;
        let slot = cpu.itlb.get_index_of(TARGET, 0);
        let entry = &mut cpu.itlb.entries[slot];
        entry.lpf = TARGET;
        entry.host_page_addr = host_ptr as _;
        entry.access_bits = 1;
        cpu.eip_fetch_ptr = Some(&[0]);
        cpu.sync_itlb_pin_slot(TARGET, 0);
        assert!(pin.is_range_pinned(host_ptr, host_ptr + MIB));

        cpu.invalidate_itlb_pin_slot(TARGET, 0);

        assert!(cpu.eip_fetch_ptr.is_none());
        assert!(!pin.is_range_pinned(host_ptr, host_ptr + MIB));
        mem.write_ram(pins, TARGET, &[0xa5]).unwrap();
        let mut replaced = [0];
        assert_eq!(mem.read_ram(pins, TARGET, &mut replaced).unwrap(), 1);
        assert_eq!(replaced, [0xa5]);
        cpu.clear_memory_access();
    }

    #[test]
    fn svm_pin_sidecar_tracks_guest_state_transitions() {
        let mut cpu = BxCpuBuilder::<Corei7SkylakeX>::new().build().unwrap();
        let mut mem = BxMemC::new(
            BxMemoryStubC::create_and_init(1 << 20, 1 << 20, 4096).unwrap(),
            false,
        );
        let pin = CpuTlbPin::new(&cpu);
        cpu.wire_memory_access(
            NonNull::from(&mut mem),
            core::slice::from_ref(&pin),
            &pin,
        );

        cpu.in_svm_guest = true;
        cpu.vmcbhostptr = 0x6000;
        cpu.sync_vmcb_pin();
        assert!(pin.is_range_pinned(0x6000, 0x7000));

        cpu.in_svm_guest = false;
        cpu.sync_vmcb_pin();
        assert!(!pin.is_range_pinned(0x6000, 0x7000));
        cpu.clear_memory_access();
    }
}
