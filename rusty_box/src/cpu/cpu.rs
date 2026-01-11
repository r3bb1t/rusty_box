use core::{cell::UnsafeCell, marker::PhantomData};

use crate::{
    config::{BxAddress, BxPhyAddress, BxPtrEquiv},
    cpu::{
        cpuid::{SVMExtensions, VMXExtensions},
        crregs::BxEfer,
        decoder::{features::X86Feature, BxSegregs, BX_64BIT_REG_RIP},
        paging::translate_linear,
        rusty_box::MemoryAccessType,
        smm::SMMRAM_Fields,
        tlb::{lpf_of, page_offset, ppf_of, Tlb},
        CpuError,
    },
    impl_eflag,
    memory::{BxMemC, BxMemoryStubC},
};

use super::{
    apic::BxLocalApic,
    cpuid::BxCpuIdTrait,
    cpustats::BxCpuStatistics,
    crregs::{BxCr0, BxCr4, BxDr6, BxDr7, Xcr0, MSR},
    decoder::{BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE, BX_XMM_REGISTERS, BxInstructionGenerated},
    descriptor::{BxGlobalSegmentReg, BxSegmentReg},
    i387::{BxPackedRegister, I387},
    icache::{BxIcache, BxIcacheEntry},
    lazy_flags::BxLazyflagsEntry,
    segment_ctrl_pro::parse_selector,
    svm::VmcbCache,
    tlb::BxHostpageaddr,
    vmx::{VmcsCache, VmcsMapping, VmxCap},
    xmm::{BxMxcsr, BxZmmReg},
    Result,
};

use crate::cpu::decoder::{decode_simple_32, fetch_decode32_chatgpt_generated_instr};

pub(super) const BX_ASYNC_EVENT_STOP_TRACE: u32 = 1 << 31;

const BX_DTLB_SIZE: usize = 2048;
const BX_ITLB_SIZE: usize = 1024;

#[cfg(feature = "bx_support_amx")]
use super::avx::amx::AMX;

#[cfg(feature = "bx_support_memtype")]
use super::tlb::BxMemType;

// region:  x64 big endian

#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWord {
    pub dword_filler: u16,
    pub word_filler: u16,
    pub rx: u16,
    pub byte: BxWordByte,
}

#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxGenRegDword {
    pub hrx: u32,
    pub erx: u32,
}

#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWordInner {
    pub rx: y16,
    pub byte: BxWordByte,
}

#[cfg(feature = "bx_big_endian")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxWordByte {
    pub rh: u8,
    pub rl: u8,
}

// endregion:  x64 big endian

// region:  x64 little endian

#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenReg {
    pub word: BxGenRegWord,
    pub rrx: u64,
    pub dword: BxGenRegDword,
}

impl Default for BxGenReg {
    fn default() -> Self {
        Self { rrx: 0 }
    }
}

#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Copy, Clone)]
pub union BxGenRegWord {
    pub rx: u16,
    pub byte: BxWordByte,
    pub word_filler: u16,
    pub dword_filler: u16,
}

#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxGenRegDword {
    pub erx: u32,
    pub hrx: u32,
}

#[cfg(not(feature = "bx_big_endian"))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct BxWordByte {
    pub rl: u8,
    pub rh: u8,
}

// endregion:  x64 little endian

impl core::fmt::Debug for BxGenReg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", unsafe { self.rrx })?;
        Ok(())
    }
}

// <TAG-INSTRUMENTATION_COMMON-BEGIN>

// possible types passed to BX_INSTR_TLB_CNTRL()
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

#[derive(PartialEq, Clone, Debug)]
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

pub(super) enum CpExceptionErrorCode {
    NearRet = 1,
    FarRetIret = 2,
    Endbranch = 3,
    Rstorssp = 4,
    SETSSBSY = 5,
}

pub(super) const BX_CPU_HANDLED_EXCEPTIONS: usize = 32;

pub(super) enum ExceptionClass {
    Trap = 0,
    Fault = 1,
    Abort = 2,
}

#[derive(Debug, Default, PartialEq)]
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

#[derive(Debug, Default)]
pub(super) enum CpuActivityState {
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
    pub(super) eflags: u32, // Raw 32-bit value in x86 bit position.

    /// lazy arithmetic flags state
    pub(super) oszapc: BxLazyflagsEntry,

    /// so that we can back up when handling faults, exceptions, etc.
    /// we need to store the value of the instruction pointer, before
    /// each fetch/execute cycle.
    pub(super) prev_rip: BxAddress,
    pub(super) prev_rsp: BxAddress,

    pub(super) prev_ssp: BxAddress,
    pub(super) speculative_rsp: bool,

    pub(super) icount: u64,
    pub(super) icount_last_sync: u64,

    /// What events to inhibit at any given time.  Certain instructions
    /// inhibit interrupts, some debug exceptions and single-step traps.
    pub(super) inhibit_mask: u32,
    pub(super) inhibit_icount: u64,

    /// user segment register set
    pub(super) sregs: [BxSegmentReg; 6],

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
    pub(super) cr0: BxCr0,
    pub(super) cr2: BxAddress,
    pub(super) cr3: BxAddress,

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
    #[cfg(feature = "bx_support_pkeys")]
    pub(super) pkru: u32,
    #[cfg(feature = "bx_support_pkeys")]
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
    #[cfg(feature = "bx_support_pkeys")]
    pub(super) rd_pkey: [u32; 16],
    #[cfg(feature = "bx_support_pkeys")]
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

    #[cfg(feature = "bx_support_monitor_mwait")]
    pub(super) monitor: MonitorAddr,

    #[cfg(feature = "bx_support_apic")]
    pub(super) lapic: BxLocalApic,

    /// SMM base register
    pub(super) smbase: u32,

    pub(super) msr: BxRegsMsr,

    #[cfg(feature = "bx_configure_msrs")]
    pub(super) msrs: [MSR; BX_MSR_MAX_INDEX],

    #[cfg(feature = "bx_support_amx")]
    pub(super) amx: Option<AMX>,

    pub(super) in_vmx: bool,
    pub(super) in_vmx_guest: bool,
    /// save in_vmx and in_vmx_guest flags when in SMM mode
    pub(super) in_smm_vmx: bool,
    pub(super) in_smm_vmx_guest: bool,
    pub(super) vmcsptr: u64,

    #[cfg(feature = "bx_support_memtype")]
    vmcs_memtype: BxMemType,

    pub(super) vmxonptr: u64,

    pub(super) vmcs: VmcsCache,
    pub(super) vmx_cap: VmxCap,
    pub(super) vmcs_map: VmcsMapping,

