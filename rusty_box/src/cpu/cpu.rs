#![allow(non_snake_case, unused_variables, unused_assignments, dead_code)]
#![allow(unused_unsafe)]

use alloc::vec;
use core::{marker::PhantomData, ptr::NonNull};

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
    memory::BxMemC,
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

// Bochs uses 2048 DTLB / 1024 ITLB (direct-mapped). Real CPUs have much
// larger set-associative TLBs (Intel Skylake: 1536 4K + 32 2M/4M data entries).
// 4096 entries reduce direct-mapped eviction pressure during Linux kernel
// startup where boot page tables overlap with decompressed kernel data.
const BX_DTLB_SIZE: usize = 4096;
const BX_ITLB_SIZE: usize = 1024;

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
    pub fn rrx(&self) -> u64 { self.value }
    #[inline(always)]
    pub fn set_rrx(&mut self, v: u64) { self.value = v; }

    /// Lower 32-bit dword (EAX, ECX, ...). Does NOT zero-extend on read.
    #[inline(always)]
    pub fn erx(&self) -> u32 { self.value as u32 }
    /// Write lower 32 bits, PRESERVING upper 32 bits.
    /// Callers that need x86-64 zero-extension must also call set_hrx(0).
    #[inline(always)]
    pub fn set_erx(&mut self, v: u32) {
        self.value = (self.value & 0xFFFF_FFFF_0000_0000) | v as u64;
    }

    /// Upper 32 bits (used for zero-extension checks).
    #[inline(always)]
    pub fn hrx(&self) -> u32 { (self.value >> 32) as u32 }
    #[inline(always)]
    pub fn set_hrx(&mut self, v: u32) {
        self.value = (self.value & 0x0000_0000_FFFF_FFFF) | ((v as u64) << 32);
    }

    /// Lower 16-bit word (AX, CX, ...).
    #[inline(always)]
    pub fn rx(&self) -> u16 { self.value as u16 }
    /// Write lower 16 bits, preserving all other bits.
    #[inline(always)]
    pub fn set_rx(&mut self, v: u16) {
        self.value = (self.value & !0xFFFF) | v as u64;
    }

    /// Low byte (AL, CL, ...).
    #[inline(always)]
    pub fn rl(&self) -> u8 { self.value as u8 }
    #[inline(always)]
    pub fn set_rl(&mut self, v: u8) {
        self.value = (self.value & !0xFF) | v as u64;
    }

    /// High byte of low word (AH, CH, ...).
    #[inline(always)]
    pub fn rh(&self) -> u8 { (self.value >> 8) as u8 }
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
pub struct BxCpuC<'c, I: BxCpuIdTrait> {
    pub(super) bx_cpuid: u32,

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

    pub(crate) icount: u64,
    pub(super) icount_last_sync: u64,

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
    pub(super) cr4_suppmask: u32,

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

    #[cfg(feature = "bx_configure_msrs")]
    pub(super) msrs: [MSR; BX_MSR_MAX_INDEX],

    pub(super) amx: Option<AMX>,

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

    pub(super) nmi_unblocking_iret: bool,

    /// 1 if processing external interrupt or exception
    /// or if not related to current instruction,
    /// 0 if current CS:EIP caused exception */
    pub(super) ext: bool,

    pub(crate) activity_state: CpuActivityState,

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
    pub(super) last_exception_type: u32,

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

    #[cfg(feature = "bx_instrumentation")]
    pub(super) far_branch: FarBranch,

    /// Bochs-style instrumentation (instrument/instrumentation.txt).
    /// Install via `set_instrumentation()`. Feature-gated by `bx_instrumentation`.
    #[cfg(feature = "bx_instrumentation")]
    pub(crate) instrumentation: Option<alloc::boxed::Box<dyn super::Instrumentation>>,

    pub(crate) dtlb: Tlb<BX_DTLB_SIZE>,
    pub(super) itlb: Tlb<BX_ITLB_SIZE>,

    pub(super) pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    pub(super) i_cache: BxICache,
    pub(super) fetch_mode_mask: super::opcodes_table::FetchModeMask,

    pub(super) address_xlation: AddressXlation,

    /* Now other not so obvious fields */
    pub(super) smram_map: [u32; SMMRAM_Fields::SMRAM_FIELD_LAST as _],

    pub(super) phantom: PhantomData<I>,

    /// Temporary memory pointer for instruction execution (set during cpu_loop)
    /// This is a raw pointer to avoid lifetime issues - only valid during cpu_loop
    /// SAFETY: Must only be used during cpu_loop when memory is valid
    pub(super) mem_ptr: Option<*mut u8>,
    pub(super) mem_len: usize,

    /// Host memory base pointer, pointing to physical address 0 (accounts for vector_offset).
    /// Used for direct memory access on TLB hits, bypassing get_host_mem_addr().
    /// SAFETY: Only valid during cpu_loop when memory is valid.
    pub(crate) mem_host_base: *mut u8,
    /// Usable guest RAM length (not including ROM/bogus).  Physical addresses below this
    /// (and outside VGA/MMIO ranges) can be accessed directly via mem_host_base.
    pub(crate) mem_host_len: usize,

    /// Optional memory system pointer (MMIO/ROM handler access), wired during execution.
    ///
    /// This mirrors Bochs' v2h/getHostMemAddr model: the CPU can attempt direct host access
    /// when allowed, and fall back to handler-aware reads/writes when access is vetoed.
    ///
    /// It must only be set for the duration of a CPU execution call and cleared afterwards.
    pub(super) mem_bus: Option<NonNull<crate::memory::BxMemC<'c>>>,

    /// Optional I/O bus (device port handlers), wired by the emulator during execution.
    ///
    /// This is a raw pointer to avoid borrow checker overhead in the hot path.
    /// It must only be set for the duration of a CPU execution call and cleared afterwards.
    pub(super) io_bus: Option<NonNull<crate::iodev::BxDevicesC>>,

    /// Optional PC system pointer for timer queries (getNumCpuTicksLeftNextEvent).
    /// Wired by the emulator during execution, cleared afterwards.
    pub(super) pc_system_ptr: Option<NonNull<crate::pc_system::BxPcSystemC>>,


    /// Debug flags for one-time boot diagnostics (no globals).
    ///
    /// Bit 0: reported unsupported opcode
    /// Bit 1: reported real-mode IVT vector to 0000:0000
    pub(super) boot_debug_flags: u8,
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) const BX_ASYNC_EVENT_STOP_TRACE: u32 = 1 << 31;
    /// Persistent sleep sentinel set by enter_sleep_state (HLT/MWAIT).
    /// Matches Bochs proc_ctrl.cc `async_event = 1` — survives the
    /// `&= ~STOP_TRACE` clearing so handle_async_event is called next
    /// outer-loop iteration to check for wake conditions.
    pub(super) const BX_ASYNC_EVENT_SLEEP: u32 = 1;

    /// Event bit: external interrupt pending (PIC int_pin asserted).
    /// Bochs uses bit 10; we use bit 0 for internal consistency.
    pub(crate) const BX_EVENT_PENDING_INTR: u32 = 1 << 0;

    /// Event bit: NMI pending/masked. Bochs cpu.h uses bit 0,
    /// but we use bit 1 to avoid conflict with PENDING_INTR.
    /// Masked on NMI delivery, unmasked on IRET.
    pub(super) const BX_EVENT_NMI: u32 = 1 << 1;

    /// Event bit: LAPIC interrupt pending.
    /// Bochs cpu.h uses bit 11; we use bit 2.
    pub(crate) const BX_EVENT_PENDING_LAPIC_INTR: u32 = 1 << 2;

    /// Event bit: System Management Interrupt pending.
    /// Bochs cpu.h uses bit 1; we use bit 6 (bits 0-2 already taken).
    /// SMI enters System Management Mode — not implemented for single-CPU Alpine/DLX.
    #[allow(dead_code)]
    pub(super) const BX_EVENT_SMI: u32 = 1 << 6;

    /// Event bit: INIT signal pending (CPU reset).
    /// Bochs cpu.h uses bit 2; we use bit 7 (bits 0-2 already taken).
    /// INIT is used by multiprocessor startup (INIT-SIPI-SIPI) — not implemented.
    #[allow(dead_code)]
    pub(super) const BX_EVENT_INIT: u32 = 1 << 7;

    /// Returns a mutable raw pointer to the Local APIC for cross-module wiring.
    /// Used by emulator.rs to wire I/O APIC → LAPIC interrupt delivery.
    pub(crate) fn lapic_ptr_mut(&mut self) -> *mut crate::cpu::apic::BxLocalApic {
        &mut self.lapic as *mut _
    }

    /// Check LAPIC IRR/ISR for a specific vector (immutable access for diagnostics).
    #[cfg(feature = "bx_support_apic")]
    pub(crate) fn lapic_vector_state(&self, vector: u8) -> (bool, bool) {
        self.lapic.vector_state(vector)
    }

    /// Check if the LAPIC has a pending interrupt (immutable access).
    #[cfg(feature = "bx_support_apic")]
    pub(crate) fn lapic_has_intr(&self) -> bool {
        self.lapic.intr
    }
}

