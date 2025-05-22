use crate::config::{BxAddress, BxPhyAddress, BxPtrEquiv};

use super::{
    apic::BxLocalApic,
    cpuid::BxCpuIdTrait,
    cpustats::BxCpuStatistics,
    crregs::{BxCr0, BxCr4, BxDr6, BxDr7, Xcr0, MSR},
    decoder::{
        BxSegregs, BX_16BIT_REG_IP, BX_32BIT_REG_EIP, BX_64BIT_REG_RIP, BX_64BIT_REG_SSP,
        BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE, BX_TMP_REGISTER, BX_XMM_REGISTERS,
    },
    descriptor::{BxGlobalSegmentReg, BxSegmentReg},
    i387::{BxPackedRegister, I387},
    icache::BxIcache,
    lazy_flags::BxLazyflagsEntry,
    svm::VmcbCache,
    tlb::BxHostpageaddr,
    vmx::{VmcsCache, VmcsMapping, VmxCap},
    xmm::{BxMxcsr, BxZmmReg},
};

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

const BX_MSR_MAX_INDEX: usize = 0x1000;

#[derive(Debug)]
pub struct BxCpuC<'c, I: BxCpuIdTrait> {
    bx_cpuid: u32,

    cpuid: I,

    ia_extensions_bitmask: [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE],

    vmx_extensions_bitmask: u32,

    svm_extensions_bitmask: u32,

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
    eflags: u32, // Raw 32-bit value in x86 bit position.

    /// lazy arithmetic flags state
    oszapc: BxLazyflagsEntry,

    /// so that we can back up when handling faults, exceptions, etc.
    /// we need to store the value of the instruction pointer, before
    /// each fetch/execute cycle.
    prev_rip: BxAddress,
    prev_rsp: BxAddress,

    prev_ssp: BxAddress,
    speculative_rsp: bool,

    icount: u64,
    icount_last_sync: u64,

    /// What events to inhibit at any given time.  Certain instructions
    /// inhibit interrupts, some debug exceptions and single-step traps.
    inhibit_mask: u32,
    inhibit_icount: u64,

    /// user segment register set
    sregs: [BxSegmentReg; 6],

    // system segment registers
    /// global descriptor table register
    gdtr: BxGlobalSegmentReg,
    /// interrupt descriptor table register
    idtr: BxGlobalSegmentReg,
    /// local descriptor table register
    ldtr: BxSegmentReg,
    /// task register
    tr: BxSegmentReg,

    // debug registers DR0-DR7
    /// Dr0-DR3
    dr: [BxAddress; 5],
    dr6: BxDr6,
    dr7: BxDr7,

    /// holds DR6 value (16bit) to be set
    debug_trap: u32,

    // Control registers
    bx_cr0_t: BxCr0,
    cr2: BxAddress,
    cr3: BxAddress,

    cr4: BxCr4,
    cr4_suppmask: u32,

    linaddr_width: u32,
    efer_suppmask: u32,

    /// TSC: Time Stamp Counter
    /// Instead of storing a counter and incrementing it every instruction, we
    /// remember the time in ticks that it was reset to zero.  With a little
    /// algebra, we can also support setting it to something other than zero.
    /// Don't read this directly; use get_TSC and set_TSC to access the TSC.
    tsc_adjust: i64,

    tsc_offset: i64,

    xcr0: Xcr0,

    xcr0_suppmask: u32,
    ia32_xss_suppmask: u32,

    // protection keys
    #[cfg(feature = "bx_support_pkeys")]
    pkru: u32,
    #[cfg(feature = "bx_support_pkeys")]
    pkrs: u32,

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
    rd_pkey: [u32; 16],
    #[cfg(feature = "bx_support_pkeys")]
    wr_pkey: [u32; 16],

    uintr: Uintr,

    the_i387: I387,

    // Vector register set
    // vmm0-vmmN: up to 32 vector registers
    // vtmp: temp register
    vmm: [BxZmmReg; BX_XMM_REGISTERS],
    // Note, didnt check for other features. Basically only aligment changes
    mxcsr: BxMxcsr,
    mxcsr_mask: u32,

    opmask: [BxGenReg; 8],

    #[cfg(feature = "bx_support_monitor_mwait")]
    monitor: MonitorAddr,

    #[cfg(feature = "bx_support_apic")]
    lapic: BxLocalApic<'c, I>,

    /// SMM base register
    smbase: u32,

    msr: BxRegsMsr,

    #[cfg(feature = "bx_configure_msrs")]
    msrs: [MSR; BX_MSR_MAX_INDEX],

    #[cfg(feature = "bx_support_amx")]
    amx: AMX,

    in_vmx: bool,
    in_vmx_guest: bool,
    /// save in_vmx and in_vmx_guest flags when in SMM mode
    in_smm_vmx: bool,
    in_smm_vmx_guest: bool,
    vmcsptr: u64,

    #[cfg(feature = "bx_support_memtype")]
    vmcs_memtype: BxMemType,

    vmxonptr: u64,

    vmcs: VmcsCache,
    vmx_cap: VmxCap,
    vmcs_map: VmcsMapping,

    in_svm_guest: bool,
    /// global interrupt enable flag, when zero all external interrupt disabled
    svm_gif: bool,
    vmcbptr: BxPhyAddress,
    vmcbhostptr: BxHostpageaddr,
    #[cfg(feature = "bx_support_memtype")]
    vmcb_memtype: BxMemType,

    vmcb: VmcbCache,

    in_event: bool,

    nmi_unblocking_iret: bool,

    /// 1 if processing external interrupt or exception
    /// or if not related to current instruction,
    /// 0 if current CS:EIP caused exception */
    ext: bool,

    // Todo: Maybe enum?
    activity_state: u32,

    pending_event: u32,
    event_mask: u32,
    // keep 32-bit because of BX_ASYNC_EVENT_STOP_TRACE
    async_event: u32,

    in_smm: bool,
    cpu_mode: u32,
    user_pl: bool,

    ignore_bad_msrs: bool,

    cpu_state_use_ok: u32, // format of BX_FETCH_MODE_*

    // FIXME: skipped   static jmp_buf jmp_buf_env;
    last_exception_type: u32,

    #[cfg(feature = "bx_support_handlers_chaining_speedups")]
    cpuloop_stack_anchor: Option<&'c [u8]>,

    // Boundaries of current code page, based on EIP
    eip_page_bias: BxAddress,
    eip_page_window_size: u32,
    eip_fetch_ptr: &'c [u8],
    p_addr_fetch_page: BxPhyAddress, // Guest physical address of current instruction page

    // Boundaries of current stack page, based on ESP
    // Linear address of current stack page
    esp_page_bias: BxAddress,
    esp_page_window_size: u32,
    esp_host_ptr: &'c [u8],
    /// Guest physical address of current stack page
    p_addr_stack_page: BxPhyAddress,

    #[cfg(feature = "bx_support_memtype")]
    espPageMemtype: BxMemType,

    #[cfg(not(feature = "bx_support_smp"))]
    esp_page_fine_granularity_mapping: u32,

    #[cfg(feature = "bx_support_alignment_check")]
    alignment_check_mask: u32,

    stats: BxCpuStatistics,

    #[cfg(feature = "bx_debugger")]
    watchpoint: BxPhyAddress,
    #[cfg(feature = "bx_debugger")]
    break_point: u8,
    #[cfg(feature = "bx_debugger")]
    magic_break: u8,
    #[cfg(feature = "bx_debugger")]
    stop_reason: u8,
    #[cfg(feature = "bx_debugger")]
    trace: bool,
    #[cfg(feature = "bx_debugger")]
    trace_reg: bool,
    #[cfg(feature = "bx_debugger")]
    trace_mem: bool,
    #[cfg(feature = "bx_debugger")]
    mode_break: bool,

    #[cfg(feature = "bx_debugger")]
    vmexit_break: bool,

    #[cfg(feature = "bx_debugger")]
    show_flag: u32,
    #[cfg(feature = "bx_debugger")]
    guard_found: BxGuardFound,

    #[cfg(feature = "bx_instrumentation")]
    far_branch: FarBranch,

    pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    i_cache: BxIcache,
    fetch_mode_mask: u32,

    address_xlation: AddressXlation,
}