    pub(super) in_svm_guest: bool,
    /// global interrupt enable flag, when zero all external interrupt disabled
    pub(super) svm_gif: bool,
    pub(super) vmcbptr: BxPhyAddress,
    pub(super) vmcbhostptr: BxHostpageaddr,
    #[cfg(feature = "bx_support_memtype")]
    vmcb_memtype: BxMemType,

    pub(super) vmcb: Option<VmcbCache>,

    pub(super) in_event: bool,

    pub(super) nmi_unblocking_iret: bool,

    /// 1 if processing external interrupt or exception
    /// or if not related to current instruction,
    /// 0 if current CS:EIP caused exception */
    pub(super) ext: bool,

    // Todo: Maybe enum?
    // pub(super) activity_state: u32,
    pub(super) activity_state: CpuActivityState,

    pub(super) pending_event: u32,
    pub(super) event_mask: u32,
    // keep 32-bit because of BX_ASYNC_EVENT_STOP_TRACE
    pub(super) async_event: u32,

    pub(super) in_smm: bool,
    pub(super) cpu_mode: CpuMode,
    pub(super) user_pl: bool,

    pub(super) ignore_bad_msrs: bool,

    pub(super) cpu_state_use_ok: u32, // format of BX_FETCH_MODE_*

    // FIXME: skipped   static jmp_buf jmp_buf_env;
    pub(super) last_exception_type: u32,

    #[cfg(feature = "bx_support_handlers_chaining_speedups")]
    pub(super) cpuloop_stack_anchor: Option<&'c [u8]>,

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

    #[cfg(feature = "bx_support_memtype")]
    espPageMemtype: BxMemType,

    #[cfg(not(feature = "bx_support_smp"))]
    pub(super) esp_page_fine_granularity_mapping: u32,

    #[cfg(feature = "bx_support_alignment_check")]
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
    far_branch: FarBranch,

    pub(super) dtlb: Tlb<BX_DTLB_SIZE>,
    pub(super) itlb: Tlb<BX_ITLB_SIZE>,

    pub(super) pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    //pub(super) i_cache: BxIcache,
    pub(super) i_cache: super::i_cache_v2::BxICache,
    pub(super) fetch_mode_mask: u32,

    pub(super) address_xlation: AddressXlation,

    /* Now other not so obvious fields */
    pub(super) smram_map: [u32; SMMRAM_Fields::SMRAM_FIELD_LAST as _],

    pub(super) phantom: PhantomData<I>,

    /// Temporary memory pointer for instruction execution (set during cpu_loop)
    /// This is a raw pointer to avoid lifetime issues - only valid during cpu_loop
    /// SAFETY: Must only be used during cpu_loop when memory is valid
    pub(super) mem_ptr: Option<*mut u8>,
    pub(super) mem_len: usize,
}

impl<I: BxCpuIdTrait> BxCpuC<'_, I> {
    pub(super) const BX_ASYNC_EVENT_STOP_TRACE: u32 = 1 << 31;
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

    pub(super) fn bx_write_opmask(&mut self, index: usize, val_64: u64) {
        self.opmask[index].rrx = val_64;
    }
}

#[derive(Debug, Default)]
pub(super) struct AddressXlation {
    /// The address offset after resolution
    rm_addr: BxPhyAddress,
    /// physical address after translation of 1st len1 bytes of data
    paddress1: BxPhyAddress,
    /// physical address after translation of 2nd len2 bytes of data
    paddress2: BxPhyAddress,
    /// Number of bytes in page 1
    len1: u32,
    // Number of bytes in page 2
    len2: u32,
    /// Number of pages access spans (1 or 2).  Also used
    /// for the case when a native host pointer is
    /// available for the R-M-W instructions.  The host
    /// pointer is stuffed here.  Since this field has
    /// to be checked anyways (and thus cached), if it
    /// is greated than 2 (the maximum possible for
    /// normal cases) it is a native pointer and is used
    /// for a direct write access.
    pages: BxPtrEquiv,
    #[cfg(feature = "bx_support_memtype")]
    /// memory type of the page 1
    memtype1: BxMemType,
    #[cfg(feature = "bx_support_memtype")]
    /// memory type of the page 1
    memtype2: BxMemType,
}

#[derive(Debug, Default)]
pub(super) struct PdptrCache {
    pub entry: [u64; 4],
}

#[derive(Debug)]
struct FarBranch {
    pub rev_cs: u16,
    pub rev_rip: BxAddress,
}

#[derive(Debug)]
enum BxCpuActivityState {
    ActivityStateActive = 0,
    ActivityStateHlt,
    ActivityStateShutdown,
    ActivityStateWaitForSipi,
    VmxLastActivityState,
    ActivityStateMwait,
    ActivityStateMwaitIf,
}

// Hack since duplicated 3
impl From<BxCpuActivityState> for u8 {
    fn from(value: BxCpuActivityState) -> Self {
        match value {
            BxCpuActivityState::ActivityStateActive => 0,
            BxCpuActivityState::ActivityStateHlt => 1,
            BxCpuActivityState::ActivityStateShutdown => 2,
            BxCpuActivityState::ActivityStateWaitForSipi
            | BxCpuActivityState::VmxLastActivityState => 3,
            BxCpuActivityState::ActivityStateMwait => 4,
            BxCpuActivityState::ActivityStateMwaitIf => 5,
        }
    }
}

impl Default for BxCpuActivityState {
    fn default() -> Self {
        Self::VmxLastActivityState
    }
}

#[derive(Debug, Default)]
pub struct BxRegsMsr {
    #[cfg(feature = "bx_support_apic")]
    pub apicbase: BxPhyAddress,

    // SYSCALL/SYSRET instruction msr's
    pub star: u64,

    pub lstar: u64,
    pub cstar: u64,
    pub fmask: u32,
    pub kernelgsbase: u64,
    pub tsc_aux: u32,

    // SYSENTER/SYSEXIT instruction msr's
    pub sysenter_cs_msr: u32,
    pub sysenter_esp_msr: BxAddress,
    pub sysenter_eip_msr: BxAddress,

    pub pat: BxPackedRegister,
    pub mtrrphys: [u64; 16],
    pub mtrrfix64k: BxPackedRegister,
    pub mtrrfix16k: [BxPackedRegister; 2],
    pub mtrrfix4k: [BxPackedRegister; 8],
    pub mtrr_deftype: u32,

    pub ia32_feature_ctrl: u32,

    pub svm_vm_cr: u32,
    pub svm_hsave_pa: u64,

    pub ia32_xss: u64,

    pub ia32_cet_control: [u64; 2], // indexed by CPL==3
    pub ia32_pl_ssp: [u64; 4],
    pub ia32_interrupt_ssp_table: u64,

