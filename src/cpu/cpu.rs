use core::marker::PhantomData;

use alloc::boxed::Box;

use crate::{
    config::{BxAddress, BxPhyAddress, BxPtrEquiv},
    cpu::{
        cpuid::{SVMExtensions, VMXExtensions},
        smm::SMMRAM_Fields,
        CpuError,
    },
};

use super::{
    apic::BxLocalApic,
    cpuid::BxCpuIdTrait,
    cpustats::BxCpuStatistics,
    crregs::{BxCr0, BxCr4, BxDr6, BxDr7, Xcr0, MSR},
    decoder::{BX_GENERAL_REGISTERS, BX_ISA_EXTENSIONS_ARRAY_SIZE, BX_XMM_REGISTERS},
    descriptor::{BxGlobalSegmentReg, BxSegmentReg},
    i387::{BxPackedRegister, I387},
    icache::{BxIcache, BxIcacheEntry},
    lazy_flags::BxLazyflagsEntry,
    svm::VmcbCache,
    tlb::BxHostpageaddr,
    vmx::{VmcsCache, VmcsMapping, VmxCap},
    xmm::{BxMxcsr, BxZmmReg},
};

const BX_ASYNC_EVENT_STOP_TRACE: u32 = 1 << 31;

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

pub(super) const BX_MSR_MAX_INDEX: usize = 0x1000;

#[allow(unused)]
#[derive(Debug)]
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
    pub(super) bx_cr0_t: BxCr0,
    pub(super) r2: BxAddress,
    pub(super) r3: BxAddress,

    pub(super) r4: BxCr4,
    pub(super) r4_suppmask: u32,

    pub(super) inaddr_width: u32,
    pub(super) fer_suppmask: u32,

    /// TSC: Time Stamp Counter
    /// Instead of storing a counter and incrementing it every instruction, we
    /// remember the time in ticks that it was reset to zero.  With a little
    /// algebra, we can also support setting it to something other than zero.
    /// Don't read this directly; use get_TSC and set_TSC to access the TSC.
    pub(super) sc_adjust: i64,

    pub(super) sc_offset: i64,

    pub(super) cr0: Xcr0,

    pub(super) cr0_suppmask: u32,
    pub(super) a32_xss_suppmask: u32,

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
    pub(super) activity_state: u32,

    pub(super) pending_event: u32,
    pub(super) event_mask: u32,
    // keep 32-bit because of BX_ASYNC_EVENT_STOP_TRACE
    pub(super) async_event: u32,

    pub(super) in_smm: bool,
    pub(super) cpu_mode: u32,
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
    pub(super) eip_fetch_ptr: &'c [u8],
    pub(super) p_addr_fetch_page: BxPhyAddress, // Guest physical address of current instruction page

    // Boundaries of current stack page, based on ESP
    // Linear address of current stack page
    pub(super) esp_page_bias: BxAddress,
    pub(super) esp_page_window_size: u32,
    pub(super) esp_host_ptr: &'c [u8],
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

    pub(super) pdptrcache: PdptrCache,

    /// An instruction cache.  Each entry should be exactly 32 bytes, and
    /// this structure should be aligned on a 32-byte boundary to be friendly
    /// with the host cache lines.
    pub(super) i_cache: BxIcache,
    pub(super) fetch_mode_mask: u32,

    pub(super) address_xlation: AddressXlation,

    /* Now other not so obvious fields */
    pub(super) smram_map: [u32; SMMRAM_Fields::SMRAM_FIELD_LAST as _],

    pub(super) phantom: PhantomData<I>,
}

// Implement getters and setters

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
                           //
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
    pub fn cpu_loop(&'c mut self) -> super::Result<()> {
        let stack_anchor = 0;

        self.cpuloop_stack_anchor = None;

        // FIXME: setjmp

        // We get here either by a normal function call, or by a longjmp
        // back from an exception() call.  In either case, commit the
        // new EIP/ESP, and set up other environmental fields.  This code
        // mirrors similar code below, after the interrupt() call.

        self.prev_rip = *self.rip();
        self.speculative_rsp = false;

        if self.in_vmx_guest {
            let vm = &mut self.vmcs;

            if vm.shadow_stack_prematurely_busy {
                return Err(CpuError::ShadowStackPrematurelyBusy);
            }
            vm.shadow_stack_prematurely_busy = false; // for safety
        }

        loop {
            // check on events which occurred for previous instructions (traps)
            // and ones which are asynchronous to the CPU (hardware interrupts)
            if self.async_event != 0 {
                self.handle_async_event();
                // If request to return to caller ASAP.
                return Ok(());
            }

            let entry = self.get_icache_entry();
            let mut i = entry.i;

            loop {
                self.before_execution(self.bx_cpuid);
                let old_rip = *self.rip();
                self.set_rip(old_rip + u64::from(i.ilen()));

                // TODO: Add actual instruction execution
                // TODO: And syncing of time

                if self.async_event > 0 {
                    break;
                }

                // clear stop trace magic indication that probably was set by repeat or branch32/64
                i = self.get_icache_entry().i;
            }

            self.async_event &= !BX_ASYNC_EVENT_STOP_TRACE;
        }

        todo!()
    }

    fn get_icache_entry(&mut self) -> BxIcacheEntry {
        unimplemented!()
    }

    fn before_execution(&mut self, cpu_id: u32) {
        todo!()
    }
}