// Note: Memory access is done through mem_ptr/mem_len raw pointer
// which is set during cpu_loop. See string.rs for mem_read_byte/mem_write_byte helpers.

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
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

    pub(super) fn bx_write_opmask(&mut self, index: usize, val_64: u64) {
        self.opmask[index].set_rrx(val_64);
    }

    // ── Debug trap bits (DR6 bits set by CPU) ──
    // Bochs cpu.h
    pub(super) const BX_DEBUG_SINGLE_STEP_BIT: u32 = 1 << 14; // BS flag in DR6 (bit 14)
    pub(super) const BX_DEBUG_TRAP_TASK_SWITCH_BIT: u32 = 0x8000; // BT flag in DR6

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
pub(super) struct FarBranch {
    pub(crate) rev_cs: u16,
    pub(crate) rev_rip: BxAddress,
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

    pub(crate) ia32_umwait_ctrl: u32,
    pub(crate) ia32_spec_ctrl: u32, // SCA
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
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

#[cfg(feature = "bx_support_monitor_mwait")]
#[derive(Debug, Default)]
pub struct MonitorAddr {
    pub(super) monitor_addr: BxPhyAddress,
    armed_by: u32,
}

#[cfg(feature = "bx_support_monitor_mwait")]
pub(super) const BX_MONITOR_NOT_ARMED: u32 = 0;
#[cfg(feature = "bx_support_monitor_mwait")]
pub(super) const BX_MONITOR_ARMED_BY_MONITOR: u32 = 1;
#[cfg(feature = "bx_support_monitor_mwait")]
pub(super) const BX_MONITOR_ARMED_BY_MONITORX: u32 = 2;
#[cfg(feature = "bx_support_monitor_mwait")]
pub(super) const BX_MONITOR_ARMED_BY_UMONITOR: u32 = 3;

#[cfg(feature = "bx_support_monitor_mwait")]
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
    ui_handler: BxAddress,
    stack_adjust: u64,
    /// user interrupt notification vector, actually 8 bit
    uinv: u32,
    /// user interrupt target table size
    uitt_size: u32,
    /// user interrupt target table address
    uitt_addr: BxAddress,
    /// user posted-interrupt descriptor address
    upid_addr: BxAddress,
    /// user-interrupt request register
    uirr: u64,
    /// if UIF=0 user interrupt cannot be delivered
    uif: bool,
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
type InstructionHandler<I> = fn(&mut BxCpuC<'_, I>, &Instruction) -> Result<()>;

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
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

    /// Install instrumentation hooks (Bochs instrument/instrumentation.txt).
    /// Implement the `Instrumentation` trait and pass a boxed instance.
    #[cfg(feature = "bx_instrumentation")]
    pub fn set_instrumentation(&mut self, instr: alloc::boxed::Box<dyn super::Instrumentation>) {
        self.instrumentation = Some(instr);
    }

    #[inline]
    pub(crate) fn set_pc_system_ptr(&mut self, ps: NonNull<crate::pc_system::BxPcSystemC>) {
        self.pc_system_ptr = Some(ps);
    }

    #[inline]
    pub(crate) fn clear_pc_system(&mut self) {
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

    #[inline(always)]
    pub(super) fn mem_bus_mut(&mut self) -> Option<&mut crate::memory::BxMemC<'c>> {
        self.mem_bus.map(|mut p| unsafe { p.as_mut() })
    }

    /// Returns `(&mut BxMemC, &BxCpuC)` from `&mut self` — the split-borrow
    /// pattern needed by physical memory operations that pass `&[cpu_ref]`.
    ///
    /// SAFETY (internal): Two raw-pointer derefs under the single-threaded
    /// emulator invariant — `NonNull<BxMemC>` is valid for the cpu_loop
    /// lifetime, and the `*const BxCpuC` re-borrow is non-aliasing because
    /// nothing writes through it while `mem` is live.
    #[inline(always)]
    pub(super) fn mem_bus_and_cpu(&self) -> Option<(&mut crate::memory::BxMemC<'c>, &BxCpuC<'c, I>)> {
        let mem_bus = self.mem_bus?;
        // SAFETY: mem_bus valid for duration of cpu_loop; single-threaded access
        let mem = unsafe { &mut *mem_bus.as_ptr() };
        // SAFETY: cpu_ptr derived from valid &mut self; no aliasing during this call
        let cpu_ref: &BxCpuC<'c, I> = unsafe { &*(self as *const BxCpuC<'c, I>) };
        Some((mem, cpu_ref))
    }


    /// Propagate PIC interrupt flags to CPU event state.
    ///
    /// Called after every I/O port access so the CPU sees PIC-raised
    /// interrupts within the current instruction batch rather than
    /// waiting for the next `sync_event_flags()` between batches.
    ///
    /// Reads `pic_irq_pending` / `pic_irq_cleared` flags set by I/O
    /// dispatch (which consumes the PIC's own flags after each handler
    /// call) rather than dereferencing the PIC pointer directly.
    #[inline]
    pub(super) fn sync_pic_flags(&mut self) {
        if let Some(io) = self.io_bus_mut() {
            let pending = io.pic_irq_pending;
            let cleared = io.pic_irq_cleared;
            if pending { io.pic_irq_pending = false; }
            if cleared { io.pic_irq_cleared = false; }
            // io borrow ends here (NLL); safe to mutate self fields
            if pending {
                self.pending_event |= Self::BX_EVENT_PENDING_INTR;
                self.async_event = 1;
            }
            if cleared {
                self.pending_event &= !Self::BX_EVENT_PENDING_INTR;
            }
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

    /// Advance pc_system countdown during FastRep bulk operations.
    /// Matches Bochs `BX_TICKN(byteCount)` inside `faststring.cc`.
    /// When countdown expires, sets STOP_TRACE to force trace break
    /// so the outer emulator loop can fire `countdown_event()`.
    /// Must use STOP_TRACE (bit 31), NOT bit 0 — the CPU loop fast path
    /// only clears `async_event` when it equals exactly STOP_TRACE.
    /// Bit 0 would persist and poison all subsequent instructions.
    #[inline]
    pub(super) fn tickn_fastrep(&mut self, n: usize) {
        if let Some(ps) = self.pc_system_mut() {
            let expired = ps.sub_countdown(n as u32);
            if expired {
                self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
            }
        }
    }

    #[inline]
    pub(crate) fn set_mem_bus_ptr(&mut self, mem: NonNull<crate::memory::BxMemC<'c>>) {
        self.mem_bus = Some(mem);
    }

    #[inline]
    pub(crate) fn clear_mem_bus(&mut self) {
        self.mem_bus = None;
    }

    #[inline]
    pub(crate) fn debug_putc(&mut self, ch: u8) {
        if let Some(io) = self.io_bus_mut() {
            io.outp(0x00E9, ch as u32, 1);
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
        #[cfg(debug_assertions)] {
            self.diag_inject_ext_intr_count += 1;
            self.diag_inject_ext_intr_vectors[vector as usize] += 1;
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
        let result = self.interrupt(vector, false, false, 0);

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
    pub fn cpu_loop_n_with_io(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
        max_instructions: u64,
        io: NonNull<crate::iodev::BxDevicesC>,
        pc_system: NonNull<crate::pc_system::BxPcSystemC>,
        pic: Option<&mut crate::iodev::pic::BxPicC>,
        dma: Option<&mut crate::iodev::dma::BxDmaC>,
    ) -> super::Result<u64> {
        self.set_io_bus_ptr(io);
        self.set_pc_system_ptr(pc_system);
        let result = self.cpu_loop_n(mem, cpus, max_instructions, pic, dma);
        self.clear_io_bus();
        self.clear_pc_system();
        result
    }

    pub fn cpu_loop(&mut self, mem: &'c mut BxMemC<'c>, cpus: &[&Self]) -> super::Result<()> {
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

        self.cpu_loop_n(mem, cpus, 1_000_000, None, None)?;
        Ok(())
    }

    /// Execute CPU loop with a maximum instruction count
    ///
    /// Returns Ok(instructions_executed) when limit is reached or async event occurs.
    pub fn cpu_loop_n(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
        max_instructions: u64,
        mut pic: Option<&mut crate::iodev::pic::BxPicC>,
        mut dma: Option<&mut crate::iodev::dma::BxDmaC>,
    ) -> super::Result<u64> {
        // Wire the memory system pointer for the duration of this execution call.
        // This enables Bochs-style "host-pointer-or-fallback" access in mem_read/mem_write.
        // Reborrow `mem` so we don't move the `&mut` binding.
        self.a20_mask = mem.a20_mask();
        self.set_mem_bus_ptr(NonNull::from(&mut *mem));

        // Set memory pointer for instruction execution
        // Store raw pointer to the memory vector for direct access
        let (mem_vector, mem_len) = mem.get_raw_memory_ptr();
        self.mem_ptr = Some(mem_vector);
        self.mem_len = mem_len;

        // Host base pointer: points to physical address 0 (vector_offset-adjusted).
        // Used for direct TLB-hit memory access bypassing get_host_mem_addr().
        let (host_base, host_len) = mem.get_ram_base_ptr();
        self.mem_host_base = host_base;
        self.mem_host_len = host_len;

        let mut iteration = 0u64;
        // Track icount at start for Bochs-compatible IPS measurement.
        // Bochs counts REP iterations as separate instructions via time_ticks(),
        // which matches icount (incremented per REP iteration in string.rs).
        // We return icount delta instead of iteration count to match.
        let icount_start = self.icount;
        #[cfg(feature = "profiling")]
        let mut prof_assign_ns = 0u64;
        #[cfg(feature = "profiling")]
        let mut prof_exec_ns = 0u64;
        #[cfg(feature = "profiling")]
        let mut prof_icache_ns = 0u64;

        // SAFETY: segment cache populated during segment load; union read matches descriptor type
        tracing::info!(
            "CPU loop starting at CS:IP = {:04X}:{:08X}",
            self.cs_selector_value(),
            self.rip()
        );

        let _last_diag_iteration = 0u64;
        let mut outer_loop_count = 0u64;
        let result = 'cpu_loop: loop {
            outer_loop_count += 1;
            // Detect spinning: log every 100K outer-loop iterations
            if outer_loop_count.is_multiple_of(100_000) {
                tracing::debug!(
                    "[cpu_loop-spin] outer={} iter={}/{} RIP={:#010x} async={} activity={:?}",
                    outer_loop_count,
                    iteration,
                    max_instructions,
                    self.rip(),
                    self.async_event,
                    self.activity_state,
                );
                if outer_loop_count > 50_000_000 {
                    tracing::error!("[cpu_loop] BAILOUT after {} outer loops", outer_loop_count);
                    break Ok(iteration);
                }
            }

            // Safety limit - pause when instruction limit is reached
            // Use >= so each batch runs exactly max_instructions, not max_instructions+1.
            if iteration >= max_instructions {
                #[cfg(feature = "profiling")]
                tracing::debug!(
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
                self.perf_icache_miss = 0;
                self.perf_prefetch = 0;
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
                // Fast path: if only STOP_TRACE is set and CPU is still active,
                // just clear it without calling handle_async_event(). This is the
                // common case after a taken branch — no real events to process.
                if self.async_event == BX_ASYNC_EVENT_STOP_TRACE
                    && matches!(self.activity_state, CpuActivityState::Active)
                {
                    self.async_event = 0;
                } else if self.handle_async_event(pic.as_deref_mut(), dma.as_deref_mut()) {
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
                        continue 'cpu_loop;
                    }
                    Err(e) => break 'cpu_loop Err(e),
                }
            };
            #[cfg(feature = "profiling")]
            {
                prof_icache_ns += _t0.elapsed().as_nanos() as u64;
            }
            let is_real = self.real_mode();

            let mut trace_iter = 0u64;
            'trace: loop {
                trace_iter += 1;
                // Bochs-style: pointer to mpool slot — no 24-byte copy per instruction.
                // SAFETY: execute_instruction never writes to i_cache.mpool (only CPU registers
                // and memory). serve_icache_miss is only called from get_icache_entry, not during
                // instruction execution. So the mpool slot is stable for the duration of this call.
                let i_ptr: *const Instruction = &raw const self.i_cache.mpool[instr_idx];
                // SAFETY: i_ptr points into self.i_cache.mpool which is stable for this
                // iteration — execute_instruction never writes to mpool, and serve_icache_miss
                // is only called from get_icache_entry, not during execution.
                let instr_ref = || -> &Instruction { unsafe { &*i_ptr } };

                // Bochs cpu.cc sets prev_rip AFTER execution (not before ilen).
                // prev_rip is set below, after execute_instruction returns Ok(()).

                // Advance RIP before execution (handlers may read RIP and expect it advanced)
                // SAFETY: gen_reg is initialized during CPU init; BX_64BIT_REG_RIP is always valid.
                let ilen_val = instr_ref().ilen();
                // ilen=0 is valid ONLY for InsertedOpcode (trace boundary marker)
                if ilen_val == 0 || ilen_val > 15 {
                    let oc = instr_ref().get_ia_opcode();
                    assert!(ilen_val == 0 && oc == super::decoder::Opcode::InsertedOpcode,
                        "Invalid ilen={} opcode={:?} at RIP={:#x}", ilen_val, oc, self.gen_reg[BX_64BIT_REG_RIP].rrx());
                }
                self.gen_reg[BX_64BIT_REG_RIP].set_rrx(self.gen_reg[BX_64BIT_REG_RIP].rrx() + ilen_val as u64);
                if is_real {
                    self.gen_reg[BX_64BIT_REG_RIP].set_rrx(self.gen_reg[BX_64BIT_REG_RIP].rrx() & 0xFFFF);
                }

                // Execute instruction (matching C++ BX_CPU_CALL_METHOD)
                let opcode = instr_ref().get_ia_opcode();
                #[cfg(debug_assertions)] { self.diag_current_opcode = opcode as u16; }

                // Bochs BX_INSTR_BEFORE_EXECUTION(cpu_id, i)
                #[cfg(feature = "bx_instrumentation")]
                if self.instrumentation.is_some() {
                    if self.icount == 50_000_001 {
                        tracing::trace!("[INSTR-ALIVE] instrumentation callback active at icount={}", self.icount);
                    }
                    let snap = super::CpuSnapshot {
                        rax: self.rax(), rbx: self.rbx(), rcx: self.rcx(), rdx: self.rdx(),
                        rsi: self.rsi(), rdi: self.rdi(), rbp: self.rbp(), rsp: self.rsp(),
                        r8: self.r8(), r9: self.r9(), r10: self.r10(), r11: self.r11(),
                        r12: self.r12(), r13: self.r13(), r14: self.r14(), r15: self.r15(),
                        eflags: self.eflags.bits(), icount: self.icount,
                    };
                    let rip_before = self.prev_rip;
                    // Immutable borrows of `self` (via getters) are now done.
                    // Safe to take mutable borrow of the instrumentation field.
                    if let Some(cb) = self.instrumentation.as_mut() {
                        cb.before_execution(rip_before, opcode as u16, ilen_val, &snap);
                    }
                }

                match self.execute_instruction(instr_ref()) {
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
                            tracing::debug!("CPU shutdown — exiting cpu_loop");
                            break 'cpu_loop Ok(iteration);
                        }
                        self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                        continue 'cpu_loop;
                    }
                    Err(e) => {
                        // Cold path: handle fatal/unimplemented errors
                        self.handle_execution_error(e, instr_ref())?;
                        break 'cpu_loop Err(crate::cpu::CpuError::CpuNotInitialized);
                    }
                }

                // Bochs cpu.cc — prev_rip = RIP AFTER execution ("commit new RIP")
                self.prev_rip = self.gen_reg[BX_64BIT_REG_RIP].rrx();
                // Bochs cpu.cc — icount++
                self.icount += 1;
                self.perf_instructions += 1;

                iteration += 1;







                // Check async events (matching C++ line 215: if (async_event) break;)
                // When async_event is set (branch taken, exception, HLT, etc.), we MUST
                // break out of the trace because RIP has changed and the next sequential
                // instruction in the trace is wrong. The outer loop will handle the event
                // and fetch a new trace for the updated RIP.
                if self.async_event != 0 {
                    break 'trace;
                }

                // Matching C++ line 217: if (++i == last) { get new trace }
                instr_idx += 1;
                if instr_idx >= trace_end {
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
                }
            }

            // Clear stop trace magic indication (matching C++ line 226)
            // Bochs unconditionally clears STOP_TRACE after inner loop break.
            {
                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
            }
        };

        // Clear memory pointer when done
        self.mem_ptr = None;
        self.mem_host_base = core::ptr::null_mut();
        self.mem_host_len = 0;
        self.clear_mem_bus();
        result
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
                let instr_bytes = if let Some(fetch_ptr) = &self.eip_fetch_ptr {
                    let page_base = cs_base + self.eip_page_bias;
                    let offset = (rip.wrapping_sub(page_base)) as usize;
                    let ilen = instr.ilen() as usize;
                    if offset < fetch_ptr.len() && offset + ilen <= fetch_ptr.len() {
                        fetch_ptr[offset..offset + ilen].to_vec()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };
                tracing::error!(
                    "UNIMPLEMENTED OPCODE: {} at RIP={:#x} CS:IP={:#x}:{:#x} laddr={:#x} bytes={:02x?}",
                    opcode, rip, cs_value, rip, laddr, instr_bytes
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
        cpus: &[&Self],
    ) -> Result<Instruction> {
        let mem_ptr: *mut BxMemC<'c> = mem;
        // SAFETY: mem_ptr valid for duration of cpu_loop; reborrow is non-overlapping
        let (mpool_start_idx, _tlen) = unsafe {
            let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
            self.get_icache_entry(mem_reborrowed, cpus)?
        };
        Ok(self.i_cache.mpool[mpool_start_idx])
    }

    /// Look up the instruction cache for the current RIP.
    /// Returns (mpool_start_idx, tlen) to avoid cloning BxICacheEntry on the hot path.
    /// Matching Bochs cpu.cc getICacheEntry().
    #[inline]
    fn get_icache_entry(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<(usize, usize)> {
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
            self.perf_prefetch += 1;
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
                    tracing::debug!(
                        "prefetch queue invalidated after exception, retrying (attempt {})",
                        retry_count
                    );
                    continue;
                }

                eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;

                if eip_biased >= self.eip_page_window_size {
                    tracing::debug!("eip_biased ({}) >= eip_page_window_size ({}) after prefetch, RIP={:#x}, retrying",
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
        let p_addr: BxPhyAddress = self.p_addr_fetch_page | (eip_biased as u64);

        // Direct icache lookup without cloning BxICacheEntry.
        // We only need mpool_start_idx and tlen from the entry.
        let hash_idx = BxICache::hash(p_addr, self.fetch_mode_mask.bits().into()) as usize;
        let entry = &self.i_cache.entry[hash_idx];

        // Check if entry matches and has valid instruction (matching C++ line 299)
        let cache_hit = matches!(entry.p_addr, crate::cpu::icache::IcacheAddress::Address(addr) if addr == p_addr)
            && entry.i.ilen() != 0;


        if cache_hit {
            // SMC detection: compare first 8 bytes against current memory
            let mut smc_invalid = false;
            if let Some(fetch_slice) = self.eip_fetch_ptr {
                let offset = eip_biased as usize;
                let avail = fetch_slice.len().saturating_sub(offset).min(8);
                if avail > 0 && fetch_slice[offset..offset + avail] != entry.first_bytes[..avail] {
                    smc_invalid = true;
                }
                // Page-boundary instructions: if fewer bytes are available in
                // the current page than the instruction length, the remaining
                // bytes live on the NEXT page.  The first_bytes check above
                // only verifies the bytes on THIS page.  If the next page was
                // remapped (e.g. uselib/mmap loaded a new library), the
                // second-page bytes changed but the SMC check didn't catch it.
                // Force a cache miss so boundary_fetch re-reads both pages.
                let ilen = entry.i.ilen() as usize;
                if ilen > 0 && avail < ilen {
                    smc_invalid = true;
                }
            }

            if !smc_invalid {
                // Cache hit — return indices without cloning
                return Ok((entry.mpool_start_idx, entry.tlen));
            }
        }

        // Cache miss path
        self.perf_icache_miss += 1;

        let mut dummy_mapping: [u32; 0] = [];
        let mut dummy_stamp_table = crate::cpu::icache::BxPageWriteStampTable {
            fine_granularity_mapping: &mut dummy_mapping,
        };

        // SAFETY: prefetch() borrow is released before serve_icache_miss is called
        let miss_entry = unsafe {
            let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
            self.serve_icache_miss(
                eip_biased,
                p_addr,
                mem_reborrowed,
                cpus,
                &mut dummy_stamp_table,
            )?
        };
        Ok((miss_entry.mpool_start_idx, miss_entry.tlen))
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
        // Bochs ADD_COUT_VEC: carry-out at each bit position
        // Works correctly for both ADD and ADC (result includes carry-in)
        let cout_vec = (op1 & op2) | ((op1 | op2) & !res);
        let cf = (cout_vec >> 31) & 1 != 0;
        let zf = res == 0;
        let sf = (res & 0x8000_0000) != 0;
        // Bochs GET_ADD_OVERFLOW
        let of = ((op1 ^ res) & (op2 ^ res) & 0x8000_0000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let low = (res & 0xff) as u8;
        let parity = low.count_ones().is_multiple_of(2);

        // clear relevant flags
        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if parity {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    pub(super) fn update_flags_sub32(&mut self, op1: u32, op2: u32, res: u32) {
        // Bochs SUB_COUT_VEC: borrow at each bit position
        // Works correctly for both SUB and SBB (result includes borrow-in)
        let cout_vec = (!op1 & op2) | ((!op1 ^ op2) & res);
        let cf = (cout_vec >> 31) & 1 != 0;
        let zf = res == 0;
        let sf = (res & 0x8000_0000) != 0;
        // Bochs GET_SUB_OVERFLOW
        let of = ((op1 ^ op2) & (op1 ^ res) & 0x8000_0000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let low = (res & 0xff) as u8;
        let parity = low.count_ones().is_multiple_of(2);

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if parity {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    // execute_instruction() is in dispatcher.rs
    // Moved 2026-02-27: ~2000-line opcode dispatch match extracted to keep cpu.rs focused on CPU loop

    // 8-bit flag updates
    pub(super) fn update_flags_add8(&mut self, op1: u8, op2: u8, result: u8) {
        // Bochs ADD_COUT_VEC: carry-out at each bit position
        let cout_vec = (op1 & op2) | ((op1 | op2) & !result);
        let cf = (cout_vec >> 7) & 1 != 0;
        let zf = result == 0;
        let sf = (result & 0x80) != 0;
        // Bochs GET_ADD_OVERFLOW
        let of = ((op1 ^ result) & (op2 ^ result) & 0x80) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let pf = result.count_ones().is_multiple_of(2);

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    pub(super) fn update_flags_add16(&mut self, op1: u16, op2: u16, result: u16) {
        // Bochs ADD_COUT_VEC: carry-out at each bit position
        let cout_vec = (op1 & op2) | ((op1 | op2) & !result);
        let cf = (cout_vec >> 15) & 1 != 0;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        // Bochs GET_ADD_OVERFLOW
        let of = ((op1 ^ result) & (op2 ^ result) & 0x8000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let pf = ((result & 0xFF) as u8).count_ones().is_multiple_of(2);

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    pub(super) fn update_flags_sub8(&mut self, op1: u8, op2: u8, result: u8) {
        // Bochs SUB_COUT_VEC: borrow at each bit position
        let cout_vec = (!op1 & op2) | ((!op1 ^ op2) & result);
        let cf = (cout_vec >> 7) & 1 != 0;
        let zf = result == 0;
        let sf = (result & 0x80) != 0;
        // Bochs GET_SUB_OVERFLOW
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x80) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let pf = result.count_ones().is_multiple_of(2);

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    pub(super) fn update_flags_sub16(&mut self, op1: u16, op2: u16, result: u16) {
        // Bochs SUB_COUT_VEC: borrow at each bit position
        let cout_vec = (!op1 & op2) | ((!op1 ^ op2) & result);
        let cf = (cout_vec >> 15) & 1 != 0;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        // Bochs GET_SUB_OVERFLOW
        let of = ((op1 ^ op2) & (op1 ^ result) & 0x8000) != 0;
        let af = (cout_vec >> 3) & 1 != 0;
        let pf = ((result & 0xFF) as u8).count_ones().is_multiple_of(2);

        self.eflags.remove(EFlags::OSZAPC);

        if cf {
            self.eflags.insert(EFlags::CF);
        }
        if pf {
            self.eflags.insert(EFlags::PF);
        }
        if af {
            self.eflags.insert(EFlags::AF);
        }
        if zf {
            self.eflags.insert(EFlags::ZF);
        }
        if sf {
            self.eflags.insert(EFlags::SF);
        }
        if of {
            self.eflags.insert(EFlags::OF);
        }
    }

    pub(super) fn update_flags_logic8(&mut self, result: u8) {
        // Bochs SET_FLAGS_OSZAPC_LOGIC clears OF, CF, AF
        self.eflags.remove(EFlags::OF | EFlags::CF | EFlags::AF);
        self.eflags.set(EFlags::SF, (result & 0x80) != 0);
        self.eflags.set(EFlags::ZF, result == 0);
        self.eflags.set(EFlags::PF, result.count_ones().is_multiple_of(2));
    }

    pub(super) fn update_flags_logic16(&mut self, result: u16) {
        self.eflags.remove(EFlags::OF | EFlags::CF | EFlags::AF);
        self.eflags.set(EFlags::SF, (result & 0x8000) != 0);
        self.eflags.set(EFlags::ZF, result == 0);
        self.eflags
            .set(EFlags::PF, ((result & 0xFF) as u8).count_ones().is_multiple_of(2));
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
        // Bochs SET_FLAGS_OSZAPC_LOGIC clears OF, CF, AF
        self.eflags.remove(EFlags::OF | EFlags::CF | EFlags::AF);

        // Set SF (sign flag) - bit 31 of result for 32-bit
        self.eflags.set(EFlags::SF, (result & 0x80000000) != 0);

        // Set ZF (zero flag)
        self.eflags.set(EFlags::ZF, result == 0);

        // Set PF (parity flag), based on low 8 bits
        let low_byte = (result & 0xFF) as u8;
        self.eflags.set(EFlags::PF, low_byte.count_ones().is_multiple_of(2));
    }

    fn before_execution(&mut self, _cpu_id: u32) {
        // Populate RIP ring buffer for post-mortem analysis.
        // Cheap: one array write per instruction, no I/O.
        #[cfg(debug_assertions)] {
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
    pub(super) fn prefetch(&mut self, mem: &'c mut BxMemC<'c>, _cpus: &[&Self]) -> Result<()> {
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
            tracing::debug!(
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
            // Bochs populates ITLB from DTLB, so whenever ITLB has an entry
            // the DTLB also had it (though it may have been evicted since).
            // Ensure the DTLB still has this page — if evicted, re-populate
            // via a page walk. This is critical during kernel startup where
            // boot page tables overlap with decompressed kernel data: the
            // DTLB must cache translations so data accesses don't walk
            // through corrupted page table entries.
            {
                let dtlb_lpf = self.dtlb.get_entry_of(laddr, 0).lpf;
                if dtlb_lpf != lpf {
                    // Speculative TLB population — miss is expected and harmless
                    let _ = self.translate_data_read(laddr);
                }
            }
            Some(tlb_host_addr)
        } else {
            // TLB miss - need to walk page tables
            // Create a dummy page_write_stamp_table for page table walking
            let mut dummy_mapping: [u32; 0] = [];
            let mut dummy_stamp_table = crate::cpu::icache::BxPageWriteStampTable {
                fine_granularity_mapping: &mut dummy_mapping,
            };
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
                &mut dummy_stamp_table,
            ) {
                Ok(p_addr) => {
                    self.p_addr_fetch_page = ppf_of(p_addr);
                    itlb_should_update = true;
                    tracing::debug!(
                        "prefetch: translate_linear OK, p_addr={:#x}, p_addr_fetch_page={:#x}",
                        p_addr,
                        self.p_addr_fetch_page
                    );
                    // Bochs behaviour: ITLB miss page walk populates BOTH DTLB
                    // and ITLB. The DTLB entry ensures that subsequent data
                    // accesses to the same page as code hit the DTLB without
                    // re-walking the page tables. This is critical during Linux
                    // kernel startup where boot page tables overlap with the
                    // decompressed kernel's page table symbols — the TLB must
                    // shield data accesses from the corrupted boot page tables
                    // until CR3 is switched.
                    // Speculative TLB population — miss is expected and harmless
                    let _ = self.translate_data_read(laddr);
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

        if let Some(fetch_ptr) = fetch_ptr_option {
            let fetch_ptr_as_ptr =
                // SAFETY: pointer and length validated by caller; memory region is valid
                unsafe { super::access::host_slice_u8(fetch_ptr as *const u8, 4096) };
            self.eip_fetch_ptr = Some(fetch_ptr_as_ptr);
        } else {
            let mem_len = mem.get_memory_len();

            let p_addr_fetch_page = self.p_addr_fetch_page;

            match self.get_host_mem_addr(p_addr_fetch_page, MemoryAccessType::Execute, mem) {
                Ok(Some(fetch_ptr)) => {
                    // Bound to 4096 bytes (one page) to prevent the decoder
                    // from reading past the page boundary into physically
                    // adjacent (but virtually different) memory.
                    let bounded_len = fetch_ptr.len().min(4096);
                    self.eip_fetch_ptr = Some(&fetch_ptr[..bounded_len]);
                }
                Ok(None) => {
                    self.eip_fetch_ptr = None;
                }
                Err(_e) => {
                    // Log the error and treat as no direct access
                    tracing::debug!("Failed to get host mem addr for fetch: {:?}", _e);
                    self.eip_fetch_ptr = None;
                }
            }
            // Populate iTLB after a successful translate_linear so the next prefetch to this
            // page hits the TLB instead of re-walking the page tables (avoids 200x slowdown).
            if itlb_should_update {
                if let Some(fp) = self.eip_fetch_ptr {
                    let host_page_ptr = fp.as_ptr() as super::tlb::BxHostpageaddr;
                    let ppf = self.p_addr_fetch_page;
                    // access_bits bit 0 = supervisor, bit 1 = user (matches the TLB hit check).
                    let access_bits = 1u32 << (self.user_pl as u32);
                    let tlb_entry = self.itlb.get_entry_of(lpf, 0);
                    tlb_entry.lpf = lpf;
                    tlb_entry.ppf = ppf;
                    tlb_entry.access_bits = access_bits;
                    tlb_entry.lpf_mask = 0xFFF;
                    tlb_entry.host_page_addr = host_page_ptr;
                }
            }
            let p_addr: BxPhyAddress = self.p_addr_fetch_page + u64::from(page_offset);
            if self.eip_fetch_ptr.is_none() && p_addr >= mem_len.try_into()? {
                // Address is beyond available memory - set to no direct access
                tracing::debug!("prefetch: address {p_addr:#x} beyond memory limit {mem_len:#x} and no ROM mapping");
                self.eip_fetch_ptr = None;
            }
        }

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
            tracing::debug!("BxError: Encountered an unknown instruction (signalling #UD)");
        } else {
            tracing::debug!("{:?}: instruction not supported - signalling #UD", opcode);
        }

        // Boot diagnostic: report the first unsupported opcode via port 0xE9.
        // If BIOS hits #UD early, it may vector to 0000:0000 and appear to “do nothing”.
        if (self.boot_debug_flags & 0x01) == 0 {
            self.boot_debug_flags |= 0x01;
            self.debug_puts(b"[UD]\n");
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
    #[cfg(feature = "bx_support_sse")]
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
    #[cfg(feature = "bx_support_avx")]
    pub(super) fn bx_no_avx(&mut self, instr: &Instruction) -> Result<()> {
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

        // Check XCR0 for SSE and YMM masks
        let xcr0 = self.xcr0.get32();
        const XCR0_SSE_MASK: u32 = 1 << 0;
        const XCR0_YMM_MASK: u32 = 1 << 2;
        if (xcr0 & (XCR0_SSE_MASK | XCR0_YMM_MASK)) != (XCR0_SSE_MASK | XCR0_YMM_MASK) {
            self.exception(Exception::Ud, 0)?;
            return Ok(());
        }

        let cr0_ts = (cr0 & (1 << 3)) != 0; // CR0.TS bit 3
        if cr0_ts {
            self.exception(Exception::Nm, 0)?;
        }

        // BX_ASSERT(0) in original - this should not be reached in normal operation
        tracing::warn!("BxNoAVX: AVX instruction executed but AVX not available");
        Ok(())
    }

    /// BxNoOpMask - Opmask not available handler
    /// Matches BX_CPU_C::BxNoOpMask from proc_ctrl.cc
    /// Only available if BX_SUPPORT_EVEX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    #[cfg(feature = "bx_support_evex")]
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
    #[cfg(feature = "bx_support_evex")]
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
    #[cfg(feature = "bx_support_amx")]
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
    ) -> Result<(bool, Option<InstructionHandler<I>>)> {
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
        let mut selected_handler: Option<InstructionHandler<I>> = None;
        let is_bx_error = false; // Track if BxError handler was assigned

        if let Some(entry) = &opcode_entry {
            // Handler assignment from table
            if !is_reg_form {
                // Memory form: use execute1 from table (matching line 2046)
                selected_handler = Some(entry.execute1);

                // Special case: MOV with SS segment override (matching lines 2049-2056)
                if ia_opcode == Opcode::MovOp32GdEd
                    && instr.seg() == BxSegregs::Ss as u8 {
                        // Use MOV32S_GdEdM handler (matching C++ line 2051)
                        use super::opcodes_table::mov32s_gd_ed_m_wrapper;
                        selected_handler = Some(mov32s_gd_ed_m_wrapper);
                    }
                if ia_opcode == Opcode::MovOp32EdGd
                    && instr.seg() == BxSegregs::Ss as u8 {
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
        #[cfg(feature = "bx_support_evex")]
        {
            if op_flags.contains(OpFlags::PREPARE_EVEX) {
                if instr.get_evex_b() != 0 {
                    if !is_reg_form {
                        // Memory form: check NO_BROADCAST
                        if op_flags.contains(OpFlags::PREPARE_EVEX_NO_BROADCAST) {
                            tracing::debug!(
                                "{:?}: broadcast is not supported for this instruction",
                                ia_opcode
                            );
                            // Matching C++ line 2073: assign BxError immediately
                            selected_handler = Some(Self::bx_error);
                            is_bx_error = true;
                        }
                    } else {
                        // Register form: check NO_SAE
                        if op_flags.contains(OpFlags::PREPARE_EVEX_NO_SAE) {
                            tracing::debug!(
                                "{:?}: EVEX.b in reg form is not allowed for instructions which cannot cause floating point exception",
                                ia_opcode
                            );
                            // Matching C++ line 2079: assign BxError immediately
                            use super::opcodes_table::bx_error_wrapper;
                            selected_handler = Some(bx_error_wrapper);
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
        #[cfg(feature = "bx_support_sse")]
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
        #[cfg(feature = "bx_support_avx")]
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
        #[cfg(feature = "bx_support_evex")]
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
        #[cfg(feature = "bx_support_evex")]
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
        #[cfg(feature = "bx_support_amx")]
        {
            if !fetch_mode_mask.contains(FetchModeMask::AMX_OK)
                && op_flags.contains(OpFlags::PREPARE_AMX) {
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