    pub ia32_umwait_ctrl: u32,
    pub ia32_spec_ctrl: u32, // SCA

                             // note from bochs source code:
                             /* TODO finish of the others */
                             //
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

    //    fn BX_WRITE_8BIT_REGx(index, extended, val) {\
    //  if (((index) & 4) == 0 || (extended)) \
    //    BX_CPU_THIS_PTR gen_reg[index].word.byte.rl = val; \
    //  else \
    //    BX_CPU_THIS_PTR gen_reg[(index)-4].word.byte.rh = val; \
    //}

    fn bx_write_32bit_regz(&mut self, index: usize, val: u32) {
        self.gen_reg[index].rrx = val as _;
    }

    fn bx_write_64bit_reg(&mut self, index: usize, val: u64) {
        self.gen_reg[index].rrx = val;
    }
    fn bx_clear_64bit_high(&mut self, index: usize) {
        unsafe {
            self.gen_reg[index].dword.hrx = 0;
        }
    }

    fn get_laddr32(&self, seg: usize, offset: u32) -> u32 {
        (unsafe { self.sregs[seg].cache.u.segment.base } + u64::from(offset)) as u32
    }
}

#[cfg(feature = "bx_support_monitor_mwait")]
#[derive(Debug, Default)]
pub struct MonitorAddr {
    monitor_addr: BxPhyAddress,
    armed_by: u32,
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
#[derive(Debug)]
struct BxDbgGuardState {
    /// cs:eip and linear addr of instruction at guard point
    cs: u32,
    eip: BxAddress,
    laddr: BxAddress,
    // 00 - 16 bit, 01 - 32 bit, 10 - 64-bit, 11 - illegal
    code_32_64: u32, // CS seg size at guard point
}

#[cfg(feature = "bx_debugger")]
#[derive(Debug)]
struct BxGuardFound {
    guard_found: u32,
    icount_max: u64, // stop after completing this many instructions
    iaddr_index: u32,
    guard_state: BxDbgGuardState,
}

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    pub fn cpu_loop(&mut self, mem: &'c mut BxMemC<'c>, cpus: &[&Self]) -> super::Result<()> {
        let stack_anchor = 0;

        self.cpuloop_stack_anchor = None;

        // FIXME: setjmp

        // We get here either by a normal function call, or by a longjmp
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

        self.cpu_loop_n(mem, cpus, 1_000_000)?;
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
    ) -> super::Result<u64> {
        // Set memory pointer for instruction execution
        // Store raw pointer to the memory vector for direct access
        let (mem_vector, mem_len) = mem.get_raw_memory_ptr();
        self.mem_ptr = Some(mem_vector);
        self.mem_len = mem_len;
        
        let mut iteration = 0u64;
        let result = loop {
            iteration += 1;
            
            // Safety limit - pause when instruction limit is reached
            if iteration > max_instructions {
                break Ok(iteration - 1);
            }
            
            // check on events which occurred for previous instructions (traps)
            // and ones which are asynchronous to the CPU (hardware interrupts)
            if self.async_event != 0 {
                self.handle_async_event();
                // If request to return to caller ASAP.
                break Ok(iteration);
            }

            // SAFETY: We extend the lifetime of mem temporarily for this call only.
            // The borrow is released at the end of the expression.
            let entry = unsafe {
                let mem_extended: &'c mut BxMemC<'c> = 
                    core::mem::transmute::<&mut BxMemC<'c>, &'c mut BxMemC<'c>>(mem);
                self.get_icache_entry(mem_extended, cpus)?
            };
            let mut i = entry.i;

            self.before_execution(self.bx_cpuid);
            let old_rip = self.rip();
            self.set_rip(old_rip + u64::from(i.ilen()));

            // Execute decoded instruction (simple executor for mov/add/sub)
            if let Err(e) = self.execute_instruction(&mut i) {
                tracing::warn!("instruction execution returned error: {:?}", e);
            }

            // TODO: And syncing of time
            if self.async_event > 0 {
                // clear stop trace magic indication that probably was set by repeat or branch32/64
                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
            }
        };
        
        // Clear memory pointer when done
        self.mem_ptr = None;
        result
    }