// Implement getters and setters

#[derive(Debug)]
struct AddressXlation {
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

#[derive(Debug)]
struct PdptrCache {
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

#[derive(Debug)]
pub struct BxRegsMsr {
    #[cfg(feature = "bx_support_apic")]
    apicbase: BxPhyAddress,

    // SYSCALL/SYSRET instruction msr's
    star: u64,

    lstar: u64,
    cstar: u64,
    fmask: u32,
    kernelgsbase: u64,
    tsc_aux: u32,

    // SYSENTER/SYSEXIT instruction msr's
    sysenter_cs_msr: u32,
    sysenter_esp_msr: BxAddress,
    sysenter_eip_msr: BxAddress,

    pat: BxPackedRegister,
    mtrrphys: [u64; 16],
    mtrrfix64k: BxPackedRegister,
    mtrrfix16k: [BxPackedRegister; 2],
    mtrrfix4k: [BxPackedRegister; 8],
    mtrr_deftype: u32,

    ia32_feature_ctrl: u32,

    svm_vm_cr: u32,
    svm_hsave_pa: u64,

    ia32_xss: u64,

    ia32_cet_control: [u64; 2], // indexed by CPL==3
    ia32_pl_ssp: [u64; 4],
    ia32_interrupt_ssp_table: u64,

    ia32_umwait_ctrl: u32, // SCA

                           // note from bochs source code:
                           /* TODO finish of the others */
}

#[cfg(feature = "bx_support_monitor_mwait")]
#[derive(Debug)]
pub struct MonitorAddr {
    monitor_addr: BxPhyAddress,
    armed_by: u32,
}

#[derive(Debug)]
struct Uintr {
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
