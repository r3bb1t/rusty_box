use core::{cell::UnsafeCell, marker::PhantomData, ptr::NonNull};

use crate::{
    config::{BxAddress, BxPhyAddress, BxPtrEquiv},
    cpu::{
        cpuid::{SVMExtensions, VMXExtensions},
        crregs::BxEfer,
        decoder::{features::X86Feature, BxSegregs, BX_64BIT_REG_RIP},
        rusty_box::MemoryAccessType,
        smm::SMMRAM_Fields,
        tlb::{lpf_of, page_offset, ppf_of, TLBEntry, Tlb},
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
    decoder::{
        BxInstructionGenerated, BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE,
        BX_XMM_REGISTERS,
    },
    descriptor::{BxGlobalSegmentReg, BxSegmentReg},
    i387::{BxPackedRegister, I387},
    icache::{BxICache, BxICacheEntry as BxIcacheEntry, BX_ICACHE_MEM_POOL},
    lazy_flags::BxLazyflagsEntry,
    segment_ctrl_pro::parse_selector,
    svm::VmcbCache,
    tlb::BxHostpageaddr,
    vmx::{VmcsCache, VmcsMapping, VmxCap},
    xmm::{BxMxcsr, BxZmmReg},
    Result,
};

use crate::cpu::decoder::{decode_simple_32, fetchdecode32, fetchdecode64};

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
    pub rx: u16,
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
impl_eflag!(if, 9); // Interrupt Flag (bit 9)

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
    #[cfg(feature = "bx_support_memtype")]
    pub(super) vmcb_memtype: BxMemType,

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
    pub(super) espPageMemtype: BxMemType,

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
    pub(super) far_branch: FarBranch,

    pub(super) dtlb: Tlb<BX_DTLB_SIZE>,
    pub(super) itlb: Tlb<BX_ITLB_SIZE>,

    pub(super) pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    pub(super) i_cache: BxICache,
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

    /// Debug flags for one-time boot diagnostics (no globals).
    ///
    /// Bit 0: reported unsupported opcode
    /// Bit 1: reported real-mode IVT vector to 0000:0000
    pub(super) boot_debug_flags: u8,
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

#[derive(Debug, Default)]
pub(super) struct FarBranch {
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
    pub(super) fn bx_clear_64bit_high(&mut self, index: usize) {
        unsafe {
            self.gen_reg[index].dword.hrx = 0;
        }
    }

    pub(super) fn get_laddr32(&self, seg: usize, offset: u32) -> u32 {
        (unsafe { self.sregs[seg].cache.u.segment.base } + u64::from(offset)) as u32
    }

    /// Get linear address in 64-bit mode (matching get_laddr64)
    pub(super) fn get_laddr64(&self, seg: usize, offset: u64) -> u64 {
        unsafe { self.sregs[seg].cache.u.segment.base.wrapping_add(offset) }
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
    pub(super) fn mem_write_qword(&mut self, laddr: u64, value: u64) {
        // Write 8 bytes to memory
        let bytes = value.to_le_bytes();
        self.mem_write_byte(laddr, bytes[0]);
        self.mem_write_byte(laddr + 1, bytes[1]);
        self.mem_write_byte(laddr + 2, bytes[2]);
        self.mem_write_byte(laddr + 3, bytes[3]);
        self.mem_write_byte(laddr + 4, bytes[4]);
        self.mem_write_byte(laddr + 5, bytes[5]);
        self.mem_write_byte(laddr + 6, bytes[6]);
        self.mem_write_byte(laddr + 7, bytes[7]);
    }
}

#[cfg(feature = "bx_support_monitor_mwait")]
#[derive(Debug, Default)]
pub struct MonitorAddr {
    pub(super) monitor_addr: BxPhyAddress,
    armed_by: u32,
}

#[cfg(feature = "bx_support_monitor_mwait")]
const BX_MONITOR_NOT_ARMED: u32 = 0;
#[cfg(feature = "bx_support_monitor_mwait")]
const BX_MONITOR_ARMED_BY_MONITOR: u32 = 1;
#[cfg(feature = "bx_support_monitor_mwait")]
const BX_MONITOR_ARMED_BY_MONITORX: u32 = 2;
#[cfg(feature = "bx_support_monitor_mwait")]
const BX_MONITOR_ARMED_BY_UMONITOR: u32 = 3;

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
type InstructionHandler<I> = fn(&mut BxCpuC<'_, I>, &BxInstructionGenerated) -> Result<()>;

impl<'c, I: BxCpuIdTrait> BxCpuC<'c, I> {
    #[inline]
    pub(crate) fn set_io_bus_ptr(&mut self, io: NonNull<crate::iodev::BxDevicesC>) {
        self.io_bus = Some(io);
    }

    #[inline]
    pub(crate) fn clear_io_bus(&mut self) {
        self.io_bus = None;
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
        if let Some(mut io_bus) = self.io_bus {
            // SAFETY: io_bus is execution-scoped and single-CPU today.
            unsafe { io_bus.as_mut().outp(0x00E9, ch as u32, 1) };
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
    pub(crate) fn inject_external_interrupt(&mut self, vector: u8) -> Result<()> {
        // Wake from halt/wait state.
        self.activity_state = CpuActivityState::Active;
        // Clear stop-trace so execution can resume.
        self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;

        if self.real_mode() {
            // Real-mode external interrupts use the IVT at 0000:0000.
            self.interrupt_real_mode(vector);
            Ok(())
        } else {
            // Protected-mode external interrupts go through the IDT gate.
            // `soft_int=false`, no error code pushed for external IRQs.
            self.protected_mode_int(vector, false, false, 0)
        }
    }

    /// True if the CPU is halted or waiting for an event.
    ///
    /// We use this to decide when the outer emulator loop should inject
    /// PIC interrupts (wake-from-HLT), matching Bochs' wait-for-event flow.
    pub(crate) fn is_waiting_for_event(&self) -> bool {
        !matches!(self.activity_state, CpuActivityState::Active)
    }

    /// Execute CPU loop with an attached I/O bus (port handlers).
    ///
    /// This sets the bus pointer for the duration of the call and clears it afterwards.
    #[inline]
    pub fn cpu_loop_n_with_io(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
        max_instructions: u64,
        io: NonNull<crate::iodev::BxDevicesC>,
    ) -> super::Result<u64> {
        self.set_io_bus_ptr(io);
        let result = self.cpu_loop_n(mem, cpus, max_instructions);
        self.clear_io_bus();
        result
    }

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
        // Wire the memory system pointer for the duration of this execution call.
        // This enables Bochs-style "host-pointer-or-fallback" access in mem_read/mem_write.
        // Reborrow `mem` so we don't move the `&mut` binding.
        self.set_mem_bus_ptr(NonNull::from(&mut *mem));

        // Set memory pointer for instruction execution
        // Store raw pointer to the memory vector for direct access
        let (mem_vector, mem_len) = mem.get_raw_memory_ptr();
        self.mem_ptr = Some(mem_vector);
        self.mem_len = mem_len;

        // One-time boot breadcrumb: prove the 0xE9 output pipeline works and show
        // the very first bytes the CPU sees at the current CS:IP.
        if (self.boot_debug_flags & 0x80) == 0 {
            self.boot_debug_flags |= 0x80;
            // Removed hardcoded "[cpu_loop start]" message and reset vector dump per user request
        }

        let mut iteration = 0u64;
        let mut stuck_counter = 0u64;
        let mut last_rip = self.rip();
        let mut rip_history = [0u64; 16]; // Track last 16 RIP values
        let mut history_idx = 0usize;
        const STUCK_THRESHOLD: u64 = 100000; // Warn if RIP doesn't change for this many instructions

        tracing::info!("CPU loop starting at CS:IP = {:04X}:{:08X}",
            unsafe { self.sregs[BxSegregs::Cs as usize].selector.value },
            self.rip());

        let result = 'cpu_loop: loop {
            iteration += 1;

            // Periodic progress logging (every 100k instructions for first 5M)
            if iteration % 100_000 == 0 && iteration <= 5_000_000 {
                let current_rip = self.rip();
                let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                let linear_addr = cs_base + current_rip;
                tracing::info!("Progress: {}k instructions, RIP={:#x}, Linear={:#x}", iteration / 1000, current_rip, linear_addr);
            }

            // Sample instruction execution every 250k instructions (first 5M only)
            if iteration % 250_000 == 0 && iteration <= 5_000_000 {
                let current_rip = self.rip();
                let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                let linear_addr = cs_base + current_rip;

                // Read first few bytes of instruction
                let bytes: Vec<u8> = (0..8).map(|i| self.mem_read_byte(linear_addr + i)).collect();
                tracing::debug!(
                    "Trace [{}k]: RIP={:#010x}, Linear={:#010x}, Bytes=[{:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}]",
                    iteration / 1000, current_rip, linear_addr,
                    bytes[0], bytes[1], bytes[2], bytes[3],
                    bytes[4], bytes[5], bytes[6], bytes[7]
                );
            }

            // Safety limit - pause when instruction limit is reached
            if iteration > max_instructions {
                break Ok(iteration - 1);
            }

            // check on events which occurred for previous instructions (traps)
            // and ones which are asynchronous to the CPU (hardware interrupts)
            // Matches Bochs cpu.cc:170-175
            if self.async_event != 0 {
                if self.handle_async_event() {
                    // If request to return to caller ASAP (e.g., CPU halted).
                    break Ok(iteration);
                }
            }

            // Get raw pointer to mem before the loop to work around borrow checker
            // SAFETY: We'll use this raw pointer to create new references after borrows are released
            let mem_ptr: *mut BxMemC<'c> = mem;

            // SAFETY: We extend the lifetime of mem temporarily for this call only.
            // The borrow is released at the end of the expression.
            let current_rip = self.rip();

            // Check for execution in dangerous low memory (IVT region 0x0000-0x03FF)
            // This typically indicates uninitialized interrupt vectors or stack corruption
            let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
            let linear_addr = cs_base + current_rip;

            if linear_addr < 0x400 && (self.boot_debug_flags & 0x40) == 0 {
                self.boot_debug_flags |= 0x40;

                let cs_sel = self.sregs[BxSegregs::Cs as usize].selector.value;
                let ip = self.get_ip();

                tracing::error!("╔════════════════════════════════════════════════════════════╗");
                tracing::error!("║  EXECUTION IN IVT REGION (0x0000-0x03FF)                  ║");
                tracing::error!("╠════════════════════════════════════════════════════════════╣");
                tracing::error!("║  CS:IP = {:#06x}:{:#06x}, Linear = {:#010x}", cs_sel, ip, linear_addr);
                tracing::error!("║  This indicates uninitialized interrupt vector or corruption!");
                tracing::error!("║");
                tracing::error!("║  Previous RIP values (most recent last):");
                for (i, &rip) in rip_history.iter().enumerate() {
                    if rip != 0 {
                        tracing::error!("║    [{:2}] {:#018x}", i, rip);
                    }
                }
                tracing::error!("╠════════════════════════════════════════════════════════════╣");
                tracing::error!("║  STOPPING EXECUTION - This would infinite loop            ║");
                tracing::error!("╚════════════════════════════════════════════════════════════╝");

                self.debug_puts(b"[IVT->0000:0000]\n[RIP=");
                self.debug_put_hex_u16(ip);
                self.debug_puts(b" cs:ip=");
                self.debug_put_hex_u16(cs_sel);
                self.debug_putc(b':');
                self.debug_put_hex_u16(ip);
                self.debug_puts(b"] ");

                // Dump 8 bytes from the current instruction stream
                let paddr = cs_base.wrapping_add(ip as u64);
                for i in 0..8u64 {
                    let b = self.mem_read_byte(paddr.wrapping_add(i));
                    self.debug_put_hex_u8(b);
                    self.debug_putc(if i == 7 { b'\n' } else { b' ' });
                }

                // STOP execution instead of continuing - this prevents infinite loops
                break Ok(iteration);
            }

            let mut entry = unsafe {
                let mem_extended: &'c mut BxMemC<'c> = &mut *mem_ptr;
                match self.get_icache_entry(mem_extended, cpus) {
                    Ok(e) => e,
                    Err(crate::cpu::CpuError::CpuLoopRestart) => {
                        // Exception delivery during prefetch/fetch: restart decode (Bochs longjmp).
                        self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                        continue 'cpu_loop;
                    }
                    Err(e) => break 'cpu_loop Err(e),
                }
            };
            tracing::debug!(
                "get_icache_entry: RIP={:#x}, entry.tlen={}",
                current_rip,
                entry.tlen
            );

            // Get trace start index from entry (stored when trace was created)
            // In C++, entry->i is a pointer directly into mpool, so we can do pointer arithmetic
            // In Rust, we store the mpool index explicitly
            let mut trace_start_idx = entry.mpool_start_idx;

            // If mpool_start_idx is 0 and this is NOT the first trace (mpindex > tlen),
            // it might be an old entry created before we added this field.
            // However, index 0 is valid for the very first trace, so we need to be careful.
            // Only use fallback if mpindex suggests this is a cached entry (mpindex > tlen)
            // and trace_start_idx is 0, which would be wrong for a cached entry.
            if trace_start_idx == 0 && entry.tlen > 0 && self.i_cache.mpindex > entry.tlen {
                // Fallback: calculate from current mpindex (this is what we used to do)
                // This might be wrong for cached entries, but it's better than 0
                let calculated = self.i_cache.mpindex.saturating_sub(entry.tlen);
                if calculated != 0 {
                    trace_start_idx = calculated;
                    tracing::warn!(
                        "mpool_start_idx was 0 for cached entry, calculated fallback: {}",
                        trace_start_idx
                    );
                }
            }

            // Bounds check: ensure trace_start_idx is valid
            if trace_start_idx >= BX_ICACHE_MEM_POOL {
                tracing::warn!(
                    "trace_start_idx ({}) >= BX_ICACHE_MEM_POOL ({})",
                    trace_start_idx,
                    BX_ICACHE_MEM_POOL
                );
                // Reset to start of mpool as fallback
                trace_start_idx = 0;
            }

            tracing::trace!(
                "Initial trace: RIP={:#x}, trace_start_idx={}, tlen={}, mpool_start_idx={}",
                current_rip,
                trace_start_idx,
                entry.tlen,
                entry.mpool_start_idx
            );

            // Loop through all instructions in the trace (matching C++ cpu.cc:196-222)
            let mut instr_idx = 0usize;
            let mut prev_rip_in_loop = self.rip(); // Track previous RIP for loop detection
            let mut restart_decode = false;
            loop {
                // Bounds check before accessing mpool
                if trace_start_idx + instr_idx >= BX_ICACHE_MEM_POOL {
                    tracing::warn!(
                        "trace_start_idx + instr_idx ({}) >= BX_ICACHE_MEM_POOL, breaking",
                        trace_start_idx + instr_idx
                    );
                    break;
                }

                // Get instruction from trace
                let mpool_idx = trace_start_idx + instr_idx;
                if mpool_idx >= BX_ICACHE_MEM_POOL {
                    tracing::error!(
                        "mpool_idx ({}) >= BX_ICACHE_MEM_POOL ({})",
                        mpool_idx,
                        BX_ICACHE_MEM_POOL
                    );
                    break;
                }
                let mut i = self.i_cache.mpool[mpool_idx];
                let current_rip_for_log = self.rip();
                tracing::trace!("Fetching instruction: trace_start_idx={}, instr_idx={}, mpool_idx={}, opcode={:?}, RIP={:#x}",
                    trace_start_idx, instr_idx, mpool_idx, i.get_ia_opcode(), current_rip_for_log);

                // Debug: Log when entering the I/O function at 0x506
                if current_rip_for_log == 0x506 {
                    let sp = self.sp();
                    let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                    tracing::warn!(
                        "Entering I/O function at F000:0506, SP={:#x}, CS.base={:#x}",
                        sp, cs_base
                    );

                    // Dump memory at addresses we're about to execute
                    use crate::config::BxPhyAddress;
                    for check_addr in [0x4b2, 0x506, 0x508, 0x50a, 0x50c] {
                        let mut buf = [0u8; 16];
                        if let Ok(_) = mem.read_physical_page(
                            &[self],
                            check_addr as BxPhyAddress,
                            buf.len(),
                            &mut buf,
                        ) {
                            tracing::error!("📍 Memory at {:#x}: {:02x?}", check_addr, &buf);
                        }
                    }
                }

                // Log instructions in I/O function to see parameter reads
                if current_rip_for_log >= 0x506 && current_rip_for_log <= 0x520 {
                    let sp = self.sp();
                    let bp = self.bp();
                    let dx = self.dx();
                    let al = self.al();
                    if current_rip_for_log == 0x50b || current_rip_for_log == 0x50e {
                        tracing::warn!(
                            "I/O func param read: RIP={:#x}, opcode={:?}, BP={:#x}, SP={:#x}, DX={:#x}, AL={:#x}",
                            current_rip_for_log, i.get_ia_opcode(), bp, sp, dx, al
                        );
                    }
                }

                // Log caller regions to see if parameters are pushed
                if (current_rip_for_log >= 0xc64 && current_rip_for_log <= 0xc80) ||
                   (current_rip_for_log >= 0xcbe && current_rip_for_log <= 0xcda) {
                    let sp = self.sp();
                    let ax = self.ax();
                    let dx = self.dx();
                    let bp = self.bp();
                    tracing::warn!(
                        "Caller: RIP={:#x}, opcode={:?}, SP={:#x}, AX={:#x}, DX={:#x}, BP={:#x}",
                        current_rip_for_log, i.get_ia_opcode(), sp, ax, dx, bp
                    );
                }
                // Also log the instruction that might jump TO 0xe1d59
                if current_rip_for_log >= 0xe1c00 && current_rip_for_log <= 0xe2000 {
                    tracing::warn!(
                        "RIP in BIOS area: RIP={:#x}, opcode={:?}",
                        current_rip_for_log,
                        i.get_ia_opcode()
                    );
                }

                // Track CS.base corruption - log instructions around 0xe0bf0-0xe0c00
                if current_rip_for_log >= 0xe0bf0 && current_rip_for_log <= 0xe0c00 {
                    let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                    let cs_selector = self.sregs[BxSegregs::Cs as usize].selector.value;
                    tracing::error!(
                        "🔍 CS tracking: RIP={:#x}, opcode={:?}, CS.selector={:#x}, CS.base={:#x}",
                        current_rip_for_log, i.get_ia_opcode(), cs_selector, cs_base
                    );
                }

                // want to allow changing of the instruction inside instrumentation callback
                // Matching C++ line 201: BX_INSTR_BEFORE_EXECUTION(BX_CPU_ID, i);
                self.before_execution(self.bx_cpuid);

                // Check for end-of-trace opcode (InsertedOpcode) before executing
                // InsertedOpcode has length 0 and is used to mark the end of a trace
                use crate::cpu::decoder::Opcode;
                if i.get_ia_opcode() == Opcode::InsertedOpcode {
                    // This is an end-of-trace opcode inserted by gen_dummy_icache_entry
                    // Call bx_end_trace to set the stop trace flag (matching C++ BxEndTrace)
                    self.async_event |= BX_ASYNC_EVENT_STOP_TRACE;
                    // For InsertedOpcode, we still need to set prev_rip and increment icount
                    self.prev_rip = self.rip();
                    self.icount += 1;
                    iteration += 1;
                    instr_idx += 1;
                    if instr_idx >= entry.tlen {
                        // Get new entry (matching C++ line 218-220)
                        entry = unsafe {
                            let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
                            self.get_icache_entry(mem_reborrowed, cpus)?
                        };
                        trace_start_idx = entry.mpool_start_idx;
                        instr_idx = 0;
                    }
                    continue;
                }

                // For normal instructions, check instruction length
                let ilen = i.ilen();
                if ilen == 0 {
                    tracing::error!(
                        "Instruction length is 0 for opcode {:?} at RIP={:#x}!",
                        i.get_ia_opcode(),
                        self.rip()
                    );
                    return Err(crate::cpu::CpuError::UnimplementedOpcode {
                        opcode: format!("{:?}", i.get_ia_opcode()),
                    });
                }

                // Matching C++ line 202: RIP += i->ilen();
                // Advance RIP BEFORE execution (instruction handlers may read RIP and expect it to be advanced)
                // In C++, RIP is a 64-bit register accessed directly: RIP += i->ilen()
                let current_rip = self.rip();
                let mut next_rip = current_rip + u64::from(ilen);

                // In real mode, EIP is 16-bit and should wrap at 0xFFFF
                // Matching C++ vm8086.cc:109: EIP = new_eip & 0xffff
                // We need to mask EIP immediately to prevent incorrect values from being used
                // The high 32 bits will be cleared in prefetch() via BX_CLEAR_64BIT_HIGH
                if self.real_mode() {
                    // Extract low 32 bits (EIP) and mask to 16 bits, then combine with high 32 bits
                    let eip_32bit = (next_rip & 0xFFFFFFFF) as u32;
                    let eip_16bit = eip_32bit & 0xFFFF;
                    // Preserve high 32 bits (will be cleared in prefetch), set low 32 bits to masked EIP
                    next_rip = (next_rip & 0xFFFFFFFF00000000) | u64::from(eip_16bit);
                }

                self.set_rip(next_rip);

                // Enhanced instruction tracing with CS:IP and instruction bytes
                let cs_selector = self.sregs[BxSegregs::Cs as usize].selector.value;
                let ip_16 = (current_rip & 0xFFFF) as u16;
                let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                let paddr = cs_base.wrapping_add(current_rip);

                // Read instruction bytes from memory for display
                let mut instr_bytes = [0u8; 15]; // Max x86 instruction length
                for idx in 0..ilen.min(15) as usize {
                    instr_bytes[idx] = self.mem_read_byte(paddr + idx as u64);
                }

                tracing::trace!(
                    "Execute: {:04X}:{:04X} (phys={:#x})  {:02X?}  {:?}",
                    cs_selector,
                    ip_16,
                    paddr,
                    &instr_bytes[..ilen as usize],
                    i.get_ia_opcode()
                );

                tracing::trace!("{:#x} Executing opcode: {:?} at RIP={:#x}, ilen={}, next_rip={:#x}, trace_start_idx={}, instr_idx={}",
                    self.rip(), i.get_ia_opcode(), current_rip, ilen, next_rip, trace_start_idx, instr_idx);

                // Track _start function execution (0xE0000-0xE0030)
                if current_rip >= 0xE0000 && current_rip <= 0xE0030 {
                    tracing::error!(
                        "🎯 _start: RIP={:#x} opcode={:?} ilen={} next_rip={:#x} | EAX={:#x} ECX={:#x} ESI={:#x} EDI={:#x}",
                        current_rip, i.get_ia_opcode(), ilen, next_rip,
                        self.eax(), self.ecx(), self.esi(), self.edi()
                    );
                }

                // Log every 1M instructions to detect progress
                if iteration % 1_000_000 == 0 {
                    tracing::error!(
                        "📊 Progress: {} instructions executed, RIP={:#x}, opcode={:?}",
                        iteration, current_rip, i.get_ia_opcode()
                    );
                }

                // Matching C++ line 203: BX_CPU_CALL_METHOD(i->execute1, (i));
                // might iterate repeat instruction

                // Assign handler for this instruction (matching original assignHandler logic)
                // This checks feature flags, assigns handler, and determines if trace should end
                let fetch_mode_mask = self.fetch_mode_mask;
                match self.assign_handler(&mut i, fetch_mode_mask) {
                    Ok((should_stop_trace, handler_opt)) => {
                        if should_stop_trace {
                            tracing::debug!("assign_handler returned true, stopping trace");
                            break;
                        }

                        // Execute the instruction using assigned handler if available
                        if let Some(handler) = handler_opt {
                            // Call handler function pointer directly (matching C++ i->execute1(i))
                            match handler(self, &i) {
                                Ok(()) => {
                                    // Instruction executed successfully
                                }
                                Err(crate::cpu::CpuError::CpuNotInitialized) => {
                                    // Prefetch queue invalidated - need to break and get new trace
                                    tracing::debug!(
                                        "handler returned CpuNotInitialized, breaking trace"
                                    );
                                    break;
                                }
                                Err(e) => {
                                    tracing::warn!("handler execution returned error: {:?}", e);
                                    // Continue but instruction may not have executed correctly
                                }
                            }
                        } else {
                            // Handler not in table yet - use execute_instruction match statement
                            match self.execute_instruction(&mut i) {
                                Ok(()) => {
                                    // Instruction executed successfully
                                }
                                Err(crate::cpu::CpuError::CpuNotInitialized) => {
                                    // Prefetch queue invalidated - need to break and get new trace
                                    tracing::debug!("execute_instruction returned CpuNotInitialized, breaking trace");
                                    break;
                                }
                                Err(crate::cpu::CpuError::UnimplementedOpcode { opcode }) => {
                                    // Panic on unimplemented opcode with detailed information
                                    let rip = current_rip;
                                    let cs_base = unsafe {
                                        self.sregs[BxSegregs::Cs as usize].cache.u.segment.base
                                    };
                                    let laddr = cs_base + rip;
                                    let cs_value = unsafe {
                                        self.sregs[BxSegregs::Cs as usize].selector.value
                                    };

                                    // Try to get instruction bytes for debugging
                                    let instr_bytes = if let Some(fetch_ptr) = &self.eip_fetch_ptr {
                                        let page_base = cs_base + (self.eip_page_bias as u64);
                                        let offset = (rip.wrapping_sub(page_base)) as usize;
                                        if offset < fetch_ptr.len()
                                            && offset + ilen as usize <= fetch_ptr.len()
                                        {
                                            fetch_ptr[offset..offset + ilen as usize].to_vec()
                                        } else {
                                            vec![]
                                        }
                                    } else {
                                        vec![]
                                    };

                                    panic!(
                                        "\n\
                                ╔════════════════════════════════════════════════════════════╗\n\
                                ║          UNIMPLEMENTED OPCODE DETECTED                      ║\n\
                                ╠════════════════════════════════════════════════════════════╣\n\
                                ║  Opcode:      {}                                    ║\n\
                                ║  RIP:         {:#018x}                          ║\n\
                                ║  CS:IP:       {:#04x}:{:#04x}                              ║\n\
                                ║  Linear Addr: {:#018x}                          ║\n\
                                ║  Length:      {} bytes                                    ║\n\
                                ║  Bytes:       {:02x?}                                      ║\n\
                                ╠════════════════════════════════════════════════════════════╣\n\
                                ║  Please implement this instruction in:                    ║\n\
                                ║    rusty_box/src/cpu/cpu.rs::execute_instruction()       ║\n\
                                ║                                                             ║\n\
                                ║  Check original C++ implementation in:                     ║\n\
                                ║    cpp_orig/bochs/cpu/decoder/ia_opcodes.def              ║\n\
                                ╚════════════════════════════════════════════════════════════╝\n",
                                        opcode, rip, cs_value, rip, laddr, ilen, instr_bytes
                                    );
                                }
                                Err(crate::cpu::CpuError::CpuLoopRestart) => {
                                    // Bochs longjmp: restart decode/trace immediately, do not
                                    // commit RIP or increment instruction counters for this instruction.
                                    restart_decode = true;
                                    break;
                                }
                                Err(e) => {
                                    // Unlike the old placeholder logic, do NOT continue on CPU errors.
                                    // This corrupts guest execution and quickly leads to bogus RIP=0.
                                    break 'cpu_loop Err(e);
                                }
                            }
                        }
                    }
                    Err(crate::cpu::CpuError::CpuLoopRestart) => {
                        restart_decode = true;
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("assign_handler returned error: {:?}", e);
                        // Fall back to execute_instruction match statement
                        match self.execute_instruction(&mut i) {
                            Ok(()) => {
                                // Instruction executed successfully
                            }
                            Err(crate::cpu::CpuError::CpuNotInitialized) => {
                                tracing::debug!("execute_instruction returned CpuNotInitialized, breaking trace");
                                break;
                            }
                            Err(crate::cpu::CpuError::UnimplementedOpcode { opcode }) => {
                                // Panic on unimplemented opcode with detailed information
                                let rip = current_rip;
                                let cs_base = unsafe {
                                    self.sregs[BxSegregs::Cs as usize].cache.u.segment.base
                                };
                                let laddr = cs_base + rip;
                                let cs_value =
                                    unsafe { self.sregs[BxSegregs::Cs as usize].selector.value };

                                // Try to get instruction bytes for debugging
                                let instr_bytes = if let Some(fetch_ptr) = &self.eip_fetch_ptr {
                                    let page_base = cs_base + (self.eip_page_bias as u64);
                                    let offset = (rip.wrapping_sub(page_base)) as usize;
                                    if offset < fetch_ptr.len()
                                        && offset + ilen as usize <= fetch_ptr.len()
                                    {
                                        fetch_ptr[offset..offset + ilen as usize].to_vec()
                                    } else {
                                        vec![]
                                    }
                                } else {
                                    vec![]
                                };

                                panic!(
                                    "\n\
                                    ╔════════════════════════════════════════════════════════════╗\n\
                                    ║          UNIMPLEMENTED OPCODE DETECTED                      ║\n\
                                    ╠════════════════════════════════════════════════════════════╣\n\
                                    ║  Opcode:      {}                                    ║\n\
                                    ║  RIP:         {:#018x}                          ║\n\
                                    ║  CS:IP:       {:#04x}:{:#04x}                              ║\n\
                                    ║  Linear Addr: {:#018x}                          ║\n\
                                    ║  Length:      {} bytes                                    ║\n\
                                    ║  Bytes:       {:02x?}                                      ║\n\
                                    ╠════════════════════════════════════════════════════════════╣\n\
                                    ║  Please implement this instruction in:                    ║\n\
                                    ║    rusty_box/src/cpu/cpu.rs::execute_instruction()       ║\n\
                                    ║                                                             ║\n\
                                    ║  Check original C++ implementation in:                     ║\n\
                                    ║    cpp_orig/bochs/cpu/decoder/ia_opcodes.def              ║\n\
                                    ╚════════════════════════════════════════════════════════════╝\n",
                                    opcode,
                                    rip,
                                    cs_value,
                                    rip,
                                    laddr,
                                    ilen,
                                    instr_bytes
                                );
                            }
                            Err(crate::cpu::CpuError::CpuLoopRestart) => {
                                restart_decode = true;
                                break;
                            }
                            Err(e) => {
                                break 'cpu_loop Err(e);
                            }
                        }
                    }
                }

                if restart_decode {
                    // Clear STOP_TRACE marker; we're explicitly restarting decode now.
                    self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                    continue 'cpu_loop;
                }

                // Matching C++ line 204: BX_CPU_THIS_PTR prev_rip = RIP; // commit new RIP
                self.prev_rip = self.rip();

                // Matching C++ line 205: BX_INSTR_AFTER_EXECUTION(BX_CPU_ID, i);
                // TODO: Implement BX_INSTR_AFTER_EXECUTION if needed

                // Matching C++ line 206: BX_CPU_THIS_PTR icount++;
                self.icount += 1;
                iteration += 1;

                // Matching C++ line 208: BX_SYNC_TIME_IF_SINGLE_PROCESSOR(0);
                // TODO: Implement BX_SYNC_TIME_IF_SINGLE_PROCESSOR if needed

                // note instructions generating exceptions never reach this point
                // Matching C++ line 211-213: gdbstub_instruction_epilog check
                // TODO: Implement gdbstub_instruction_epilog if needed

                // Matching C++ line 215: if (BX_CPU_THIS_PTR async_event) break;
                if self.async_event != 0 {
                    tracing::trace!("Async event detected, breaking trace loop");
                    break;
                }

                // Matching C++ line 217: if (++i == last)
                // Move to next instruction in trace (increment pointer/index)
                instr_idx += 1;
                tracing::trace!(
                    "Moved to next instruction: instr_idx={}, tlen={}",
                    instr_idx,
                    entry.tlen
                );

                // If we've executed all instructions in the trace, get a new entry
                // Matching C++ lines 217-221: if (++i == last) { entry = getICacheEntry(); i = entry->i; last = i + (entry->tlen); }
                if instr_idx >= entry.tlen {
                    tracing::trace!(
                        "Trace complete: instr_idx={} >= tlen={}, getting new entry at RIP={:#x}",
                        instr_idx,
                        entry.tlen,
                        self.rip()
                    );
                    // Get new entry (matching C++ line 218-220)
                    // SAFETY: We use the raw pointer we got earlier to work around borrow checker
                    // The borrow from the previous get_icache_entry is released, so we can safely create a new reference
                    entry = unsafe {
                        let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
                        match self.get_icache_entry(mem_reborrowed, cpus) {
                            Ok(e) => e,
                            Err(crate::cpu::CpuError::CpuLoopRestart) => {
                                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
                                continue 'cpu_loop;
                            }
                            Err(e) => break 'cpu_loop Err(e),
                        }
                    };

                    // Get trace start index from entry (stored when trace was created)
                    trace_start_idx = entry.mpool_start_idx;
                    tracing::trace!(
                        "New trace: RIP={:#x}, trace_start_idx={}, tlen={}, mpool_start_idx={}",
                        self.rip(),
                        trace_start_idx,
                        entry.tlen,
                        entry.mpool_start_idx
                    );

                    // Debug: Log RIP transitions near the problematic address
                    let rip = self.rip();
                    if rip >= 0xe1d00 && rip <= 0xe1dff {
                        tracing::warn!(
                            "RIP in 0xe1d00 range: RIP={:#x}, prev_rip={:#x}",
                            rip,
                            last_rip
                        );
                    }

                    // Reset for new trace
                    instr_idx = 0;
                }
            }

            // Clear stop trace magic indication (matching C++ line 226)
            // BUT: Don't clear if CPU is halted - we need the flag to be checked by handle_async_event()
            // in the outer loop (line 1176) so it can detect the halt state and return from cpu_loop.
            // Only clear STOP_TRACE if activity_state is Active (normal execution).
            if matches!(self.activity_state, CpuActivityState::Active) {
                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
            }

            // Use the last executed instruction for loop detection
            // If we broke early due to async_event, use the last instruction we executed
            // Otherwise, use the last instruction in the trace
            let last_instr_idx = if instr_idx > 0 { instr_idx - 1 } else { 0 };
            let mut i = self.i_cache.mpool[trace_start_idx + last_instr_idx];

            // Detect infinite loops - check multiple patterns:
            // 1. RIP doesn't change (direct infinite loop)
            // 2. RIP cycles through same small set of addresses (loop with multiple instructions)
            let current_rip = self.rip();

            // 🎯 SPECIFIC RIP TRACKING: Log problematic addresses if needed for debugging
            // (Disabled after fixing MOV_GbEbM memory form handler)

            // ⚠️ ZERO-MEMORY DETECTION: Check if we jumped to zero or low memory
            if current_rip < 0x100 {
                tracing::error!(
                    "❌ JUMPED TO NEAR-ZERO MEMORY! RIP={:#x} after {} instructions",
                    current_rip, iteration
                );
                tracing::error!("   This usually means we jumped to invalid/zeroed memory!");
                tracing::error!("   EAX={:#x} EBX={:#x} ECX={:#x} EDX={:#x}",
                    self.eax(), self.ebx(), self.ecx(), self.edx());
                tracing::error!("   ESP={:#x} EBP={:#x} ESI={:#x} EDI={:#x}",
                    self.esp(), self.ebp(), self.esi(), self.edi());
                // Don't break - let it fail naturally so we see the error
            }

            // ⚠️ ZERO-MEMORY DETECTION: Check if instruction bytes at RIP are all zeros
            let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
            let linear_addr = cs_base + current_rip;
            let mut instr_bytes_check = [0u8; 15];
            for idx in 0..15 {
                instr_bytes_check[idx] = self.mem_read_byte(linear_addr + idx as u64);
            }
            let all_zeros = instr_bytes_check.iter().all(|&b| b == 0);
            if all_zeros {
                tracing::error!(
                    "❌ EXECUTING ZEROED MEMORY! RIP={:#x} Linear={:#x} after {} instructions",
                    current_rip, linear_addr, iteration
                );
                tracing::error!("   All instruction bytes are 0x00!");
                tracing::error!("   This means we're executing from uninitialized/zeroed memory!");
            }

            // 🎯 CS.BASE TRACKING: Find where CS.base gets corrupted to 0
            static mut LAST_CS_BASE: u64 = 0xFFFFFFFFFFFFFFFF;
            let current_cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
            if current_cs_base != unsafe { LAST_CS_BASE } {
                tracing::warn!(
                    "⚠️ CS.BASE CHANGED: {:#010x} → {:#010x} at RIP={:#06x}, opcode={:?}",
                    unsafe { LAST_CS_BASE }, current_cs_base, current_rip, i.get_ia_opcode()
                );
                unsafe { LAST_CS_BASE = current_cs_base; }

                // Critical: Log when CS.base becomes 0
                if current_cs_base == 0 {
                    tracing::error!(
                        "❌ CS.BASE CORRUPTED TO ZERO! RIP={:#x}, opcode={:?}, instruction #{}, CS.selector={:#x}",
                        current_rip, i.get_ia_opcode(), iteration,
                        self.sregs[BxSegregs::Cs as usize].selector.value
                    );
                }
            }

            // 🔍 COUNTDOWN LOOP DEBUGGING: Track loop at 0x2055-0x2074
            static mut LOOP_2055_LOG_COUNTER: u64 = 0;
            if current_rip >= 0x2055 && current_rip <= 0x2074 {
                unsafe {
                    LOOP_2055_LOG_COUNTER += 1;
                    if LOOP_2055_LOG_COUNTER <= 5 {
                        let bp = self.bp();
                        let ss_base = self.sregs[BxSegregs::Ss as usize].cache.u.segment.base;
                        let counter_addr = ss_base + (bp.wrapping_sub(271) & 0xFFFF) as u64;
                        let value_addr = ss_base + (bp.wrapping_sub(547) & 0xFFFF) as u64;
                        let counter_val = self.mem_read_byte(counter_addr);
                        let value_val = self.mem_read_byte(value_addr);
                        tracing::warn!(
                            "🔍 Loop #{}: RIP={:#x}, BP={:#x}, [BP-271]@{:#x}={:#x}, [BP-547]@{:#x}={:#x}, AL={:#x}",
                            LOOP_2055_LOG_COUNTER, current_rip, bp, counter_addr, counter_val,
                            value_addr, value_val, self.al()
                        );
                    } else if LOOP_2055_LOG_COUNTER == 6 {
                        let bp = self.bp();
                        tracing::error!("❌ COUNTDOWN LOOP CONFIRMED INFINITE! BP={:#x} (should be ~0xFFFA)", bp);
                    }
                }
            }

            // Update RIP history (circular buffer)
            rip_history[history_idx] = current_rip;
            history_idx = (history_idx + 1) % rip_history.len();

            // Check if we're stuck at same RIP (jumped back to same instruction)
            if current_rip == prev_rip_in_loop {
                stuck_counter += 1;
                if stuck_counter == STUCK_THRESHOLD {
                    tracing::warn!(
                        "CPU appears stuck in infinite loop at RIP={:#x} (0x{:x}) after {} instructions. Last instruction: {:?}",
                        current_rip, current_rip, stuck_counter, i.get_ia_opcode()
                    );
                } else if stuck_counter > STUCK_THRESHOLD && (stuck_counter % 100000) == 0 {
                    tracing::warn!(
                        "CPU still stuck at RIP={:#x} after {} instructions total",
                        current_rip,
                        stuck_counter
                    );
                }
            }
            // Check if RIP is cycling (pattern repeats in history)
            else if iteration > rip_history.len() as u64 {
                // Count unique RIPs without heap allocations (no_std-friendly).
                let mut unique = [0u64; 16];
                let mut unique_len = 0usize;
                for &rip in rip_history.iter() {
                    if !unique[..unique_len].iter().any(|&x| x == rip) {
                        unique[unique_len] = rip;
                        unique_len += 1;
                        if unique_len > 4 {
                            break;
                        }
                    }
                }

                if unique_len <= 4 && stuck_counter > 10000 {
                    // Very few unique RIPs suggests a tight loop
                    stuck_counter += 1;
                    if stuck_counter == STUCK_THRESHOLD {
                        tracing::warn!(
                            "CPU appears stuck in tight loop (cycling through {} addresses) after {} instructions. Recent RIPs: {:?}",
                            unique_len, stuck_counter, rip_history
                        );
                    }
                } else {
                    stuck_counter = 0; // Reset if pattern breaks
                }
            } else {
                stuck_counter = 0; // Reset counter when RIP changes normally
            }

            // Also log every 5 million instructions to show progress (reduced from 10M)
            if iteration > 0 && (iteration % 5_000_000) == 0 {
                // Log progress periodically
                let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
                let linear_addr = cs_base + current_rip;

                // Read first 8 bytes at RIP
                let mut instr_preview = [0u8; 8];
                for idx in 0..8 {
                    instr_preview[idx] = self.mem_read_byte(linear_addr + idx as u64);
                }

                tracing::info!(
                    "📊 {} instructions: RIP={:#x} Linear={:#x} Bytes={:02X?}",
                    iteration, current_rip, linear_addr, &instr_preview
                );
                tracing::info!(
                    "   CPU State: EAX={:#x} ESP={:#x} EBP={:#x}",
                    self.eax(), self.esp(), self.ebp()
                );
            }

            // VGA BIOS execution detection (0xC0000-0xDFFFF)
            if current_rip >= 0xC0000 && current_rip < 0xE0000 {
                tracing::warn!(
                    "🎨 VGA BIOS EXECUTION: RIP={:#x}, Bytes={:02X?}",
                    current_rip, &instr_bytes_check[0..8]
                );
            }

            // BIOS option ROM scan detection
            static mut LAST_ROM_SCAN_LOG: u64 = 0;
            if current_rip >= 0xF0000 && iteration.saturating_sub(unsafe { LAST_ROM_SCAN_LOG }) > 100_000 {
                // Check if reading from option ROM range
                if linear_addr >= 0xC0000 && linear_addr < 0xE0000 {
                    tracing::info!(
                        "🔍 BIOS accessing Option ROM range: RIP={:#x}, accessing address={:#x}",
                        current_rip, linear_addr
                    );
                    unsafe { LAST_ROM_SCAN_LOG = iteration; }
                }
            }

            // TODO: And syncing of time
            // clear stop trace magic indication that probably was set by repeat or branch32/64
            // BUT: Don't clear if CPU is halted - we need the flag for handle_async_event()
            if self.async_event > 0 && matches!(self.activity_state, CpuActivityState::Active) {
                self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
            }
        };

        // Clear memory pointer when done
        self.mem_ptr = None;
        self.clear_mem_bus();
        result
    }

    fn fetch_next_instruction(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<BxInstructionGenerated> {
        // Get raw pointer to work around borrow checker if needed
        let mem_ptr: *mut BxMemC<'c> = mem;
        let entry = unsafe {
            let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
            self.get_icache_entry(mem_reborrowed, cpus)?
        };
        Ok(entry.i)
    }

    fn get_icache_entry(
        &mut self,
        mem: &'c mut BxMemC<'c>,
        cpus: &[&Self],
    ) -> Result<BxIcacheEntry> {
        // Check if we need to prefetch a new page (matching C++ lines 289-292)
        // If eip_page_window_size is 0, we haven't prefetched yet, so do it now
        let needs_prefetch = self.eip_page_window_size == 0 || {
            // Calculate eip_biased = RIP + eip_page_bias (matching C++ line 287)
            let eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;
            eip_biased >= self.eip_page_window_size
        };

        // Get raw pointer to mem before calling prefetch() to work around borrow checker
        // SAFETY: We're getting a raw pointer, which doesn't create a new borrow
        let mem_ptr: *mut BxMemC<'c> = unsafe { core::ptr::addr_of_mut!(*mem) };

        // Matching C++ cpu.cc:287-292
        let mut eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;

        if needs_prefetch {
            // Matching C++ cpu.cc:289-291 - call prefetch() and recalculate eip_biased after
            // Retry loop: if prefetch raises an exception, the handler invalidates the queue
            // and we need to retry prefetch with the new CPU state
            // Get raw pointer before loop to work around borrow checker
            let mem_ptr: *mut BxMemC<'c> = unsafe { core::ptr::addr_of_mut!(*mem) };
            let mut retry_count = 0;
            loop {
                // SAFETY: We're reborrowing mem in each loop iteration, but prefetch() releases the borrow
                let mem_reborrowed: &'c mut BxMemC<'c> = unsafe { &mut *mem_ptr };
                self.prefetch(mem_reborrowed, cpus)?;

                // After prefetch, check if it completed successfully
                // In C++, exception() uses longjmp so if it fails, we never return here
                // In Rust, exception() returns Ok(()) but invalidates the prefetch queue
                if self.eip_page_window_size == 0 || self.eip_fetch_ptr.is_none() {
                    // Prefetch queue was invalidated (likely due to exception handler)
                    // Retry prefetch with new CPU state (exception handler may have changed RIP/CS)
                    retry_count += 1;
                    if retry_count > 10 {
                        tracing::error!("prefetch retry limit exceeded, RIP={:#x}", self.rip());
                        return Err(crate::cpu::CpuError::CpuNotInitialized);
                    }
                    tracing::debug!(
                        "prefetch queue invalidated after exception, retrying (attempt {})",
                        retry_count
                    );
                    continue; // Retry prefetch
                }

                // Recalculate eip_biased after prefetch (matching C++ line 291)
                eip_biased = (self.rip() as i64).wrapping_add(self.eip_page_bias as i64) as u32;

                // If RIP changed, eip_page_bias should still be valid (it's recalculated in prefetch)
                // But verify it's within bounds
                if eip_biased >= self.eip_page_window_size {
                    tracing::debug!("eip_biased ({}) >= eip_page_window_size ({}) after prefetch, RIP={:#x}, retrying", 
                        eip_biased, self.eip_page_window_size, self.rip());
                    // eip_page_bias might be wrong - invalidate and retry
                    self.eip_fetch_ptr = None;
                    self.eip_page_window_size = 0;
                    retry_count += 1;
                    if retry_count > 10 {
                        tracing::error!("prefetch eip_biased retry limit exceeded");
                        return Err(crate::cpu::CpuError::CpuNotInitialized);
                    }
                    continue; // Retry prefetch
                }

                // Prefetch successful
                break;
            }
        }

        // Physical address for this instruction
        let p_addr: BxPhyAddress = self.p_addr_fetch_page | (eip_biased as u64);

        // Find entry in cache
        let entry_option = self.i_cache.find_entry(p_addr, self.fetch_mode_mask.into());

        // Check if cache miss or entry has invalid instruction (matching C++ line 299)
        if entry_option.is_none() || entry_option.as_ref().unwrap().i.meta_info.ilen == 0 {
            // iCache miss. Call serve_icache_miss
            // Create a dummy page_write_stamp_table for now (matches prefetch approach)
            let mut dummy_mapping: [u32; 0] = [];
            let mut dummy_stamp_table = crate::cpu::icache::BxPageWriteStampTable {
                fine_granularity_mapping: &mut dummy_mapping,
            };

            // Work around borrow checker: prefetch() borrows mem, but that borrow is released when it returns.
            // However, Rust's borrow checker is conservative and doesn't allow us to borrow mem again immediately.
            // We use unsafe to work around this limitation.
            // SAFETY:
            // 1. prefetch() returns before we call serve_icache_miss, so the borrows don't overlap at runtime
            // 2. serve_icache_miss only uses mem for boundary_fetch (error case), not in the common path
            // 3. We're not actually creating overlapping borrows - the borrow from prefetch is released
            // 4. We use the raw pointer we got before prefetch() to create a new reference
            // The borrow checker sees that `mem` is borrowed in the function signature and also used in prefetch(),
            // but it can't prove that the borrow from prefetch() is released before we call serve_icache_miss.
            // We know this is safe because prefetch() returns before serve_icache_miss is called.
            // SAFETY: The borrow from prefetch() is released when it returns, so we can safely create a new reference.
            let entry = unsafe {
                // Create a new mutable reference from the raw pointer we got before prefetch()
                // This is safe because prefetch() has already returned, releasing its borrow
                // We're not actually creating overlapping borrows - the borrow from prefetch is released
                let mem_reborrowed: &'c mut BxMemC<'c> = &mut *mem_ptr;
                self.serve_icache_miss(
                    eip_biased,
                    p_addr,
                    mem_reborrowed,
                    cpus,
                    &mut dummy_stamp_table,
                )?
            };
            return Ok(entry);
        }

        // Return cached entry
        Ok(entry_option.unwrap())
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

        if cf {
            self.eflags |= 1 << 0;
        }
        if parity {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
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

        if cf {
            self.eflags |= 1 << 0;
        }
        if parity {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
    }

    fn execute_instruction(&mut self, instr: &mut BxInstructionGenerated) -> Result<()> {
        use crate::cpu::arith;
        use crate::cpu::data_xfer;
        use crate::cpu::decoder::Opcode;

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
            Opcode::MovGbEb => {
                if instr.mod_c0() {
                    // Register form
                    self.mov_gb_eb_r(instr);
                } else {
                    // Memory form
                    self.mov_gb_eb_m(instr);
                }
                Ok(())
            }
            Opcode::MovEbGb => {
                self.mov_eb_gb_r(instr);
                Ok(())
            }
            Opcode::MovEbIb => {
                self.mov_rb_ib(instr);
                Ok(())
            }

            // =========================================================================
            // 8-bit Arithmetic instructions (ADD, SUB, etc.)
            // =========================================================================
            Opcode::AddEbGb => {
                use crate::cpu::arith;
                arith::ADD_EbGb(self, instr)
            }
            Opcode::AddGbEb => {
                use crate::cpu::arith;
                arith::ADD_GbEb(self, instr)
            }
            Opcode::AdcEbGb => {
                use crate::cpu::arith;
                arith::ADC_EbGb(self, instr)
            }
            Opcode::AdcGwEw => {
                use crate::cpu::arith;
                arith::ADC_GwEw(self, instr)
            }
            Opcode::SubEbGb => {
                use crate::cpu::arith;
                arith::SUB_EbGb(self, instr)
            }
            Opcode::SubGbEb => {
                use crate::cpu::arith;
                arith::SUB_GbEb(self, instr)
            }
            Opcode::AndEbGb => {
                // Memory form - register form is handled separately
                self.and_eb_gb_m(instr);
                Ok(())
            }
            Opcode::AndGbEb => {
                if instr.mod_c0() {
                    // Register form
                    self.and_gb_eb_r(instr);
                } else {
                    // Memory form
                    self.and_gb_eb_m(instr);
                }
                Ok(())
            }
            Opcode::AndEbIb => {
                // Memory form
                self.and_eb_ib_m(instr);
                Ok(())
            }
            Opcode::OrEbGb => {
                // Memory form
                self.or_eb_gb_m(instr);
                Ok(())
            }
            Opcode::OrGbEb => {
                if instr.mod_c0() {
                    // Register form
                    self.or_gb_eb_r(instr);
                } else {
                    // Memory form
                    self.or_gb_eb_m(instr);
                }
                Ok(())
            }
            Opcode::OrEbIb => {
                // Memory form
                self.or_eb_ib_m(instr);
                Ok(())
            }
            Opcode::XorEbGb => {
                // Memory form
                self.xor_eb_gb_m(instr);
                Ok(())
            }
            Opcode::XorGbEb => {
                if instr.mod_c0() {
                    // Register form
                    self.xor_gb_eb_r(instr);
                } else {
                    // Memory form
                    self.xor_gb_eb_m(instr);
                }
                Ok(())
            }
            Opcode::XorEbIb => {
                // Memory form
                self.xor_eb_ib_m(instr);
                Ok(())
            }
            Opcode::NotEb => {
                // Memory form
                self.not_eb_m(instr);
                Ok(())
            }
            Opcode::TestEbGb => {
                // Memory form
                self.test_eb_gb_m(instr);
                Ok(())
            }
            Opcode::TestEbIb => {
                // Memory form
                self.test_eb_ib_m(instr);
                Ok(())
            }

            // =========================================================================
            // Data transfer (MOV) instructions - 16-bit
            // =========================================================================
            Opcode::MovGwEw => {
                // Debug MOV DX, [BP+4] at 0x50B
                if self.prev_rip >= 0x50b && self.prev_rip <= 0x50d {
                    tracing::warn!("MOV DX,[BP+4] at {:#x}: mod_c0={}", self.prev_rip, instr.mod_c0());
                }

                if instr.mod_c0() {
                    // Register form
                    self.mov_gw_ew_r(instr);
                } else {
                    // Memory form
                    self.mov_gw_ew_m(instr);
                }
                Ok(())
            }
            Opcode::MovEwGw => {
                self.mov_ew_gw_r(instr);
                Ok(())
            }
            Opcode::MovEwIw => {
                self.mov_rw_iw(instr);
                Ok(())
            }

            // =========================================================================
            // Segment register MOV
            // =========================================================================
            Opcode::MovEwSw => {
                self.mov_ew_sw(instr);
                Ok(())
            }
            Opcode::MovSwEw => {
                self.mov_sw_ew(instr)?;
                Ok(())
            }

            // =========================================================================
            // MOV with direct memory offset
            // =========================================================================
            Opcode::MovAlod => data_xfer::MOV_ALOd(self, instr),
            Opcode::MovAxod => data_xfer::MOV_AXOd(self, instr),
            Opcode::MovOdAl => data_xfer::MOV_OdAL(self, instr),
            Opcode::MovOdAx => data_xfer::MOV_OdAX(self, instr),
            Opcode::MovEaxod => data_xfer::MOV_EAXOd(self, instr),
            Opcode::MovOdEax => data_xfer::MOV_OdEAX(self, instr),

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
            Opcode::PushIw => {
                self.push_iw(instr);
                Ok(())
            }
            Opcode::PushSIb16 => {
                self.push_sib16(instr);
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
            Opcode::AddAxiw => {
                use crate::cpu::arith;
                arith::ADD_Axiw(self, instr)
            }
            Opcode::AddAlib => {
                use crate::cpu::arith;
                arith::ADD_EbIb(self, instr)
            }
            Opcode::AddEwsIb => {
                use crate::cpu::arith;
                arith::ADD_EwIbR(self, instr)
            }
            Opcode::AddEwIw => {
                use crate::cpu::arith;
                arith::ADD_EwIw(self, instr)
            }
            Opcode::AddEwGw => {
                use crate::cpu::arith;
                arith::ADD_EwGw(self, instr)
            }
            Opcode::AddEdsIb => {
                arith::ADD_EdId_R(self, instr);
                Ok(())
            }
            Opcode::AddEdId => {
                arith::ADD_EdId_R(self, instr);
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
            Opcode::SubAlib => {
                use crate::cpu::arith;
                arith::SUB_AL_Ib(self, instr)
            }
            Opcode::SubEdsIb => {
                eprintln!("★★★ DISPATCH SubEdsIb with ib={:#x}, meta_data[0]={}", instr.ib(), instr.meta_data[0]);
                arith::SUB_EdId_R(self, instr);
                Ok(())
            }
            Opcode::SubEdId => {
                eprintln!("★★★ DISPATCH SubEdId with id={:#x}, meta_data[0]={}", instr.id(), instr.meta_data[0]);
                arith::SUB_EdId_R(self, instr);
                Ok(())
            }
            // XOR instructions
            Opcode::XorEdGd => {
                // Memory form
                self.xor_ed_gd_m(instr);
                Ok(())
            }
            Opcode::XorEdGdZeroIdiom | Opcode::XorGdEdZeroIdiom => {
                self.zero_idiom_gd_r(instr);
                Ok(())
            }
            Opcode::XorGdEd => {
                if instr.mod_c0() {
                    // Register form
                    self.xor_gd_ed_r(instr);
                } else {
                    // Memory form
                    self.xor_gd_ed_m(instr);
                }
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
            Opcode::XorEwGw => {
                // Memory form
                self.xor_ew_gw_m(instr);
                Ok(())
            }
            Opcode::XorGwEw => {
                if instr.mod_c0() {
                    // Register form
                    self.xor_gw_ew_r(instr);
                } else {
                    // Memory form
                    self.xor_gw_ew_m(instr);
                }
                Ok(())
            }
            Opcode::XorEwGwZeroIdiom | Opcode::XorGwEwZeroIdiom => {
                self.zero_idiom_gw_r(instr);
                Ok(())
            }
            Opcode::XorAlib => {
                // XOR AL, imm8 (XOR r/m8, imm8 register form)
                self.xor_eb_ib_r(instr);
                Ok(())
            }
            // FAR JMP - Jump to absolute address with segment change
            Opcode::JmpfAp => {
                // For JMP FAR Ap, offset size depends on operand size (os32)
                // In 16-bit mode: offset is Iw (16-bit), in 32-bit mode: offset is Id (32-bit)
                // Segment selector is always Iw2 (16-bit)
                let segment = instr.iw2();

                tracing::error!("🚀 JmpfAp HANDLER: os32_l={}, ilen={}, Id={:#x}, Iw={:#x}, Iw2={:#x}, EIP={:#x}",
                    instr.os32_l(), instr.ilen(), instr.id(), instr.iw(), instr.iw2(), self.eip());

                if instr.os32_l() != 0 {
                    // 32-bit operand size: offset is Id (32-bit)
                    let offset32 = instr.id();
                    tracing::error!("🚀 FAR JMP 32-BIT to {:04x}:{:08x}", segment, offset32);
                    self.jmp_far32(instr, segment, offset32)?;
                } else {
                    // 16-bit operand size: offset is Iw (16-bit)
                    let offset16 = instr.iw();
                    tracing::error!("🚀 FAR JMP 16-BIT to {:04x}:{:04x}", segment, offset16);
                    self.jmp_far16(instr, segment, offset16)?;
                }
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
            Opcode::LidtMs => {
                self.lidt_ms(instr)?;
                Ok(())
            }
            Opcode::LgdtMs => {
                self.lgdt_ms(instr)?;
                Ok(())
            }

            // =========================================================================
            // Control Register Read Operations (MOV r32, CRx)
            // =========================================================================
            Opcode::MovRdCr0 => {
                self.mov_rd_cr0(instr)?;
                Ok(())
            }
            Opcode::MovRdCr2 => {
                self.mov_rd_cr2(instr)?;
                Ok(())
            }
            Opcode::MovRdCr3 => {
                self.mov_rd_cr3(instr)?;
                Ok(())
            }
            Opcode::MovRdCr4 => {
                self.mov_rd_cr4(instr)?;
                Ok(())
            }

            // =========================================================================
            // Control Register Write Operations (MOV CRx, r32)
            // =========================================================================
            Opcode::MovCr0rd => {
                self.mov_cr0_rd(instr)?;
                Ok(())
            }
            Opcode::MovCr2rd => {
                self.mov_cr2_rd(instr)?;
                Ok(())
            }
            Opcode::MovCr3rd => {
                self.mov_cr3_rd(instr)?;
                Ok(())
            }
            Opcode::MovCr4rd => {
                self.mov_cr4_rd(instr)?;
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
            Opcode::JoJbw => {
                self.jo_jb(instr);
                Ok(())
            }
            Opcode::JnoJbw => {
                self.jno_jb(instr);
                Ok(())
            }
            Opcode::JbJbw => {
                self.jb_jb(instr);
                Ok(())
            }
            Opcode::JnbJbw => {
                self.jnb_jb(instr);
                Ok(())
            }
            Opcode::JzJbw => {
                self.jz_jb(instr);
                Ok(())
            }
            Opcode::JnzJbw => {
                self.jnz_jb(instr);
                Ok(())
            }
            Opcode::JbeJbw => {
                self.jbe_jb(instr);
                Ok(())
            }
            Opcode::JnbeJbw => {
                self.jnbe_jb(instr);
                Ok(())
            }
            Opcode::JsJbw => {
                self.js_jb(instr);
                Ok(())
            }
            Opcode::JnsJbw => {
                self.jns_jb(instr);
                Ok(())
            }
            Opcode::JpJbw => {
                self.jp_jb(instr);
                Ok(())
            }
            Opcode::JnpJbw => {
                self.jnp_jb(instr);
                Ok(())
            }
            Opcode::JlJbw => {
                self.jl_jb(instr);
                Ok(())
            }
            Opcode::JnlJbw => {
                self.jnl_jb(instr);
                Ok(())
            }
            Opcode::JleJbw => {
                self.jle_jb(instr);
                Ok(())
            }
            Opcode::JnleJbw => {
                self.jnle_jb(instr);
                Ok(())
            }

            // Conditional jumps (16-bit displacement)
            Opcode::JzJw => {
                self.jz_jw(instr);
                Ok(())
            }
            Opcode::JnzJw => {
                self.jnz_jw(instr);
                Ok(())
            }

            // JMP instructions
            Opcode::JmpJbw => {
                self.jmp_jb(instr);
                Ok(())
            }
            Opcode::JmpJw => {
                self.jmp_jw(instr);
                Ok(())
            }
            Opcode::JmpJd => {
                self.jmp_jd(instr)?;
                Ok(())
            }
            Opcode::JmpJbd => {
                // JMP rel8 in 32-bit mode - uses same implementation as JmpJd
                // Decoder sign-extends byte displacement to dword in id()
                self.jmp_jd(instr)?;
                Ok(())
            }
            Opcode::JmpEw => {
                self.jmp_ew_r(instr);
                Ok(())
            }
            Opcode::JmpEd => {
                self.jmp_ed_r(instr)?;
                Ok(())
            }

            // CALL instructions
            Opcode::CallJw => {
                self.call_jw(instr);
                Ok(())
            }
            Opcode::CallJd => {
                self.call_jd(instr)?;
                Ok(())
            }
            Opcode::CallEw => {
                self.call_ew_r(instr);
                Ok(())
            }
            Opcode::CallEd => {
                self.call_ed_r(instr)?;
                Ok(())
            }

            // RET instructions
            Opcode::RetOp16 => {
                self.ret_near16(instr);
                Ok(())
            }
            Opcode::RetOp16Iw => {
                self.ret_near16_iw(instr);
                Ok(())
            }
            Opcode::RetOp32 => {
                self.ret_near32(instr)?;
                Ok(())
            }
            Opcode::RetOp32Iw => {
                self.ret_near32_iw(instr)?;
                Ok(())
            }

            // LOOP instructions
            Opcode::LoopJbw => {
                self.loop16_jb(instr);
                Ok(())
            }
            Opcode::LoopeJbw => {
                self.loope16_jb(instr);
                Ok(())
            }
            Opcode::LoopneJbw => {
                self.loopne16_jb(instr);
                Ok(())
            }
            Opcode::JcxzJbw => {
                self.jcxz_jb(instr);
                Ok(())
            }
            Opcode::JecxzJbd => {
                self.jecxz_jb(instr);
                Ok(())
            }

            // =========================================================================
            // Far CALL instructions (32-bit)
            // =========================================================================
            Opcode::CallfOp32Ap => self.call32_ap(instr),
            Opcode::CallfOp32Ep => self.call32_ep(instr),

            // =========================================================================
            // Far JMP instructions (32-bit)
            // =========================================================================
            Opcode::JmpfOp32Ep => self.jmp32_ep(instr),

            // =========================================================================
            // Far RET instructions (32-bit)
            // =========================================================================
            Opcode::RetfOp32 => self.retfar32(instr),
            Opcode::RetfOp32Iw => self.retfar32_iw(instr),

            // =========================================================================
            // Conditional jumps with 32-bit displacement (Jd variants)
            // =========================================================================
            Opcode::JoJd => {
                self.jo_jd(instr)?;
                Ok(())
            }
            Opcode::JnoJd => {
                self.jno_jd(instr)?;
                Ok(())
            }
            Opcode::JbJd => {
                self.jb_jd(instr)?;
                Ok(())
            }
            Opcode::JnbJd => {
                self.jnb_jd(instr)?;
                Ok(())
            }
            Opcode::JzJd => {
                self.jz_jd(instr)?;
                Ok(())
            }
            Opcode::JzJbd => {
                self.jz_jd(instr)?;
                Ok(())
            }
            Opcode::JnzJd => {
                self.jnz_jd(instr)?;
                Ok(())
            }
            Opcode::JnzJbd => {
                self.jnz_jd(instr)?;
                Ok(())
            }
            Opcode::JbeJd => {
                self.jbe_jd(instr)?;
                Ok(())
            }
            Opcode::JnbeJd => {
                self.jnbe_jd(instr)?;
                Ok(())
            }
            Opcode::JsJd => {
                self.js_jd(instr)?;
                Ok(())
            }
            Opcode::JnsJd => {
                self.jns_jd(instr)?;
                Ok(())
            }
            Opcode::JpJd => {
                self.jp_jd(instr)?;
                Ok(())
            }
            Opcode::JnpJd => {
                self.jnp_jd(instr)?;
                Ok(())
            }
            Opcode::JlJd => {
                self.jl_jd(instr)?;
                Ok(())
            }
            Opcode::JnlJd => {
                self.jnl_jd(instr)?;
                Ok(())
            }
            Opcode::JleJd => {
                self.jle_jd(instr)?;
                Ok(())
            }
            Opcode::JnleJd => {
                self.jnle_jd(instr)?;
                Ok(())
            }

            // Note: LOOP instructions in 32-bit mode use the same opcodes as 16-bit mode
            // (LoopJbw, LoopeJbw, LoopneJbw) but behavior is determined by operand size
            // The existing loop16_jb, loope16_jb, loopne16_jb functions already handle 32-bit mode

            // =========================================================================
            // Far CALL instructions (16-bit)
            // =========================================================================
            Opcode::CallfOp16Ap => self.call16_ap(instr),
            Opcode::CallfOp16Ep => self.call16_ep(instr),

            // =========================================================================
            // Far JMP instructions (16-bit)
            // =========================================================================
            Opcode::JmpfOp16Ep => self.jmp16_ep(instr),
            // JmpfAp is already implemented above

            // =========================================================================
            // Far RET instructions (16-bit)
            // =========================================================================
            Opcode::RetfOp16 => self.retfar16(instr),
            Opcode::RetfOp16Iw => self.retfar16_iw(instr),

            // =========================================================================
            // Conditional jumps with 16-bit displacement (Jw variants)
            // =========================================================================
            Opcode::JoJw => {
                self.jo_jw(instr);
                Ok(())
            }
            Opcode::JnoJw => {
                self.jno_jw(instr);
                Ok(())
            }
            Opcode::JbJw => {
                self.jb_jw(instr);
                Ok(())
            }
            Opcode::JnbJw => {
                self.jnb_jw(instr);
                Ok(())
            }
            Opcode::JbeJw => {
                self.jbe_jw(instr);
                Ok(())
            }
            Opcode::JnbeJw => {
                self.jnbe_jw(instr);
                Ok(())
            }
            Opcode::JsJw => {
                self.js_jw(instr);
                Ok(())
            }
            Opcode::JnsJw => {
                self.jns_jw(instr);
                Ok(())
            }
            Opcode::JpJw => {
                self.jp_jw(instr);
                Ok(())
            }
            Opcode::JnpJw => {
                self.jnp_jw(instr);
                Ok(())
            }
            Opcode::JlJw => {
                self.jl_jw(instr);
                Ok(())
            }
            Opcode::JnlJw => {
                self.jnl_jw(instr);
                Ok(())
            }
            Opcode::JleJw => {
                self.jle_jw(instr);
                Ok(())
            }
            Opcode::JnleJw => {
                self.jnle_jw(instr);
                Ok(())
            }

            // =========================================================================
            // CMP instructions
            // =========================================================================
            Opcode::CmpGbEb => {
                self.cmp_gb_eb_r(instr);
                Ok(())
            }
            Opcode::CmpGwEw => {
                self.cmp_gw_ew_r(instr);
                Ok(())
            }
            Opcode::CmpGdEd => {
                self.cmp_gd_ed_r(instr);
                Ok(())
            }
            Opcode::CmpEwGw => {
                use crate::cpu::arith;
                arith::CMP_EwGw(self, instr)
            }
            Opcode::CmpAlib => {
                self.cmp_al_ib(instr);
                Ok(())
            }
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
            Opcode::CmpAxiw => {
                self.cmp_ax_iw(instr);
                Ok(())
            }
            Opcode::CmpEaxid => {
                self.cmp_eax_id(instr);
                Ok(())
            }
            Opcode::CmpEwIw => {
                self.cmp_ew_iw_r(instr);
                Ok(())
            }
            Opcode::CmpEdId => {
                self.cmp_ed_id_r(instr);
                Ok(())
            }
            Opcode::CmpEdsIb => {
                self.cmp_ed_id_r(instr);
                Ok(())
            }
            Opcode::CmpEdGd => {
                // CMP r/m32, r32 (register form) - Based on Bochs arith32.cc:274-285
                arith::arith32::CMP_EdGd(self, instr);
                Ok(())
            }

            // TEST instructions
            Opcode::TestEbGb => {
                // Memory form
                self.test_eb_gb_m(instr);
                Ok(())
            }
            Opcode::TestEwGw => {
                // Memory form
                self.test_ew_gw_m(instr);
                Ok(())
            }
            Opcode::TestEdGd => {
                // Memory form
                self.test_ed_gd_m(instr);
                Ok(())
            }
            Opcode::TestAlib => {
                self.test_al_ib(instr);
                Ok(())
            }
            Opcode::TestAxiw => {
                self.test_ax_iw(instr);
                Ok(())
            }
            Opcode::TestEaxid => {
                self.test_eax_id(instr);
                Ok(())
            }
            Opcode::TestEwIw => {
                if instr.mod_c0() {
                    // Register form
                    self.test_ew_iw_r(instr);
                } else {
                    // Memory form
                    self.test_ew_iw_m(instr);
                }
                Ok(())
            }
            Opcode::TestEdId => {
                if instr.mod_c0() {
                    // Register form
                    self.test_ed_id_r(instr);
                } else {
                    // Memory form
                    self.test_ed_id_m(instr);
                }
                Ok(())
            }

            // =========================================================================
            // AND/OR/NOT instructions
            // =========================================================================
            Opcode::AndGwEw => {
                if instr.mod_c0() {
                    // Register form
                    self.and_gw_ew_r(instr);
                } else {
                    // Memory form
                    self.and_gw_ew_m(instr);
                }
                Ok(())
            }
            Opcode::AndEwGw => {
                // Memory form
                self.and_ew_gw_m(instr);
                Ok(())
            }
            Opcode::AndGdEd => {
                if instr.mod_c0() {
                    // Register form
                    self.and_gd_ed_r(instr);
                } else {
                    // Memory form
                    self.and_gd_ed_m(instr);
                }
                Ok(())
            }
            Opcode::AndEdGd => {
                // Memory form
                self.and_ed_gd_m(instr);
                Ok(())
            }
            Opcode::AndAlib => {
                self.and_al_ib(instr);
                Ok(())
            }
            Opcode::AndAxiw => {
                self.and_ax_iw(instr);
                Ok(())
            }
            Opcode::AndEaxid => {
                self.and_eax_id(instr);
                Ok(())
            }
            Opcode::AndEwIw => {
                if instr.mod_c0() {
                    // Register form
                    self.and_ew_iw_r(instr);
                } else {
                    // Memory form
                    self.and_ew_iw_m(instr);
                }
                Ok(())
            }
            Opcode::AndEdId => {
                if instr.mod_c0() {
                    // Register form
                    self.and_ed_id_r(instr);
                } else {
                    // Memory form
                    self.and_ed_id_m(instr);
                }
                Ok(())
            }
            Opcode::AndEdsIb => {
                if instr.mod_c0() {
                    // Register form
                    self.and_ed_id_r(instr);
                } else {
                    // Memory form
                    self.and_ed_id_m(instr);
                }
                Ok(())
            }

            Opcode::OrGwEw => {
                if instr.mod_c0() {
                    // Register form
                    self.or_gw_ew_r(instr);
                } else {
                    // Memory form
                    self.or_gw_ew_m(instr);
                }
                Ok(())
            }
            Opcode::OrEwGw => {
                // Memory form
                self.or_ew_gw_m(instr);
                Ok(())
            }
            Opcode::OrGdEd => {
                if instr.mod_c0() {
                    // Register form
                    self.or_gd_ed_r(instr);
                } else {
                    // Memory form
                    self.or_gd_ed_m(instr);
                }
                Ok(())
            }
            Opcode::OrEdGd => {
                // Memory form
                self.or_ed_gd_m(instr);
                Ok(())
            }
            Opcode::OrAlib => {
                self.or_al_ib(instr);
                Ok(())
            }
            Opcode::OrAxiw => {
                self.or_ax_iw(instr);
                Ok(())
            }
            Opcode::OrEaxid => {
                self.or_eax_id(instr);
                Ok(())
            }
            Opcode::OrEwIw => {
                if instr.mod_c0() {
                    // Register form
                    self.or_ew_iw_r(instr);
                } else {
                    // Memory form
                    self.or_ew_iw_m(instr);
                }
                Ok(())
            }
            Opcode::OrEdId => {
                if instr.mod_c0() {
                    // Register form
                    self.or_ed_id_r(instr);
                } else {
                    // Memory form
                    self.or_ed_id_m(instr);
                }
                Ok(())
            }
            Opcode::XorEwIw => {
                if instr.mod_c0() {
                    // Register form
                    self.xor_ew_iw_r(instr);
                } else {
                    // Memory form
                    self.xor_ew_iw_m(instr);
                }
                Ok(())
            }
            Opcode::XorEdId => {
                if instr.mod_c0() {
                    // Register form
                    self.xor_ed_id_r(instr);
                } else {
                    // Memory form
                    self.xor_ed_id_m(instr);
                }
                Ok(())
            }
            Opcode::NotEw => {
                if instr.mod_c0() {
                    // Register form
                    self.not_ew_r(instr);
                } else {
                    // Memory form
                    self.not_ew_m(instr);
                }
                Ok(())
            }
            Opcode::NotEd => {
                if instr.mod_c0() {
                    // Register form
                    self.not_ed_r(instr);
                } else {
                    // Memory form
                    self.not_ed_m(instr);
                }
                Ok(())
            }

            // =========================================================================
            // Multiplication and Division instructions
            // =========================================================================
            Opcode::MulAleb => {
                if instr.mod_c0() {
                    // Register form
                    self.mul_al_eb_r(instr)?;
                } else {
                    // Memory form
                    self.mul_al_eb_m(instr)?;
                }
                Ok(())
            }
            Opcode::ImulAleb => {
                if instr.mod_c0() {
                    // Register form
                    self.imul_al_eb_r(instr)?;
                } else {
                    // Memory form
                    self.imul_al_eb_m(instr)?;
                }
                Ok(())
            }
            Opcode::DivAleb => {
                if instr.mod_c0() {
                    // Register form
                    self.div_al_eb_r(instr)?;
                } else {
                    // Memory form
                    self.div_al_eb_m(instr)?;
                }
                Ok(())
            }
            Opcode::IdivAleb => {
                if instr.mod_c0() {
                    // Register form
                    self.idiv_al_eb_r(instr)?;
                } else {
                    // Memory form
                    self.idiv_al_eb_m(instr)?;
                }
                Ok(())
            }
            Opcode::MulAxew => {
                if instr.mod_c0() {
                    // Register form
                    self.mul_ax_ew_r(instr)?;
                } else {
                    // Memory form
                    self.mul_ax_ew_m(instr)?;
                }
                Ok(())
            }
            Opcode::ImulAxew => {
                if instr.mod_c0() {
                    // Register form
                    self.imul_ax_ew_r(instr)?;
                } else {
                    // Memory form
                    self.imul_ax_ew_m(instr)?;
                }
                Ok(())
            }
            Opcode::DivAxew => {
                if instr.mod_c0() {
                    // Register form
                    self.div_ax_ew_r(instr)?;
                } else {
                    // Memory form
                    self.div_ax_ew_m(instr)?;
                }
                Ok(())
            }
            Opcode::IdivAxew => {
                if instr.mod_c0() {
                    // Register form
                    self.idiv_ax_ew_r(instr)?;
                } else {
                    // Memory form
                    self.idiv_ax_ew_m(instr)?;
                }
                Ok(())
            }
            Opcode::MulEaxed => {
                if instr.mod_c0() {
                    // Register form
                    self.mul_eax_ed_r(instr)?;
                } else {
                    // Memory form
                    self.mul_eax_ed_m(instr)?;
                }
                Ok(())
            }
            Opcode::ImulEaxed => {
                if instr.mod_c0() {
                    // Register form
                    self.imul_eax_ed_r(instr)?;
                } else {
                    // Memory form
                    self.imul_eax_ed_m(instr)?;
                }
                Ok(())
            }
            Opcode::ImulGdEdsIb => {
                self.imul_gd_ed_ib(instr)?;
                Ok(())
            }
            Opcode::DivEaxed => {
                if instr.mod_c0() {
                    // Register form
                    self.div_eax_ed_r(instr)?;
                } else {
                    // Memory form
                    self.div_eax_ed_m(instr)?;
                }
                Ok(())
            }
            Opcode::IdivEaxed => {
                if instr.mod_c0() {
                    // Register form
                    self.idiv_eax_ed_r(instr)?;
                } else {
                    // Memory form
                    self.idiv_eax_ed_m(instr)?;
                }
                Ok(())
            }

            // =========================================================================
            // INC/DEC instructions
            // =========================================================================
            Opcode::IncEb => arith::INC_Eb(self, instr),
            Opcode::DecEb => arith::DEC_Eb(self, instr),
            Opcode::IncEw => {
                self.inc_ew_r(instr);
                Ok(())
            }
            Opcode::IncEd => {
                self.inc_ed_r(instr);
                Ok(())
            }
            Opcode::DecEw => {
                self.dec_ew_r(instr);
                Ok(())
            }
            Opcode::DecEd => {
                self.dec_ed_r(instr);
                Ok(())
            }

            // =========================================================================
            // PUSH/POP instructions
            // =========================================================================
            Opcode::PushEw => {
                self.push_ew_r(instr);
                Ok(())
            }
            Opcode::PushEd => {
                self.push_ed_r(instr);
                Ok(())
            }
            Opcode::PushId => {
                self.push_id(instr);
                Ok(())
            }
            Opcode::PushSIb32 => {
                self.push_id(instr);
                Ok(())
            }
            Opcode::PopEw => {
                self.pop_ew_r(instr);
                Ok(())
            }
            Opcode::PopEd => {
                self.pop_ed_r(instr);
                Ok(())
            }
            Opcode::PopOp32Sw => {
                // POP segment register (32-bit mode) - Based on Bochs stack32.cc:87-111
                self.pop32_sw(instr)?;
                Ok(())
            }
            Opcode::PushaOp16 => {
                self.pusha16(instr);
                Ok(())
            }
            Opcode::PushaOp32 => {
                self.pusha32(instr);
                Ok(())
            }
            Opcode::PopaOp16 => {
                self.popa16(instr);
                Ok(())
            }
            Opcode::PopaOp32 => {
                self.popa32(instr);
                Ok(())
            }
            Opcode::PushfFw => {
                self.pushf_fw(instr);
                Ok(())
            }
            Opcode::PopfFw => {
                self.popf_fw(instr);
                Ok(())
            }
            Opcode::PushfFd => {
                self.pushf_fd(instr);
                Ok(())
            }
            Opcode::PopfFd => {
                self.popf_fd(instr);
                Ok(())
            }
            Opcode::LeaveOp16 => {
                self.leave16(instr);
                Ok(())
            }

            // =========================================================================
            // String instructions
            // =========================================================================
            Opcode::RepMovsbYbXb => {
                self.rep_movsb16(instr);
                Ok(())
            }
            Opcode::RepMovsdYdXd => {
                // REP MOVSD - Move dword from DS:SI/ESI to ES:DI/EDI with repeat
                // Based on BX_CPU_C::REP_MOVSD_YdXd in string.cc:71-88
                if instr.as32_l() != 0 {
                    // 32-bit address mode
                    self.rep_movsd32(instr);
                } else {
                    // 16-bit address mode
                    self.rep_movsd16(instr);
                }
                Ok(())
            }
            Opcode::RepStosbYbAl => {
                self.rep_stosb16(instr);
                Ok(())
            }
            Opcode::RepStoswYwAx => {
                self.rep_stosw16(instr);
                Ok(())
            }
            Opcode::RepStosdYdEax => {
                // REP STOSD - Store EAX at ES:DI/EDI with repeat
                // Based on BX_CPU_C::REP_STOSD_YdEAX in string.cc
                if instr.as32_l() != 0 {
                    // 32-bit address mode
                    self.rep_stosd32(instr);
                } else {
                    // 16-bit address mode
                    self.rep_stosd16(instr);
                }
                Ok(())
            }
            Opcode::RepLodsbAlxb => {
                self.rep_lodsb16(instr);
                Ok(())
            }

            // =========================================================================
            // Software interrupts
            // =========================================================================
            Opcode::IntIb => {
                self.int_ib(instr);
                Ok(())
            }
            Opcode::INT3 => {
                self.int3(instr);
                Ok(())
            }
            Opcode::IretOp16 => {
                self.iret16(instr);
                Ok(())
            }
            Opcode::IretOp32 => {
                self.iret32(instr);
                Ok(())
            }

            // =========================================================================
            // BOUND - Check Array Index Against Bounds
            // =========================================================================
            Opcode::BoundGwMa => {
                self.bound_gw_ma(instr);
                Ok(())
            }
            Opcode::BoundGdMa => {
                self.bound_gd_ma(instr);
                Ok(())
            }

            // =========================================================================
            // 64-bit control transfer instructions
            // =========================================================================
            Opcode::CallJq => self.call_jq(instr),
            Opcode::CallEq => self.call_eq_r(instr),
            Opcode::CallfOp64Ep => self.call64_ep(instr),
            Opcode::JmpJq => self.jmp_jq(instr),
            Opcode::JmpEq => self.jmp_eq_r(instr),
            Opcode::JmpfOp64Ep => self.jmp64_ep(instr),
            Opcode::RetOp64Iw => self.retnear64_iw(instr),
            Opcode::RetfOp64 => self.retfar64(instr),
            Opcode::RetfOp64Iw => self.retfar64_iw(instr),
            Opcode::IretOp64 => self.iret64(instr),
            Opcode::JrcxzJbq => {
                self.jrcxz_jb(instr);
                Ok(())
            }

            // =========================================================================
            // Conditional jumps with 64-bit displacement (Jq variants)
            // =========================================================================
            Opcode::JoJq => {
                self.jo_jq(instr);
                Ok(())
            }
            Opcode::JnoJq => {
                self.jno_jq(instr);
                Ok(())
            }
            Opcode::JbJq => {
                self.jb_jq(instr);
                Ok(())
            }
            Opcode::JnbJq => {
                self.jnb_jq(instr);
                Ok(())
            }
            Opcode::JzJq => {
                self.jz_jq(instr);
                Ok(())
            }
            Opcode::JnzJq => {
                self.jnz_jq(instr);
                Ok(())
            }
            Opcode::JbeJq => {
                self.jbe_jq(instr);
                Ok(())
            }
            Opcode::JnbeJq => {
                self.jnbe_jq(instr);
                Ok(())
            }
            Opcode::JsJq => {
                self.js_jq(instr);
                Ok(())
            }
            Opcode::JnsJq => {
                self.jns_jq(instr);
                Ok(())
            }
            Opcode::JpJq => {
                self.jp_jq(instr);
                Ok(())
            }
            Opcode::JnpJq => {
                self.jnp_jq(instr);
                Ok(())
            }
            Opcode::JlJq => {
                self.jl_jq(instr);
                Ok(())
            }
            Opcode::JnlJq => {
                self.jnl_jq(instr);
                Ok(())
            }
            Opcode::JleJq => {
                self.jle_jq(instr);
                Ok(())
            }
            Opcode::JnleJq => {
                self.jnle_jq(instr);
                Ok(())
            }

            Opcode::Hlt => {
                self.hlt(instr);
                Ok(())
            }
            Opcode::Cpuid => {
                self.cpuid(instr);
                Ok(())
            }

            // =========================================================================
            // Shift/Rotate instructions
            // =========================================================================
            Opcode::ShlEbI1 => {
                self.shl_eb_1(instr);
                Ok(())
            }
            Opcode::ShlEb => {
                self.shl_eb_cl(instr);
                Ok(())
            }
            Opcode::ShlEbIb => {
                self.shl_eb_ib(instr);
                Ok(())
            }
            Opcode::ShlEwI1 => {
                self.shl_ew_1(instr);
                Ok(())
            }
            Opcode::ShlEw => {
                self.shl_ew_cl(instr);
                Ok(())
            }
            Opcode::ShlEwIb => {
                self.shl_ew_ib(instr);
                Ok(())
            }
            Opcode::ShlEdI1 => {
                self.shl_ed_1(instr);
                Ok(())
            }
            Opcode::ShlEd => {
                self.shl_ed_cl(instr);
                Ok(())
            }
            Opcode::ShlEdIb => {
                self.shl_ed_ib(instr);
                Ok(())
            }
            Opcode::ShldEdGdIb => {
                self.shld_ed_gd_ib(instr);
                Ok(())
            }
            Opcode::ShldEdGd => {
                self.shld_ed_gd_cl(instr);
                Ok(())
            }
            Opcode::ShrdEdGdIb => {
                self.shrd_ed_gd_ib(instr);
                Ok(())
            }
            Opcode::ShrdEdGd => {
                self.shrd_ed_gd_cl(instr);
                Ok(())
            }
            Opcode::SarEbIb => {
                self.sar_eb_ib(instr);
                Ok(())
            }

            Opcode::ShrEbI1 => {
                self.shr_eb_1(instr);
                Ok(())
            }
            Opcode::ShrEb => {
                self.shr_eb_cl(instr);
                Ok(())
            }
            Opcode::ShrEbIb => {
                self.shr_eb_ib(instr);
                Ok(())
            }
            Opcode::ShrEwI1 => {
                self.shr_ew_1(instr);
                Ok(())
            }
            Opcode::ShrEw => {
                self.shr_ew_cl(instr);
                Ok(())
            }
            Opcode::ShrEwIb => {
                self.shr_ew_ib(instr);
                Ok(())
            }
            Opcode::ShrEdI1 => {
                self.shr_ed_1(instr);
                Ok(())
            }
            Opcode::ShrEd => {
                self.shr_ed_cl(instr);
                Ok(())
            }
            Opcode::ShrEdIb => {
                self.shr_ed_ib(instr);
                Ok(())
            }

            // ROL - Rotate Left
            Opcode::RolEbI1 => {
                self.rol_eb_1(instr);
                Ok(())
            }
            Opcode::RolEb => {
                self.rol_eb_cl(instr);
                Ok(())
            }
            Opcode::RolEbIb => {
                self.rol_eb_cl(instr);  // Uses same implementation
                Ok(())
            }
            Opcode::RolEwI1 => {
                self.rol_ew_1(instr);
                Ok(())
            }
            Opcode::RolEw => {
                self.rol_ew_cl(instr);
                Ok(())
            }

            // =========================================================================
            // Data transfer extensions
            // =========================================================================
            Opcode::LeaGwM => {
                self.lea_gw_m(instr);
                Ok(())
            }
            Opcode::LeaGdM => {
                self.lea_gd_m(instr);
                Ok(())
            }
            Opcode::XchgEwGw => {
                self.xchg_ew_gw(instr);
                Ok(())
            }
            Opcode::XchgEdGd => {
                self.xchg_ed_gd(instr);
                Ok(())
            }
            Opcode::Cbw => {
                self.cbw(instr);
                Ok(())
            }
            Opcode::MovsxGdEb => {
                self.movsx_gd_eb(instr);
                Ok(())
            }
            Opcode::MovzxGdEb => {
                data_xfer::MOVZX_GdEb(self, instr);
                Ok(())
            }
            Opcode::MovzxGdEw => {
                data_xfer::MOVZX_GdEw(self, instr);
                Ok(())
            }
            Opcode::Cwd => {
                self.cwd(instr);
                Ok(())
            }
            Opcode::Cwde => {
                self.cwde(instr);
                Ok(())
            }
            Opcode::Cdq => {
                self.cdq(instr);
                Ok(())
            }
            Opcode::Xlat => {
                self.xlat(instr);
                Ok(())
            }
            Opcode::Lahf => {
                self.lahf(instr);
                Ok(())
            }
            Opcode::Sahf => {
                self.sahf(instr);
                Ok(())
            }

            // =========================================================================
            // Data transfer (64-bit) instructions
            // =========================================================================
            Opcode::MovRrxiq => {
                self.mov_rrxiq(instr);
                Ok(())
            }
            Opcode::MovOp64GdEd => {
                self.mov64_gd_ed_m(instr);
                Ok(())
            }
            Opcode::MovOp64EdGd => {
                self.mov64_ed_gd_m(instr);
                Ok(())
            }
            Opcode::MovEqGq => {
                self.mov_eq_gq_m(instr);
                Ok(())
            }
            Opcode::MovGqEq => {
                self.mov_gq_eq_m(instr);
                Ok(())
            }
            Opcode::LeaGqM => {
                self.lea_gq_m(instr);
                Ok(())
            }
            Opcode::MovAloq => {
                self.mov_aloq(instr);
                Ok(())
            }
            Opcode::MovOqAl => {
                self.mov_oq_al(instr);
                Ok(())
            }
            Opcode::MovAxoq => {
                self.mov_ax_oq(instr);
                Ok(())
            }
            Opcode::MovOqAx => {
                self.mov_oq_ax(instr);
                Ok(())
            }
            Opcode::MovEaxoq => {
                self.mov_eax_oq(instr);
                Ok(())
            }
            Opcode::MovOqEax => {
                self.mov_oq_eax(instr);
                Ok(())
            }
            Opcode::MovRaxoq => {
                self.mov_rax_oq(instr);
                Ok(())
            }
            Opcode::MovOqRax => {
                self.mov_oq_rax(instr);
                Ok(())
            }
            Opcode::MovEqId => {
                self.mov_eq_id_r(instr);
                Ok(())
            }
            Opcode::MovzxGqEb => {
                self.movzx_gq_eb_r(instr);
                Ok(())
            }
            Opcode::MovzxGqEw => {
                self.movzx_gq_ew_r(instr);
                Ok(())
            }
            Opcode::MovsxGqEb => {
                self.movsx_gq_eb_r(instr);
                Ok(())
            }
            Opcode::MovsxGqEw => {
                self.movsx_gq_ew_r(instr);
                Ok(())
            }
            Opcode::MovsxdGqEd => {
                self.movsx_gq_ed_r(instr);
                Ok(())
            }
            Opcode::XchgEqGq => {
                self.xchg_eq_gq_r(instr);
                Ok(())
            }
            Opcode::CmovoGqEq => {
                self.cmovo_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnoGqEq => {
                self.cmovno_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovbGqEq => {
                self.cmovb_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnbGqEq => {
                self.cmovnb_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovzGqEq => {
                self.cmovz_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnzGqEq => {
                self.cmovnz_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovbeGqEq => {
                self.cmovbe_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnbeGqEq => {
                self.cmovnbe_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovsGqEq => {
                self.cmovs_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnsGqEq => {
                self.cmovns_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovpGqEq => {
                self.cmovp_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnpGqEq => {
                self.cmovnp_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovlGqEq => {
                self.cmovl_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnlGqEq => {
                self.cmovnl_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovleGqEq => {
                self.cmovle_gq_eq_r(instr);
                Ok(())
            }
            Opcode::CmovnleGqEq => {
                self.cmovnle_gq_eq_r(instr);
                Ok(())
            }

            // =========================================================================
            // BCD (Binary Coded Decimal) instructions
            // =========================================================================
            Opcode::Das => crate::cpu::bcd::DAS(self, instr),

            _ => {
                tracing::error!("Unimplemented opcode: {:?}", instr.get_ia_opcode());
                Err(crate::cpu::CpuError::UnimplementedOpcode {
                    opcode: format!("{:?}", instr.get_ia_opcode()),
                })
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

        if cf {
            self.eflags |= 1 << 0;
        }
        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
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

        if cf {
            self.eflags |= 1 << 0;
        }
        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
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

        if cf {
            self.eflags |= 1 << 0;
        }
        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
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

        if cf {
            self.eflags |= 1 << 0;
        }
        if pf {
            self.eflags |= 1 << 2;
        }
        if af {
            self.eflags |= 1 << 4;
        }
        if zf {
            self.eflags |= 1 << 6;
        }
        if sf {
            self.eflags |= 1 << 7;
        }
        if of {
            self.eflags |= 1 << 11;
        }
    }

    pub(super) fn update_flags_logic8(&mut self, result: u8) {
        self.eflags &= !((1 << 11) | (1 << 0)); // OF=0, CF=0
        if (result & 0x80) != 0 {
            self.eflags |= 1 << 7;
        } else {
            self.eflags &= !(1 << 7);
        }
        if result == 0 {
            self.eflags |= 1 << 6;
        } else {
            self.eflags &= !(1 << 6);
        }
        if (result.count_ones() % 2) == 0 {
            self.eflags |= 1 << 2;
        } else {
            self.eflags &= !(1 << 2);
        }
    }

    pub(super) fn update_flags_logic16(&mut self, result: u16) {
        self.eflags &= !((1 << 11) | (1 << 0)); // OF=0, CF=0
        if (result & 0x8000) != 0 {
            self.eflags |= 1 << 7;
        } else {
            self.eflags &= !(1 << 7);
        }
        if result == 0 {
            self.eflags |= 1 << 6;
        } else {
            self.eflags &= !(1 << 6);
        }
        if (((result & 0xFF) as u8).count_ones() % 2) == 0 {
            self.eflags |= 1 << 2;
        } else {
            self.eflags &= !(1 << 2);
        }
    }

    /// Get segment base address safely
    /// This is a safe wrapper around the unsafe union access
    pub(super) fn get_segment_base(&self, seg: super::decoder::BxSegregs) -> BxAddress {
        // Safe: We know seg is a valid BxSegregs enum value (0-5, 7)
        // and sregs array has 6 elements, so seg as usize is always in bounds
        unsafe { self.sregs[seg as usize].cache.u.segment.base }
    }

    /// Get segment limit safely
    /// This is a safe wrapper around the unsafe union access
    pub(super) fn get_segment_limit(&self, seg: super::decoder::BxSegregs) -> u32 {
        // Safe: We know seg is a valid BxSegregs enum value (0-5, 7)
        // and sregs array has 6 elements, so seg as usize is always in bounds
        unsafe { self.sregs[seg as usize].cache.u.segment.limit_scaled }
    }

    /// Get segment d_b flag safely
    /// This is a safe wrapper around the unsafe union access
    pub(super) fn get_segment_d_b(&self, seg: super::decoder::BxSegregs) -> bool {
        // Safe: We know seg is a valid BxSegregs enum value (0-5, 7)
        // and sregs array has 6 elements, so seg as usize is always in bounds
        unsafe { self.sregs[seg as usize].cache.u.segment.d_b }
    }

    /// Set segment base address safely
    /// This is a safe wrapper around the unsafe union access
    pub(super) fn set_segment_base(&mut self, seg: super::decoder::BxSegregs, base: BxAddress) {
        // Safe: We know seg is a valid BxSegregs enum value (0-5, 7)
        // and sregs array has 6 elements, so seg as usize is always in bounds
        unsafe {
            self.sregs[seg as usize].cache.u.segment.base = base;
        }
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
    pub(super) fn prefetch(&mut self, mem: &'c mut BxMemC<'c>, cpus: &[&Self]) -> Result<()> {
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

            // If EIP was masked, update it (matching C++ vm8086.cc:109: EIP = new_eip & 0xffff)
            if self.real_mode() && eip != eip_raw {
                self.set_eip(eip);
            }

            laddr = BxAddress::from(self.get_laddr32(BxSegregs::Cs as _, eip));
            let cs_base = unsafe { self.sregs[BxSegregs::Cs as usize].cache.u.segment.base };
            tracing::info!(
                "prefetch: CS.base={:#x}, EIP={:#x}, laddr={:#x}",
                cs_base,
                eip,
                laddr
            );
            page_offset = super::tlb::page_offset(laddr);

            // Calculate RIP at the beginning of the page.
            let eip_page_bias_calc = BxAddress::from(page_offset.wrapping_sub(eip));

            let limit: u32 = unsafe {
                self.sregs[BxSegregs::Cs as usize]
                    .cache
                    .u
                    .segment
                    .limit_scaled
            };
            if eip > limit {
                // Matching C++ cpu.cc:656-659 - raise exception (does not return normally)
                tracing::error!("prefetch: EIP [{eip:#x}] > CS.limit [{limit:#x}]",);
                // In C++, exception() uses setjmp/longjmp and doesn't return here
                // In Rust, exception() returns Ok(()), but control was transferred to handler
                self.eip_page_bias = 0; // Reset to prevent using stale value
                self.exception(Exception::Gp, 0)?;
                // After exception handler runs, check if the new EIP is valid
                // If not, we're in a loop (exception handler also has invalid EIP)
                let new_eip = self.eip();
                let new_limit: u32 = unsafe {
                    self.sregs[BxSegregs::Cs as usize]
                        .cache
                        .u
                        .segment
                        .limit_scaled
                };
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

        // Check TLB entry - extract values to avoid holding mutable borrow
        let (tlb_hit, tlb_ppf, tlb_host_addr) = {
            let tlb_entry = self.itlb.get_entry_of(laddr, 0);
            let hit = (tlb_entry.lpf == lpf)
                && (tlb_entry.access_bits & (1 << u32::from(self.user_pl))) != 0;
            (hit, tlb_entry.ppf, tlb_entry.host_page_addr)
        };

        let fetch_ptr_option = if tlb_hit {
            self.p_addr_fetch_page = tlb_ppf;
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
            let dummy_tlb_entry = unsafe { core::mem::zeroed::<TLBEntry>() };
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
                    tracing::info!(
                        "prefetch: translate_linear OK, p_addr={:#x}, p_addr_fetch_page={:#x}",
                        p_addr,
                        self.p_addr_fetch_page
                    );
                    None
                }
                Err(_) => {
                    // Page fault occurred, exception was raised
                    // Return None to indicate we need to handle the exception
                    None
                }
            }
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
                Ok(Some(fetch_ptr)) => self.eip_fetch_ptr = Some(fetch_ptr),
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

    // =========================================================================
    // Error handlers matching original C++ BxError, BxNoFPU, etc.
    // =========================================================================

    /// BxError - Invalid instruction handler
    /// Matches BX_CPU_C::BxError from proc_ctrl.cc:40
    /// Raises #UD (Undefined Instruction) exception
    pub(super) fn bx_error(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoFPU from proc_ctrl.cc:463
    /// Raises #NM (Device Not Available) if CR0.EM or CR0.TS is set
    pub(super) fn bx_no_fpu(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoMMX from proc_ctrl.cc:473
    /// Raises #UD if CR0.EM is set, #NM if CR0.TS is set
    pub(super) fn bx_no_mmx(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoSSE from proc_ctrl.cc:502
    /// Only available if CPU_LEVEL >= 6
    /// Raises #UD if CR0.EM is set or CR4.OSFXSR is clear, #NM if CR0.TS is set
    #[cfg(feature = "bx_support_sse")]
    pub(super) fn bx_no_sse(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoAVX from proc_ctrl.cc:557
    /// Only available if BX_SUPPORT_AVX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    #[cfg(feature = "bx_support_avx")]
    pub(super) fn bx_no_avx(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoOpMask from proc_ctrl.cc:575
    /// Only available if BX_SUPPORT_EVEX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    #[cfg(feature = "bx_support_evex")]
    pub(super) fn bx_no_opmask(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoEVEX from proc_ctrl.cc:591
    /// Only available if BX_SUPPORT_EVEX
    /// Raises #UD if not in protected mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    /// Raises #NM if CR0.TS is set
    #[cfg(feature = "bx_support_evex")]
    pub(super) fn bx_no_evex(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matches BX_CPU_C::BxNoAMX from proc_ctrl.cc:609
    /// Only available if BX_SUPPORT_AMX
    /// Raises #UD if not in long64 mode, CR4.OSXSAVE is clear, or XCR0 doesn't have required bits
    #[cfg(feature = "bx_support_amx")]
    pub(super) fn bx_no_amx(&mut self, instr: &BxInstructionGenerated) -> Result<()> {
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
    /// Matching C++ `BX_CPU_C::assignHandler` in fetchdecode32.cc:2041-2139
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
        instr: &mut BxInstructionGenerated,
        fetch_mode_mask: u32,
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
        let mut is_bx_error = false; // Track if BxError handler was assigned

        if let Some(entry) = &opcode_entry {
            // Handler assignment from table
            if !is_reg_form {
                // Memory form: use execute1 from table (matching line 2046)
                selected_handler = Some(entry.execute1);

                // Special case: MOV with SS segment override (matching lines 2049-2056)
                if ia_opcode == Opcode::MovOp32GdEd {
                    if instr.seg() == BxSegregs::Ss as u8 {
                        // Use MOV32S_GdEdM handler (matching C++ line 2051)
                        use super::opcodes_table::mov32s_gd_ed_m_wrapper;
                        selected_handler = Some(mov32s_gd_ed_m_wrapper);
                    }
                }
                if ia_opcode == Opcode::MovOp32EdGd {
                    if instr.seg() == BxSegregs::Ss as u8 {
                        // Use MOV32S_EdGdM handler (matching C++ line 2055)
                        use super::opcodes_table::mov32s_ed_gd_m_wrapper;
                        selected_handler = Some(mov32s_ed_gd_m_wrapper);
                    }
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
        let fetch_mode = FetchModeMask::from_bits_truncate(fetch_mode_mask);

        // Check FPU/MMX availability
        if !fetch_mode.contains(FetchModeMask::FETCH_MODE_FPU_MMX_OK) {
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
            if !fetch_mode.contains(FetchModeMask::FETCH_MODE_SSE_OK) {
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
            if !fetch_mode.contains(FetchModeMask::FETCH_MODE_AVX_OK) {
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
            if !fetch_mode.contains(FetchModeMask::FETCH_MODE_OPMASK_OK) {
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
            if !fetch_mode.contains(FetchModeMask::FETCH_MODE_EVEX_OK) {
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
            if !fetch_mode.contains(FetchModeMask::FETCH_MODE_AMX_OK) {
                if op_flags.contains(OpFlags::PREPARE_AMX) {
                    // Matching C++ line 2126: only assign if execute1 != BxError
                    if !is_bx_error {
                        use super::opcodes_table::bx_no_amx_wrapper;
                        selected_handler = Some(bx_no_amx_wrapper);
                    }
                    return Ok((true, selected_handler)); // Stop trace
                }
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