    fn fetch_next_instruction(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<BxInstructionGenerated> {
        let entry = self.get_icache_entry(mem, cpus)?;
        Ok(entry.i)
    }

    fn get_icache_entry(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<BxIcacheEntry> {
        // Check if we need to prefetch a new page
        // eip_page_window_size == 0 means we haven't prefetched yet
        if self.eip_page_window_size == 0 || self.eip_fetch_ptr.is_none() {
            self.prefetch(mem, cpus)?;
        }

        // Calculate the offset within the current page
        // In real/protected mode: laddr = CS.base + EIP
        // page_offset = laddr & 0xFFF
        let laddr = if self.long64_mode() {
            self.rip()
        } else {
            BxAddress::from(self.get_laddr32(BxSegregs::Cs as _, self.eip()))
        };
        let page_offset = (laddr & 0xFFF) as usize;
        
        // Physical address for this instruction
        let p_addr: BxPhyAddress = self.p_addr_fetch_page | (page_offset as u64);
        
        tracing::debug!("get_icache_entry: laddr={:#x}, page_offset={:#x}, p_addr={:#x}", 
            laddr, page_offset, p_addr);

        let entry_option = self.i_cache.find_entry(p_addr, self.fetch_mode_mask.into());

        if entry_option.is_none() {
            // iCache miss. Try a tiny inline decoder first (simple cases)
            tracing::trace!("iCache miss at {p_addr:#x}, attempting decode");
            // try to use direct fetch pointer if available
            if let Some(fetch_ptr) = self.eip_fetch_ptr {
                if page_offset < fetch_ptr.len() {
                    let slice = &fetch_ptr[page_offset..];
                    tracing::trace!("Decoding from fetch_ptr, first bytes: {:02x?}", 
                        &slice[..core::cmp::min(16, slice.len())]);
                    
                    // Try simple decoder first (faster for common instructions)
                    if let Some(decoded) = decode_simple_32(slice) {
                        let tlen = decoded.meta_info.ilen as u32;
                        tracing::trace!("Simple decoded: opcode={:?}, ilen={}", 
                            decoded.meta_info.ia_opcode, decoded.meta_info.ilen);
                        return Ok(BxIcacheEntry { p_addr, trace_mask: 0, tlen, i: decoded });
                    }
                    
                    // Fallback to full decoder
                    // Use 16-bit mode for real mode (no protected mode yet)
                    let is_32bit = false; // Real mode is 16-bit
                    match fetch_decode32_chatgpt_generated_instr(slice, is_32bit) {
                        Ok(decoded) => {
                            let tlen = decoded.meta_info.ilen as u32;
                            tracing::trace!("Full decoded: opcode={:?}, ilen={}", 
                                decoded.meta_info.ia_opcode, decoded.meta_info.ilen);
                            return Ok(BxIcacheEntry { p_addr, trace_mask: 0, tlen, i: decoded });
                        }
                        Err(e) => {
                            tracing::trace!("Decode failed at {p_addr:#x}: {:?}, bytes: {:02x?}", 
                                e, &slice[..core::cmp::min(8, slice.len())]);
                        }
                    }
                }
            }

            // Fallback: return a NOP instruction as a stub (should not happen normally)
            tracing::trace!("All decoding failed at {p_addr:#x}, returning NOP stub (advancing by 1 byte)");
        }

        // FIXME: Return actual decoded instruction from iCache or decode from memory
        // For now, return a stub entry with NOP (0x90), ilen=1 to advance RIP
        let mut stub = BxInstructionGenerated::default();
        stub.meta_info.ilen = 1; // Ensure RIP advances by 1 byte
        stub.meta_info.ia_opcode = crate::cpu::decoder::Opcode::Nop;
        Ok(BxIcacheEntry {
            p_addr,
            trace_mask: 0,
            tlen: 1,
            i: stub,
        })
    }

    pub(super) fn get_gpr32(&self, idx: usize) -> u32 {
        match idx {
            0 => self.eax(),
            1 => self.ecx(),
            2 => self.edx(),
            3 => self.ebx(),
            4 => self.esp(),
            5 => self.ebp(),
            6 => self.esi(),
            7 => self.edi(),
            _ => 0,
        }
    }

    pub(super) fn set_gpr32(&mut self, idx: usize, val: u32) {
        match idx {
            0 => self.set_eax(val),
            1 => self.set_ecx(val),
            2 => self.set_edx(val),
            3 => self.set_ebx(val),
            4 => self.set_esp(val),
            5 => self.set_ebp(val),
            6 => self.set_esi(val),
            7 => self.set_edi(val),
            _ => (),
        }
    }

    pub(super) fn update_flags_add32(&mut self, op1: u32, op2: u32, res: u32) {
        // CF
        let cf = (res as u64) < (op1 as u64);
        // ZF
        let zf = res == 0;
        // SF
        let sf = (res & 0x8000_0000) != 0;
        // OF : use signed overflow detection
        let of = (op1 as i32).checked_add(op2 as i32).is_none();
        // AF - auxiliary carry (bit 4)
        let af = ((op1 ^ op2 ^ res) & 0x10) != 0;
        // PF - parity of low byte (even parity)
        let low = (res & 0xff) as u8;
        let parity = low.count_ones() % 2 == 0;

        // clear relevant flags
        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf { self.eflags |= 1 << 0; }
        if parity { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    pub(super) fn update_flags_sub32(&mut self, op1: u32, op2: u32, res: u32) {
        // CF for subtraction: borrow occured when op1 < op2
        let cf = op1 < op2;
        let zf = res == 0;
        let sf = (res & 0x8000_0000) != 0;
        // OF: signed overflow on subtraction
        let of = (op1 as i32).checked_sub(op2 as i32).is_none();
        let af = ((op1 ^ op2 ^ res) & 0x10) != 0;
        let low = (res & 0xff) as u8;
        let parity = low.count_ones() % 2 == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf { self.eflags |= 1 << 0; }
        if parity { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    fn execute_instruction(&mut self, instr: &mut BxInstructionGenerated) -> Result<()> {
        use crate::cpu::decoder::Opcode;
        use crate::cpu::arith;
        use crate::cpu::data_xfer;
        
        match instr.get_ia_opcode() {
            // =========================================================================
            // Data transfer (MOV) instructions - 32-bit
            // =========================================================================
            Opcode::MovOp32GdEd => {
                data_xfer::MOV_GdEd_R(self, instr);
                Ok(())
            }
            Opcode::MovOp32EdGd => {
                data_xfer::MOV_EdGd_R(self, instr);
                Ok(())
            }
            Opcode::MovEdId => {
                data_xfer::MOV_EdId_R(self, instr);
                Ok(())
            }
            
            // =========================================================================
            // Data transfer (MOV) instructions - 8-bit
            // =========================================================================
            Opcode::MovGbEb => { self.mov_gb_eb_r(instr); Ok(()) }
            Opcode::MovEbGb => { self.mov_eb_gb_r(instr); Ok(()) }
            Opcode::MovEbIb => { self.mov_rb_ib(instr); Ok(()) }
            
            // =========================================================================
            // Data transfer (MOV) instructions - 16-bit
            // =========================================================================
            Opcode::MovGwEw => { self.mov_gw_ew_r(instr); Ok(()) }
            Opcode::MovEwGw => { self.mov_ew_gw_r(instr); Ok(()) }
            Opcode::MovEwIw => { self.mov_rw_iw(instr); Ok(()) }
            
            // =========================================================================
            // Segment register MOV
            // =========================================================================
            Opcode::MovEwSw => { self.mov_ew_sw(instr); Ok(()) }
            Opcode::MovSwEw => { self.mov_sw_ew(instr); Ok(()) }
            
            // =========================================================================
            // MOV with direct memory offset
            // =========================================================================
            Opcode::MovAlod => {
                // MOV AL, moffs8 - Load AL from memory
                let offset = instr.id() as u64;
                let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
                let addr = ds_base.wrapping_add(offset);
                let val = self.mem_read_byte(addr);
                self.set_al(val);
                Ok(())
            }
            Opcode::MovAxod => {
                // MOV AX, moffs16 - Load AX from memory
                let offset = instr.id() as u64;
                let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
                let addr = ds_base.wrapping_add(offset);
                let val = self.mem_read_word(addr);
                self.set_ax(val);
                Ok(())
            }
            Opcode::MovOdAl => {
                // MOV moffs8, AL - Store AL to memory
                let offset = instr.id() as u64;
                let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
                let addr = ds_base.wrapping_add(offset);
                self.mem_write_byte(addr, self.al());
                Ok(())
            }
            Opcode::MovOdAx => {
                // MOV moffs16, AX - Store AX to memory
                let offset = instr.id() as u64;
                let ds_base = unsafe { self.sregs[BxSegregs::Ds as usize].cache.u.segment.base };
                let addr = ds_base.wrapping_add(offset);
                self.mem_write_word(addr, self.ax());
                Ok(())
            }
            
            // =========================================================================
            // PUSH/POP segment registers
            // =========================================================================
            Opcode::PushOp16Sw => {
                let seg = instr.meta_data[0] as usize;
                let val = self.sregs[seg].selector.value;
                self.push_16(val);
                Ok(())
            }
            Opcode::PopOp16Sw => {
                let seg = instr.meta_data[0] as usize;
                let val = self.pop_16();
                // Don't allow loading CS
                if seg != BxSegregs::Cs as usize {
                    parse_selector(val, &mut self.sregs[seg].selector);
                    unsafe {
                        self.sregs[seg].cache.u.segment.base = (val as u64) << 4;
                    }
                }
                Ok(())
            }
            // Arithmetic (ADD) instructions
            Opcode::AddGdEd => {
                arith::ADD_GdEd_R(self, instr);
                Ok(())
            }
            Opcode::AddEdGd => {
                arith::ADD_EdGd_R(self, instr);
                Ok(())
            }
            Opcode::AddEaxid => {
                arith::ADD_EAX_Id(self, instr);
                Ok(())
            }
            Opcode::AddAlib => {
                // ADD AL, imm8
                let al = self.al();
                let imm = instr.ib();
                let result = al.wrapping_add(imm);
                self.set_al(result);
                self.update_flags_add8(al, imm, result);
                Ok(())
            }
            Opcode::AddEwsIb => {
                // ADD r/m16, imm8 (sign-extended)
                let dst = instr.meta_data[0] as usize;
                let op1 = self.get_gpr16(dst);
                let op2 = (instr.ib() as i8 as i16 as u16);
                let result = op1.wrapping_add(op2);
                self.set_gpr16(dst, result);
                self.update_flags_add16(op1, op2, result);
                Ok(())
            }
            // Arithmetic (SUB) instructions
            Opcode::SubGdEd => {
                arith::SUB_GdEd_R(self, instr);
                Ok(())
            }
            Opcode::SubEdGd => {
                arith::SUB_EdGd_R(self, instr);
                Ok(())
            }
            Opcode::SubEaxid => {
                arith::SUB_EAX_Id(self, instr);
                Ok(())
            }
            // XOR instructions
            Opcode::XorEdGd | Opcode::XorEdGdZeroIdiom => {
                let dst = instr.meta_data[0] as usize;
                let src = instr.meta_data[1] as usize;
                let val1 = self.get_gpr32(dst);
                let val2 = self.get_gpr32(src);
                let result = val1 ^ val2;
                self.set_gpr32(dst, result);
                // Update flags for XOR
                self.update_flags_logic32(result);
                Ok(())
            }
            Opcode::XorGdEd | Opcode::XorGdEdZeroIdiom => {
                let dst = instr.meta_data[0] as usize;
                let src = instr.meta_data[1] as usize;
                let val1 = self.get_gpr32(dst);
                let val2 = self.get_gpr32(src);
                let result = val1 ^ val2;
                self.set_gpr32(dst, result);
                self.update_flags_logic32(result);
                Ok(())
            }
            Opcode::XorEbGb | Opcode::XorGbEb => {
                // XOR r8, r/m8
                let dst = instr.meta_data[0] as usize;
                let src = instr.meta_data[1] as usize;
                let val1 = self.get_gpr8(dst);
                let val2 = self.get_gpr8(src);
                let result = val1 ^ val2;
                self.set_gpr8(dst, result);
                self.update_flags_logic8(result);
                Ok(())
            }
            Opcode::XorEwGw | Opcode::XorGwEw => {
                // XOR r16, r/m16
                let dst = instr.meta_data[0] as usize;
                let src = instr.meta_data[1] as usize;
                let val1 = self.get_gpr16(dst);
                let val2 = self.get_gpr16(src);
                let result = val1 ^ val2;
                self.set_gpr16(dst, result);
                self.update_flags_logic16(result);
                Ok(())
            }
            Opcode::XorAlib => {
                // XOR AL, imm8
                let al = self.al();
                let imm = instr.ib();
                let result = al ^ imm;
                self.set_al(result);
                self.update_flags_logic8(result);
                Ok(())
            }
            // FAR JMP - Jump to absolute address with segment change
            Opcode::JmpfAp => {
                // Operand contains segment:offset (segment in upper 16 bits)
                let operand: u32 = unsafe { core::mem::transmute(instr.modrm_form.operand_data) };
                let offset = (operand & 0xFFFF) as u16;
                let segment = ((operand >> 16) & 0xFFFF) as u16;
                tracing::debug!("FAR JMP to {:04x}:{:04x}", segment, offset);
                
                // In real mode, just update CS and EIP
                // CS.base = segment << 4
                let cs_index = BxSegregs::Cs as usize;
                parse_selector(segment, &mut self.sregs[cs_index].selector);
                self.sregs[cs_index].cache.u.segment.base = ((segment as u32) << 4) as u64;
                self.set_rip(offset as u64);
                
                // Invalidate prefetch since we jumped
                self.eip_fetch_ptr = None;
                self.eip_page_window_size = 0;
                Ok(())
            }
            // Flag manipulation instructions
            Opcode::Cli => {
                // Clear Interrupt Flag
                self.eflags &= !(1 << 9); // IF is bit 9
                tracing::debug!("CLI: Interrupts disabled");
                Ok(())
            }
            Opcode::Sti => {
                // Set Interrupt Flag
                self.eflags |= 1 << 9;
                tracing::debug!("STI: Interrupts enabled");
                Ok(())
            }
            Opcode::Cld => {
                // Clear Direction Flag
                self.eflags &= !(1 << 10); // DF is bit 10
                tracing::debug!("CLD: Direction flag cleared");
                Ok(())
            }
            Opcode::Std => {
                // Set Direction Flag
                self.eflags |= 1 << 10;
                tracing::debug!("STD: Direction flag set");
                Ok(())
            }
            Opcode::Nop => {
                // NOP - do nothing
                Ok(())
            }
            // I/O port instructions
            Opcode::InAlib => {
                self.in_al_ib(instr);
                Ok(())
            }
            Opcode::InAxib => {
                self.in_ax_ib(instr);
                Ok(())
            }
            Opcode::InEaxib => {
                self.in_eax_ib(instr);
                Ok(())
            }
            Opcode::OutIbAl => {
                self.out_ib_al(instr);
                Ok(())
            }
            Opcode::OutIbAx => {
                self.out_ib_ax(instr);
                Ok(())
            }
            Opcode::OutIbEax => {
                self.out_ib_eax(instr);
                Ok(())
            }
            Opcode::InAlDx => {
                self.in_al_dx(instr);
                Ok(())
            }
            Opcode::InAxDx => {
                self.in_ax_dx(instr);
                Ok(())
            }
            Opcode::InEaxDx => {
                self.in_eax_dx(instr);
                Ok(())
            }
            Opcode::OutDxAl => {
                self.out_dx_al(instr);
                Ok(())
            }
            Opcode::OutDxAx => {
                self.out_dx_ax(instr);
                Ok(())
            }
            Opcode::OutDxEax => {
                self.out_dx_eax(instr);
                Ok(())
            }
            
            // =========================================================================
            // Conditional jumps (8-bit displacement, 16-bit mode)
            // =========================================================================
            Opcode::JoJbw => { self.jo_jb(instr); Ok(()) }
            Opcode::JnoJbw => { self.jno_jb(instr); Ok(()) }
            Opcode::JbJbw => { self.jb_jb(instr); Ok(()) }
            Opcode::JnbJbw => { self.jnb_jb(instr); Ok(()) }
            Opcode::JzJbw => { self.jz_jb(instr); Ok(()) }
            Opcode::JnzJbw => { self.jnz_jb(instr); Ok(()) }
            Opcode::JbeJbw => { self.jbe_jb(instr); Ok(()) }
            Opcode::JnbeJbw => { self.jnbe_jb(instr); Ok(()) }
            Opcode::JsJbw => { self.js_jb(instr); Ok(()) }
            Opcode::JnsJbw => { self.jns_jb(instr); Ok(()) }
            Opcode::JpJbw => { self.jp_jb(instr); Ok(()) }
            Opcode::JnpJbw => { self.jnp_jb(instr); Ok(()) }
            Opcode::JlJbw => { self.jl_jb(instr); Ok(()) }
            Opcode::JnlJbw => { self.jnl_jb(instr); Ok(()) }
            Opcode::JleJbw => { self.jle_jb(instr); Ok(()) }
            Opcode::JnleJbw => { self.jnle_jb(instr); Ok(()) }
            
            // Conditional jumps (16-bit displacement)
            Opcode::JzJw => { self.jz_jw(instr); Ok(()) }
            Opcode::JnzJw => { self.jnz_jw(instr); Ok(()) }
            
            // JMP instructions
            Opcode::JmpJbw => { self.jmp_jb(instr); Ok(()) }
            Opcode::JmpJw => { self.jmp_jw(instr); Ok(()) }
            Opcode::JmpJd => { self.jmp_jd(instr); Ok(()) }
            Opcode::JmpEw => { self.jmp_ew_r(instr); Ok(()) }
            Opcode::JmpEd => { self.jmp_ed_r(instr); Ok(()) }
            
            // CALL instructions
            Opcode::CallJw => { self.call_jw(instr); Ok(()) }
            Opcode::CallJd => { self.call_jd(instr); Ok(()) }
            Opcode::CallEw => { self.call_ew_r(instr); Ok(()) }
            Opcode::CallEd => { self.call_ed_r(instr); Ok(()) }
            
            // RET instructions
            Opcode::RetOp16 => { self.ret_near16(instr); Ok(()) }
            Opcode::RetOp16Iw => { self.ret_near16_iw(instr); Ok(()) }
            Opcode::RetOp32 => { self.ret_near32(instr); Ok(()) }
            Opcode::RetOp32Iw => { self.ret_near32_iw(instr); Ok(()) }
            
            // LOOP instructions
            Opcode::LoopJbw => { self.loop16_jb(instr); Ok(()) }
            Opcode::LoopeJbw => { self.loope16_jb(instr); Ok(()) }
            Opcode::LoopneJbw => { self.loopne16_jb(instr); Ok(()) }
            Opcode::JcxzJbw => { self.jcxz_jb(instr); Ok(()) }
            
            // =========================================================================
            // CMP instructions
            // =========================================================================
            Opcode::CmpGbEb => { self.cmp_gb_eb_r(instr); Ok(()) }
            Opcode::CmpGwEw => { self.cmp_gw_ew_r(instr); Ok(()) }
            Opcode::CmpGdEd => { self.cmp_gd_ed_r(instr); Ok(()) }
            Opcode::CmpAlib => { self.cmp_al_ib(instr); Ok(()) }
            Opcode::CmpEbIb => {
                // CMP r/m8, imm8
                let dst = instr.meta_data[0] as usize;
                let op1 = self.get_gpr8(dst);
                let op2 = instr.ib();
                let result = op1.wrapping_sub(op2);
                self.update_flags_sub8(op1, op2, result);
                Ok(())
            }
            Opcode::CmpEbGb => {
                // CMP r/m8, r8
                let dst = instr.meta_data[0] as usize;
                let src = instr.meta_data[1] as usize;
                let op1 = self.get_gpr8(dst);
                let op2 = self.get_gpr8(src);
                let result = op1.wrapping_sub(op2);
                self.update_flags_sub8(op1, op2, result);
                Ok(())
            }
            Opcode::CmpAxiw => { self.cmp_ax_iw(instr); Ok(()) }
            Opcode::CmpEaxid => { self.cmp_eax_id(instr); Ok(()) }
            Opcode::CmpEwIw => { self.cmp_ew_iw_r(instr); Ok(()) }
            Opcode::CmpEdId => { self.cmp_ed_id_r(instr); Ok(()) }
            
            // TEST instructions
            Opcode::TestEbGb => { self.test_eb_gb_r(instr); Ok(()) }
            Opcode::TestEwGw => { self.test_ew_gw_r(instr); Ok(()) }
            Opcode::TestEdGd => { self.test_ed_gd_r(instr); Ok(()) }
            Opcode::TestAlib => { self.test_al_ib(instr); Ok(()) }
            Opcode::TestAxiw => { self.test_ax_iw(instr); Ok(()) }
            Opcode::TestEaxid => { self.test_eax_id(instr); Ok(()) }
            Opcode::TestEwIw => { self.test_ew_iw_r(instr); Ok(()) }
            Opcode::TestEdId => { self.test_ed_id_r(instr); Ok(()) }
            
            // =========================================================================
            // AND/OR/NOT instructions
            // =========================================================================
            Opcode::AndGbEb => { self.and_gb_eb_r(instr); Ok(()) }
            Opcode::AndGwEw => { self.and_gw_ew_r(instr); Ok(()) }
            Opcode::AndGdEd => { self.and_gd_ed_r(instr); Ok(()) }
            Opcode::AndAlib => { self.and_al_ib(instr); Ok(()) }
            Opcode::AndAxiw => { self.and_ax_iw(instr); Ok(()) }
            Opcode::AndEaxid => { self.and_eax_id(instr); Ok(()) }
            Opcode::AndEwIw => { self.and_ew_iw_r(instr); Ok(()) }
            Opcode::AndEdId => { self.and_ed_id_r(instr); Ok(()) }
            
            Opcode::OrGbEb => { self.or_gb_eb_r(instr); Ok(()) }
            Opcode::OrGwEw => { self.or_gw_ew_r(instr); Ok(()) }
            Opcode::OrGdEd => { self.or_gd_ed_r(instr); Ok(()) }
            Opcode::OrAlib => { self.or_al_ib(instr); Ok(()) }
            Opcode::OrAxiw => { self.or_ax_iw(instr); Ok(()) }
            Opcode::OrEaxid => { self.or_eax_id(instr); Ok(()) }
            
            // =========================================================================
            // INC/DEC instructions
            // =========================================================================
            Opcode::IncEw => { self.inc_ew_r(instr); Ok(()) }
            Opcode::IncEd => { self.inc_ed_r(instr); Ok(()) }
            Opcode::DecEw => { self.dec_ew_r(instr); Ok(()) }
            Opcode::DecEd => { self.dec_ed_r(instr); Ok(()) }
            
            // =========================================================================
            // PUSH/POP instructions
            // =========================================================================
            Opcode::PushEw => { self.push_ew_r(instr); Ok(()) }
            Opcode::PushEd => { self.push_ed_r(instr); Ok(()) }
            Opcode::PopEw => { self.pop_ew_r(instr); Ok(()) }
            Opcode::PopEd => { self.pop_ed_r(instr); Ok(()) }
            Opcode::PushaOp16 => { self.pusha16(instr); Ok(()) }
            Opcode::PopaOp16 => { self.popa16(instr); Ok(()) }
            Opcode::PushfFw => { self.pushf_fw(instr); Ok(()) }
            Opcode::PopfFw => { self.popf_fw(instr); Ok(()) }
            Opcode::PushfFd => { self.pushf_fd(instr); Ok(()) }
            Opcode::PopfFd => { self.popf_fd(instr); Ok(()) }
            
            // =========================================================================
            // String instructions
            // =========================================================================
            Opcode::RepMovsbYbXb => { self.rep_movsb16(instr); Ok(()) }
            Opcode::RepStosbYbAl => { self.rep_stosb16(instr); Ok(()) }
            Opcode::RepStoswYwAx => { self.rep_stosw16(instr); Ok(()) }
            Opcode::RepLodsbAlxb => { self.rep_lodsb16(instr); Ok(()) }
            
            // =========================================================================
            // Software interrupts
            // =========================================================================
            Opcode::IntIb => { self.int_ib(instr); Ok(()) }
            Opcode::INT3 => { self.int3(instr); Ok(()) }
            Opcode::IretOp16 => { self.iret16(instr); Ok(()) }
            Opcode::IretOp32 => { self.iret32(instr); Ok(()) }
            Opcode::Hlt => { self.hlt(instr); Ok(()) }
            
            // =========================================================================
            // Shift/Rotate instructions
            // =========================================================================
            Opcode::ShlEbI1 => { self.shl_eb_1(instr); Ok(()) }
            Opcode::ShlEb => { self.shl_eb_cl(instr); Ok(()) }
            Opcode::ShlEbIb => { self.shl_eb_ib(instr); Ok(()) }
            Opcode::ShlEwI1 => { self.shl_ew_1(instr); Ok(()) }
            Opcode::ShlEw => { self.shl_ew_cl(instr); Ok(()) }
            Opcode::ShlEwIb => { self.shl_ew_ib(instr); Ok(()) }
            Opcode::ShlEdI1 => { self.shl_ed_1(instr); Ok(()) }
            Opcode::ShlEd => { self.shl_ed_cl(instr); Ok(()) }
            Opcode::ShlEdIb => { self.shl_ed_ib(instr); Ok(()) }
            
            Opcode::ShrEbI1 => { self.shr_eb_1(instr); Ok(()) }
            Opcode::ShrEb => { self.shr_eb_cl(instr); Ok(()) }
            Opcode::ShrEwI1 => { self.shr_ew_1(instr); Ok(()) }
            Opcode::ShrEw => { self.shr_ew_cl(instr); Ok(()) }
            Opcode::ShrEwIb => { self.shr_ew_ib(instr); Ok(()) }
            Opcode::ShrEdI1 => { self.shr_ed_1(instr); Ok(()) }
            Opcode::ShrEd => { self.shr_ed_cl(instr); Ok(()) }
            
            // =========================================================================
            // Data transfer extensions
            // =========================================================================
            Opcode::LeaGwM => { self.lea_gw_m(instr); Ok(()) }
            Opcode::LeaGdM => { self.lea_gd_m(instr); Ok(()) }
            Opcode::XchgEwGw => { self.xchg_ew_gw(instr); Ok(()) }
            Opcode::XchgEdGd => { self.xchg_ed_gd(instr); Ok(()) }
            Opcode::Cbw => { self.cbw(instr); Ok(()) }
            Opcode::Cwd => { self.cwd(instr); Ok(()) }
            Opcode::Cwde => { self.cwde(instr); Ok(()) }
            Opcode::Cdq => { self.cdq(instr); Ok(()) }
            Opcode::Xlat => { self.xlat(instr); Ok(()) }
            Opcode::Lahf => { self.lahf(instr); Ok(()) }
            Opcode::Sahf => { self.sahf(instr); Ok(()) }
            
            _ => {
                tracing::trace!("Unimplemented opcode: {:?}", instr.get_ia_opcode());
                Ok(()) // unsupported -- treat as NOP for now
            }
        }
    }

    // 8-bit flag updates
    pub(super) fn update_flags_add8(&mut self, op1: u8, op2: u8, result: u8) {
        let cf = result < op1; // Carry occurred
        let zf = result == 0;
        let sf = (result & 0x80) != 0;
        let of = ((op1 ^ result) & (op2 ^ result) & 0x80) != 0; // Signed overflow
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = (result.count_ones() % 2) == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf { self.eflags |= 1 << 0; }
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    pub(super) fn update_flags_add16(&mut self, op1: u16, op2: u16, result: u16) {
        let cf = result < op1;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = ((op1 ^ result) & (op2 ^ result) & 0x8000) != 0;
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = ((result & 0xFF) as u8).count_ones() % 2 == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf { self.eflags |= 1 << 0; }
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    pub(super) fn update_flags_sub8(&mut self, op1: u8, op2: u8, result: u8) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x80) != 0;
        let of = (op1 as i8).checked_sub(op2 as i8).is_none();
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = (result.count_ones() % 2) == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf { self.eflags |= 1 << 0; }
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    pub(super) fn update_flags_sub16(&mut self, op1: u16, op2: u16, result: u16) {
        let cf = op1 < op2;
        let zf = result == 0;
        let sf = (result & 0x8000) != 0;
        let of = (op1 as i16).checked_sub(op2 as i16).is_none();
        let af = ((op1 ^ op2 ^ result) & 0x10) != 0;
        let pf = ((result & 0xFF) as u8).count_ones() % 2 == 0;

        const MASK: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        self.eflags &= !MASK;

        if cf { self.eflags |= 1 << 0; }
        if pf { self.eflags |= 1 << 2; }
        if af { self.eflags |= 1 << 4; }
        if zf { self.eflags |= 1 << 6; }
        if sf { self.eflags |= 1 << 7; }
        if of { self.eflags |= 1 << 11; }
    }

    pub(super) fn update_flags_logic8(&mut self, result: u8) {
        self.eflags &= !((1 << 11) | (1 << 0)); // OF=0, CF=0
        if (result & 0x80) != 0 { self.eflags |= 1 << 7; } else { self.eflags &= !(1 << 7); }
        if result == 0 { self.eflags |= 1 << 6; } else { self.eflags &= !(1 << 6); }
        if (result.count_ones() % 2) == 0 { self.eflags |= 1 << 2; } else { self.eflags &= !(1 << 2); }
    }

    pub(super) fn update_flags_logic16(&mut self, result: u16) {
        self.eflags &= !((1 << 11) | (1 << 0)); // OF=0, CF=0
        if (result & 0x8000) != 0 { self.eflags |= 1 << 7; } else { self.eflags &= !(1 << 7); }
        if result == 0 { self.eflags |= 1 << 6; } else { self.eflags &= !(1 << 6); }
        if (((result & 0xFF) as u8).count_ones() % 2) == 0 { self.eflags |= 1 << 2; } else { self.eflags &= !(1 << 2); }
    }

    pub(super) fn update_flags_logic32(&mut self, result: u32) {
        // Clear OF, CF (always 0 for logical operations)
        self.eflags &= !((1 << 11) | (1 << 0)); // OF=0, CF=0
        
        // Set SF (sign flag) - bit 7 of result for 32-bit
        if (result & 0x80000000) != 0 {
            self.eflags |= 1 << 7;
        } else {
            self.eflags &= !(1 << 7);
        }
        
        // Set ZF (zero flag) - bit 6
        if result == 0 {
            self.eflags |= 1 << 6;
        } else {
            self.eflags &= !(1 << 6);
        }
        
        // Set PF (parity flag) - bit 2, based on low 8 bits
        let low_byte = (result & 0xFF) as u8;
        let ones = low_byte.count_ones();
        if ones % 2 == 0 {
            self.eflags |= 1 << 2;
        } else {
            self.eflags &= !(1 << 2);
        }
    }

    fn before_execution(&mut self, _cpu_id: u32) {
        // FIXME: Implement actual before-execution logic
        // This would include things like checking for traps, updating state, etc.
    }

    // boundaries of consideration:
    //
    //  * physical memory boundary: 1024k (1Megabyte) (increments of...)
    //  * A20 boundary:             1024k (1Megabyte)
    //  * page boundary:            4k
    //  * ROM boundary:             2k (dont care since we are only reading)
    //  * segment boundary:         any
    fn prefetch(&mut self, mem: &'c mut BxMemC<'c>, cpus: &[&Self]) -> Result<()> {
        // let cpus = [&self];
        let mut laddr: BxAddress;
        let mut page_offset;

        if self.long64_mode() {
            if self.is_canonical_access(self.rip(), MemoryAccessType::Execute, self.user_pl()) {
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
                && self.cr4.pvi() | (self.v8086_mode() && self.cr4.vme())
            {
                tracing::error!("prefetch: inconsistent VME state");
                self.exception(Exception::Gp, 0)?;
            }

            self.bx_clear_64bit_high(BX_64BIT_REG_RIP); /* avoid 32-bit EIP wrap */

            laddr = BxAddress::from(self.get_laddr32(BxSegregs::Cs as _, self.eip()));
            page_offset = super::tlb::page_offset(laddr);

            // Calculate RIP at the beginning of the page.
            self.eip_page_bias = BxAddress::from(page_offset.wrapping_sub(self.eip()));

            let limit: u32 = unsafe {
                self.sregs[BxSegregs::Cs as usize]
                    .cache
                    .u
                    .segment
                    .limit_scaled
            };

            let eip = self.eip();
            if eip > limit {
                tracing::error!("prefetch: EIP [{eip:#x}] > CS.limit [{limit:#x}]",);
                self.exception(Exception::Gp, 0)?;
            }

            self.eip_page_window_size = 4096;

            if limit + self.eip_page_window_size < 4096 {
                self.eip_page_window_size = (u64::from(limit) + self.eip_page_bias + 1) as u32;
            }
        }
        // skip the
        // '''cpp
        // '#if BX_X86_DEBUGGER
        // '''
        self.clear_rf();
        let lpf = lpf_of(laddr);
        let tlb_entry = self.itlb.get_entry_of(laddr, 0);

        let fetch_ptr_option = if (tlb_entry.lpf == lpf)
            && (tlb_entry.access_bits & (1 << u32::from(self.user_pl))) != 0
        {
            self.p_addr_fetch_page = tlb_entry.ppf;
            Some(tlb_entry.host_page_addr)
        } else {
            let p_addr =
                translate_linear(tlb_entry, laddr, self.user_pl, MemoryAccessType::Execute, mem.a20_mask());
            self.p_addr_fetch_page = ppf_of(p_addr);
            None
        };

        if let Some(fetch_ptr) = fetch_ptr_option {
            // let fetch_ptr_as_ptr = fetch_ptr as *mut u8;
            let fetch_ptr_as_ptr =
                unsafe { core::slice::from_raw_parts(fetch_ptr as *mut u8, 4096) };
            self.eip_fetch_ptr = Some(fetch_ptr_as_ptr);
        } else {
            // FIXME: Add here
            let mem_len = mem.get_memory_len();

            let p_addr_fetch_page = self.p_addr_fetch_page.clone();

            match self.get_host_mem_addr(p_addr_fetch_page, MemoryAccessType::Execute, mem) {
                Ok(Some(fetch_ptr)) => {
                    self.eip_fetch_ptr = Some(fetch_ptr)
                }
                Ok(None) => {
                    self.eip_fetch_ptr = None;
                }
                Err(_e) => {
                    // Log the error and treat as no direct access
                    tracing::warn!("Failed to get host mem addr for fetch: {:?}", _e);
                    self.eip_fetch_ptr = None;
                }
            }
            // self.eip_fetch_ptr = eip_fetch_ptr.as_deref();
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

    pub(crate) fn smm_mode(&self) -> bool {
        self.in_smm
    }
}

