// Intel VT-x (VMX) — Bochs cpu/vmx.cc.

#![allow(dead_code, non_camel_case_types)]

use super::cpu::Exception;
use super::decoder::{BxSegregs, Instruction};
use super::instrumentation::Instrumentation;
use super::{BxCpuC, BxCpuIdTrait, Result};

// Bochs vmx.h BX_IA32_FEATURE_CONTROL_* bits.
pub const BX_IA32_FEATURE_CONTROL_LOCK_BIT: u32 = 0x1;
pub const BX_IA32_FEATURE_CONTROL_VMX_ENABLE_BIT: u32 = 0x4;
pub const BX_IA32_FEATURE_CONTROL_BITS: u32 =
    BX_IA32_FEATURE_CONTROL_LOCK_BIT | BX_IA32_FEATURE_CONTROL_VMX_ENABLE_BIT;

/// The VMCS revision ID this implementation advertises via IA32_VMX_BASIC.
/// Bochs uses 1 — kernels treat any value the host returns as authoritative.
pub const BX_VMCS_REVISION_ID: u32 = 1;

/// Fixed offset within the 4 KiB VMCS region where we store the launch-state
/// flag. Bochs picks an implementation-specific offset via its `vmcs_map`;
/// since our table is not ported yet, pin launch-state to bytes 4..8 (right
/// after the revision ID dword at offset 0). This is invisible to guests —
/// they only touch this byte via VMCLEAR / VMLAUNCH / VMRESUME semantics.
pub const VMCS_LAUNCH_STATE_OFFSET: u64 = 4;

pub const VMCS_STATE_CLEAR: u32 = 0;
pub const VMCS_STATE_LAUNCHED: u32 = 1;

// ──────────────────────────────────────────────────────────────────────────
// VMCS field encodings — Bochs cpu/vmx.h. Grouped by width and role.
// ──────────────────────────────────────────────────────────────────────────

// 16-bit guest selectors (Bochs vmx.h VMCS_16BIT_GUEST_*_SELECTOR).
const VMCS_16BIT_GUEST_ES_SELECTOR: u32 = 0x0800;
const VMCS_16BIT_GUEST_CS_SELECTOR: u32 = 0x0802;
const VMCS_16BIT_GUEST_SS_SELECTOR: u32 = 0x0804;
const VMCS_16BIT_GUEST_DS_SELECTOR: u32 = 0x0806;
const VMCS_16BIT_GUEST_FS_SELECTOR: u32 = 0x0808;
const VMCS_16BIT_GUEST_GS_SELECTOR: u32 = 0x080A;
const VMCS_16BIT_GUEST_LDTR_SELECTOR: u32 = 0x080C;
const VMCS_16BIT_GUEST_TR_SELECTOR: u32 = 0x080E;

// 16-bit host selectors (Bochs vmx.h VMCS_16BIT_HOST_*_SELECTOR).
const VMCS_16BIT_HOST_ES_SELECTOR: u32 = 0x0C00;
const VMCS_16BIT_HOST_CS_SELECTOR: u32 = 0x0C02;
const VMCS_16BIT_HOST_SS_SELECTOR: u32 = 0x0C04;
const VMCS_16BIT_HOST_DS_SELECTOR: u32 = 0x0C06;
const VMCS_16BIT_HOST_FS_SELECTOR: u32 = 0x0C08;
const VMCS_16BIT_HOST_GS_SELECTOR: u32 = 0x0C0A;
const VMCS_16BIT_HOST_TR_SELECTOR: u32 = 0x0C0C;

// 64-bit control / guest / host fields.
const VMCS_64BIT_CONTROL_IO_BITMAP_A: u32 = 0x2000;
const VMCS_64BIT_CONTROL_IO_BITMAP_B: u32 = 0x2002;
const VMCS_64BIT_CONTROL_MSR_BITMAPS: u32 = 0x2004;
const VMCS_64BIT_CONTROL_TSC_OFFSET: u32 = 0x2010;
const VMCS_64BIT_GUEST_LINK_POINTER: u32 = 0x2800;
const VMCS_64BIT_GUEST_IA32_PAT: u32 = 0x2804;
const VMCS_64BIT_GUEST_IA32_EFER: u32 = 0x2806;
const VMCS_64BIT_HOST_IA32_PAT: u32 = 0x2C00;
const VMCS_64BIT_HOST_IA32_EFER: u32 = 0x2C02;

// 32-bit control fields.
const VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS: u32 = 0x4000;
const VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS: u32 = 0x4002;
const VMCS_32BIT_CONTROL_EXECUTION_BITMAP: u32 = 0x4004;
const VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MASK: u32 = 0x4006;
const VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MATCH: u32 = 0x4008;
const VMCS_32BIT_CONTROL_CR3_TARGET_COUNT: u32 = 0x400A;
const VMCS_CR3_TARGET0: u32 = 0x6008;
const VMCS_CR3_TARGET1: u32 = 0x600A;
const VMCS_CR3_TARGET2: u32 = 0x600C;
const VMCS_CR3_TARGET3: u32 = 0x600E;
const VMCS_32BIT_CONTROL_VMEXIT_CONTROLS: u32 = 0x400C;
const VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS: u32 = 0x401E;
const VMCS_32BIT_CONTROL_VMENTRY_CONTROLS: u32 = 0x4012;
const VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO: u32 = 0x4016;
const VMCS_32BIT_CONTROL_VMENTRY_EXCEPTION_ERR_CODE: u32 = 0x4018;
const VMCS_32BIT_CONTROL_VMENTRY_INSTRUCTION_LENGTH: u32 = 0x401A;

// 32-bit read-only exit data.
const VMCS_32BIT_INSTRUCTION_ERROR: u32 = 0x4400;
const VMCS_32BIT_VMEXIT_REASON: u32 = 0x4402;
const VMCS_32BIT_VMEXIT_INTERRUPTION_INFO: u32 = 0x4404;
const VMCS_32BIT_VMEXIT_INTERRUPTION_ERR_CODE: u32 = 0x4406;
const VMCS_32BIT_IDT_VECTORING_INFO: u32 = 0x4408;
const VMCS_32BIT_IDT_VECTORING_ERR_CODE: u32 = 0x440A;
const VMCS_32BIT_VMEXIT_INSTRUCTION_LENGTH: u32 = 0x440C;
const VMCS_32BIT_VMEXIT_INSTRUCTION_INFO: u32 = 0x440E;
const VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE: u32 = 0x482E;

// 32-bit guest state.
const VMCS_32BIT_GUEST_ES_LIMIT: u32 = 0x4800;
const VMCS_32BIT_GUEST_CS_LIMIT: u32 = 0x4802;
const VMCS_32BIT_GUEST_SS_LIMIT: u32 = 0x4804;
const VMCS_32BIT_GUEST_DS_LIMIT: u32 = 0x4806;
const VMCS_32BIT_GUEST_FS_LIMIT: u32 = 0x4808;
const VMCS_32BIT_GUEST_GS_LIMIT: u32 = 0x480A;
const VMCS_32BIT_GUEST_LDTR_LIMIT: u32 = 0x480C;
const VMCS_32BIT_GUEST_TR_LIMIT: u32 = 0x480E;
const VMCS_32BIT_GUEST_GDTR_LIMIT: u32 = 0x4810;
const VMCS_32BIT_GUEST_IDTR_LIMIT: u32 = 0x4812;
const VMCS_32BIT_GUEST_ES_ACCESS_RIGHTS: u32 = 0x4814;
const VMCS_32BIT_GUEST_CS_ACCESS_RIGHTS: u32 = 0x4816;
const VMCS_32BIT_GUEST_SS_ACCESS_RIGHTS: u32 = 0x4818;
const VMCS_32BIT_GUEST_DS_ACCESS_RIGHTS: u32 = 0x481A;
const VMCS_32BIT_GUEST_FS_ACCESS_RIGHTS: u32 = 0x481C;
const VMCS_32BIT_GUEST_GS_ACCESS_RIGHTS: u32 = 0x481E;
const VMCS_32BIT_GUEST_LDTR_ACCESS_RIGHTS: u32 = 0x4820;
const VMCS_32BIT_GUEST_TR_ACCESS_RIGHTS: u32 = 0x4822;
const VMCS_32BIT_GUEST_INTERRUPTIBILITY_STATE: u32 = 0x4824;
const VMCS_32BIT_GUEST_ACTIVITY_STATE: u32 = 0x4826;
const VMCS_32BIT_GUEST_IA32_SYSENTER_CS_MSR: u32 = 0x482A;

// 32-bit host state.
const VMCS_32BIT_HOST_IA32_SYSENTER_CS_MSR: u32 = 0x4C00;

// Natural-width control fields.
const VMCS_CONTROL_CR0_GUEST_HOST_MASK: u32 = 0x6000;
const VMCS_CONTROL_CR4_GUEST_HOST_MASK: u32 = 0x6002;
const VMCS_CONTROL_CR0_READ_SHADOW: u32 = 0x6004;
const VMCS_CONTROL_CR4_READ_SHADOW: u32 = 0x6006;

// Natural-width read-only exit data.
const VMCS_VMEXIT_QUALIFICATION: u32 = 0x6400;
const VMCS_VMEXIT_GUEST_LINEAR_ADDR: u32 = 0x640A;

// Natural-width guest state.
const VMCS_GUEST_CR0: u32 = 0x6800;
const VMCS_GUEST_CR3: u32 = 0x6802;
const VMCS_GUEST_CR4: u32 = 0x6804;
const VMCS_GUEST_ES_BASE: u32 = 0x6806;
const VMCS_GUEST_CS_BASE: u32 = 0x6808;
const VMCS_GUEST_SS_BASE: u32 = 0x680A;
const VMCS_GUEST_DS_BASE: u32 = 0x680C;
const VMCS_GUEST_FS_BASE: u32 = 0x680E;
const VMCS_GUEST_GS_BASE: u32 = 0x6810;
const VMCS_GUEST_LDTR_BASE: u32 = 0x6812;
const VMCS_GUEST_TR_BASE: u32 = 0x6814;
const VMCS_GUEST_GDTR_BASE: u32 = 0x6816;
const VMCS_GUEST_IDTR_BASE: u32 = 0x6818;
const VMCS_GUEST_DR7: u32 = 0x681A;
const VMCS_GUEST_RSP: u32 = 0x681C;
const VMCS_GUEST_RIP: u32 = 0x681E;
const VMCS_GUEST_RFLAGS: u32 = 0x6820;
const VMCS_GUEST_IA32_SYSENTER_ESP_MSR: u32 = 0x6824;
const VMCS_GUEST_IA32_SYSENTER_EIP_MSR: u32 = 0x6826;

// Natural-width host state.
const VMCS_HOST_CR0: u32 = 0x6C00;
const VMCS_HOST_CR3: u32 = 0x6C02;
const VMCS_HOST_CR4: u32 = 0x6C04;
const VMCS_HOST_FS_BASE: u32 = 0x6C06;
const VMCS_HOST_GS_BASE: u32 = 0x6C08;
const VMCS_HOST_TR_BASE: u32 = 0x6C0A;
const VMCS_HOST_GDTR_BASE: u32 = 0x6C0C;
const VMCS_HOST_IDTR_BASE: u32 = 0x6C0E;
const VMCS_HOST_IA32_SYSENTER_ESP_MSR: u32 = 0x6C10;
const VMCS_HOST_IA32_SYSENTER_EIP_MSR: u32 = 0x6C12;
const VMCS_HOST_RSP: u32 = 0x6C14;
const VMCS_HOST_RIP: u32 = 0x6C16;

/// Bochs vmx.h `enum VMX_vmexit_reason` — every reason the host reads from
/// VMCS_EXIT_REASON after a VM-exit. Session 5 port; the individual exit
/// paths that set each reason land incrementally in Sessions 5+.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmxVmexitReason {
    ExceptionNmi = 0,
    ExternalInterrupt = 1,
    TripleFault = 2,
    Init = 3,
    Sipi = 4,
    IoSmi = 5,
    Smi = 6,
    InterruptWindow = 7,
    NmiWindow = 8,
    TaskSwitch = 9,
    Cpuid = 10,
    Getsec = 11,
    Hlt = 12,
    Invd = 13,
    Invlpg = 14,
    Rdpmc = 15,
    Rdtsc = 16,
    Rsm = 17,
    Vmcall = 18,
    Vmclear = 19,
    Vmlaunch = 20,
    Vmptrld = 21,
    Vmptrst = 22,
    Vmread = 23,
    Vmresume = 24,
    Vmwrite = 25,
    Vmxoff = 26,
    Vmxon = 27,
    CrAccess = 28,
    DrAccess = 29,
    IoInstruction = 30,
    Rdmsr = 31,
    Wrmsr = 32,
    VmentryFailureGuestState = 33,
    VmentryFailureMsr = 34,
    Reserved35 = 35,
    Mwait = 36,
    MonitorTrapFlag = 37,
    Reserved38 = 38,
    Monitor = 39,
    Pause = 40,
    VmentryFailureMca = 41,
    Reserved42 = 42,
    TprThreshold = 43,
    ApicAccess = 44,
    VirtualizedEoi = 45,
    GdtrIdtrAccess = 46,
    LdtrTrAccess = 47,
    EptViolation = 48,
    EptMisconfiguration = 49,
    Invept = 50,
    Rdtscp = 51,
    VmxPreemptionTimerExpired = 52,
    Invvpid = 53,
    Wbinvd = 54,
    Xsetbv = 55,
    ApicWrite = 56,
    Rdrand = 57,
    Invpcid = 58,
    Vmfunc = 59,
    Encls = 60,
    Rdseed = 61,
    PmlLogfull = 62,
    Xsaves = 63,
    Xrstors = 64,
    Pconfig = 65,
    Spp = 66,
    Umwait = 67,
    Tpause = 68,
    Loadiwkey = 69,
    Enclv = 70,
    Reserved71 = 71,
    EnqcmdPasid = 72,
    EnqcmdsPasid = 73,
    BusLock = 74,
    InstructionTimeout = 75,
    Seamcall = 76,
    Tdcall = 77,
    Rdmsrlist = 78,
    Wrmsrlist = 79,
    Urdmsr = 80,
    Uwrmsr = 81,
    Reserved82 = 82,
    Reserved83 = 83,
    RdmsrImm = 84,
    Wrmsrns = 85,
}

// ──────────────────────────────────────────────────────────────────────────
// VM-execution control bits — Bochs cpu/vmx_ctrls.h.
//
// The storage in `BxVmcs` is still raw u32 (proc_based_ctls, etc.), so the
// intercept checks below `& mask != 0` against the right control word. A
// later pass can replace the u32s with bitflags once the control-storage
// refactor lands; the bit numbers stay stable either way.
// ──────────────────────────────────────────────────────────────────────────

// Pin-based VM-execution controls.
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_EXTERNAL_INTERRUPT_VMEXIT: u32 = 1 << 0;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_NMI_EXITING: u32 = 1 << 3;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_VIRTUAL_NMI: u32 = 1 << 5;
pub(super) const VMX_PIN_BASED_VMEXEC_CTRL_VMX_PREEMPTION_TIMER_VMEXIT: u32 = 1 << 6;

// Primary processor-based VM-execution controls (Bochs VmxVmexec1Controls).
pub(super) const VMX_VM_EXEC_CTRL1_INTERRUPT_WINDOW_VMEXIT: u32 = 1 << 2;
pub(super) const VMX_VM_EXEC_CTRL1_HLT_VMEXIT: u32 = 1 << 7;
pub(super) const VMX_VM_EXEC_CTRL1_INVLPG_VMEXIT: u32 = 1 << 9;
pub(super) const VMX_VM_EXEC_CTRL1_MWAIT_VMEXIT: u32 = 1 << 10;
pub(super) const VMX_VM_EXEC_CTRL1_RDPMC_VMEXIT: u32 = 1 << 11;
pub(super) const VMX_VM_EXEC_CTRL1_RDTSC_VMEXIT: u32 = 1 << 12;
pub(super) const VMX_VM_EXEC_CTRL1_CR3_WRITE_VMEXIT: u32 = 1 << 15;
pub(super) const VMX_VM_EXEC_CTRL1_CR3_READ_VMEXIT: u32 = 1 << 16;
pub(super) const VMX_VM_EXEC_CTRL1_CR8_WRITE_VMEXIT: u32 = 1 << 19;
pub(super) const VMX_VM_EXEC_CTRL1_CR8_READ_VMEXIT: u32 = 1 << 20;
pub(super) const VMX_VM_EXEC_CTRL1_NMI_WINDOW_EXITING: u32 = 1 << 22;
pub(super) const VMX_VM_EXEC_CTRL1_DRX_ACCESS_VMEXIT: u32 = 1 << 23;
pub(super) const VMX_VM_EXEC_CTRL1_IO_VMEXIT: u32 = 1 << 24;
pub(super) const VMX_VM_EXEC_CTRL1_IO_BITMAPS: u32 = 1 << 25;
pub(super) const VMX_VM_EXEC_CTRL1_MONITOR_TRAP_FLAG: u32 = 1 << 27;
pub(super) const VMX_VM_EXEC_CTRL1_MSR_BITMAPS: u32 = 1 << 28;
pub(super) const VMX_VM_EXEC_CTRL1_MONITOR_VMEXIT: u32 = 1 << 29;
pub(super) const VMX_VM_EXEC_CTRL1_PAUSE_VMEXIT: u32 = 1 << 30;
pub(super) const VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS: u32 = 1 << 31;

// Secondary processor-based VM-execution controls (Bochs VmxVmexec2Controls).
pub(super) const VMX_VM_EXEC_CTRL2_DESCRIPTOR_TABLE_VMEXIT: u32 = 1 << 2;
pub(super) const VMX_VM_EXEC_CTRL2_RDTSCP: u32 = 1 << 3;
pub(super) const VMX_VM_EXEC_CTRL2_WBINVD_VMEXIT: u32 = 1 << 6;
pub(super) const VMX_VM_EXEC_CTRL2_PAUSE_LOOP_VMEXIT: u32 = 1 << 10;
pub(super) const VMX_VM_EXEC_CTRL2_RDRAND_VMEXIT: u32 = 1 << 11;
pub(super) const VMX_VM_EXEC_CTRL2_INVPCID: u32 = 1 << 12;
pub(super) const VMX_VM_EXEC_CTRL2_RDSEED_VMEXIT: u32 = 1 << 16;
pub(super) const VMX_VM_EXEC_CTRL2_XSAVES_XRSTORS: u32 = 1 << 20;
pub(super) const VMX_VM_EXEC_CTRL2_UMWAIT_TPAUSE_VMEXIT: u32 = 1 << 26;

// VM-exit control bits — Bochs vmx_ctrls.h.
pub(super) const VMX_VMEXIT_CTRL1_INTA_ON_VMEXIT: u32 = 1 << 15;

/// VMX-instruction error codes written into the VMCS 32-bit
/// VMCS_32BIT_INSTRUCTION_ERROR field by `VMfail`.
/// Mirrors Bochs vmx.h `enum VMX_error_code`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmxErr {
    NoError = 0,
    VmcallInVmxRootOperation = 1,
    VmclearWithInvalidAddr = 2,
    VmclearWithVmxonVmcsPtr = 3,
    VmlaunchNonClearVmcs = 4,
    VmresumeNonLaunchedVmcs = 5,
    VmresumeVmcsCorrupted = 6,
    VmentryInvalidVmControlField = 7,
    VmentryInvalidVmHostStateField = 8,
    VmptrldInvalidPhysicalAddress = 9,
    VmptrldWithVmxonPtr = 10,
    VmptrldIncorrectVmcsRevisionId = 11,
    UnsupportedVmcsComponentAccess = 12,
    VmwriteReadOnlyVmcsComponent = 13,
    VmxonInVmxRootOperation = 15,
    VmentryInvalidExecutiveVmcs = 16,
    VmentryNonLaunchedExecutiveVmcs = 17,
    VmentryNotVmxonExecutiveVmcs = 18,
    VmcallNonClearVmcs = 19,
    VmcallInvalidVmexitField = 20,
    VmcallInvalidMsegRevisionId = 22,
    VmxoffWithConfiguredSmmMonitor = 23,
    VmcallWithInvalidSmmMonitorFeatures = 24,
    VmentryInvalidVmControlFieldInExecutiveVmcs = 25,
    VmentryMovSsBlocking = 26,
    InvalidInveptInvvpid = 28,
}

// Legacy VMCS wrapper kept for the (still-stubbed) VMCS memory pointer path.
// Extended incrementally in Sessions 4+ as VMCS fields / caching are added.
pub type VmcsCache = BxVmcs;

#[derive(Debug, Default)]
pub struct VmcsMapping {}

use super::vmx_ctrls::{VmxPinBasedVmexecControls, VmxVmexec1Controls, VmxVmexec2Controls};

/// In-memory VMCS cache mirroring Bochs cpu/vmx.h `VMCS_CACHE`. Holds the
/// host and guest state that VMLAUNCH / VMRESUME / VMEXIT swap between, plus
/// the read-only fields updated by the VMEXIT machinery. Fields are laid
/// out as Rust structures so VMREAD / VMWRITE can dispatch by encoding
/// without a raw byte-offset table.
#[derive(Debug, Default)]
pub struct BxVmcs {
    // Launch state — Bochs sets this to VMCS_STATE_LAUNCHED after the first
    // successful VMLAUNCH; VMCLEAR resets it.
    pub launched: bool,

    // ---- Host state (saved on successful VMENTRY, restored on VMEXIT) ----
    pub host_cr0: u64,
    pub host_cr3: u64,
    pub host_cr4: u64,
    pub host_rsp: u64,
    pub host_rip: u64,
    pub host_cs_selector: u16,
    pub host_ss_selector: u16,
    pub host_ds_selector: u16,
    pub host_es_selector: u16,
    pub host_fs_selector: u16,
    pub host_gs_selector: u16,
    pub host_tr_selector: u16,
    pub host_fs_base: u64,
    pub host_gs_base: u64,
    pub host_tr_base: u64,
    pub host_gdtr_base: u64,
    pub host_idtr_base: u64,
    pub host_ia32_efer: u64,
    pub host_ia32_pat: u64,
    pub host_sysenter_cs: u32,
    pub host_sysenter_esp: u64,
    pub host_sysenter_eip: u64,

    // ---- Guest state (loaded on VMENTRY, saved on VMEXIT) ----
    pub guest_cr0: u64,
    pub guest_cr3: u64,
    pub guest_cr4: u64,
    pub guest_rsp: u64,
    pub guest_rip: u64,
    pub guest_rflags: u64,
    pub guest_dr7: u64,
    pub guest_ia32_efer: u64,
    pub guest_ia32_pat: u64,
    pub guest_ia32_sysenter_cs: u32,
    pub guest_ia32_sysenter_esp: u64,
    pub guest_ia32_sysenter_eip: u64,
    pub guest_cs_selector: u16,
    pub guest_ss_selector: u16,
    pub guest_ds_selector: u16,
    pub guest_es_selector: u16,
    pub guest_fs_selector: u16,
    pub guest_gs_selector: u16,
    pub guest_ldtr_selector: u16,
    pub guest_tr_selector: u16,
    pub guest_cs_base: u64,
    pub guest_ss_base: u64,
    pub guest_ds_base: u64,
    pub guest_es_base: u64,
    pub guest_fs_base: u64,
    pub guest_gs_base: u64,
    pub guest_ldtr_base: u64,
    pub guest_tr_base: u64,
    pub guest_gdtr_base: u64,
    pub guest_idtr_base: u64,
    pub guest_cs_limit: u32,
    pub guest_ss_limit: u32,
    pub guest_ds_limit: u32,
    pub guest_es_limit: u32,
    pub guest_fs_limit: u32,
    pub guest_gs_limit: u32,
    pub guest_ldtr_limit: u32,
    pub guest_tr_limit: u32,
    pub guest_gdtr_limit: u32,
    pub guest_idtr_limit: u32,
    pub guest_cs_ar: u32,
    pub guest_ss_ar: u32,
    pub guest_ds_ar: u32,
    pub guest_es_ar: u32,
    pub guest_fs_ar: u32,
    pub guest_gs_ar: u32,
    pub guest_ldtr_ar: u32,
    pub guest_tr_ar: u32,
    pub guest_activity_state: u32,
    pub guest_interruptibility_state: u32,

    // ---- Exit info (written on VMEXIT) ----
    pub vm_instruction_error: u32,
    pub exit_reason: u32,
    pub exit_qualification: u64,
    pub exit_intr_info: u32,
    pub exit_intr_error_code: u32,
    pub exit_instruction_length: u32,
    pub exit_instruction_info: u32,
    pub idt_vectoring_info: u32,
    pub idt_vectoring_error_code: u32,
    pub guest_linear_addr: u64,

    // ---- Control fields (written by host via VMWRITE before VMLAUNCH) ----
    pub pin_based_ctls: u32,
    pub proc_based_ctls: u32,
    pub secondary_proc_based_ctls: u32,
    pub vm_exit_ctls: u32,
    pub vm_entry_ctls: u32,
    pub vm_entry_intr_info: u32,
    pub vm_entry_exception_error_code: u32,
    pub vm_entry_instruction_length: u32,
    pub exception_bitmap: u32,
    // Page-fault error code mask/match for VMEXIT on #PF:
    // a #PF takes a VMEXIT iff `(errcode & vm_pf_mask) == vm_pf_match` equals
    // the exception_bitmap bit for #PF. Bochs vm_pf_mask / vm_pf_match.
    pub vm_pf_mask: u32,
    pub vm_pf_match: u32,
    // CR3-target filter for MOV CR3 writes (Bochs vm_cr3_target_cnt /
    // vm_cr3_target_value). A CR3 write that matches any enabled target
    // value does *not* VMEXIT even when CR3_WRITE_VMEXIT is set.
    pub vm_cr3_target_cnt: u32,
    pub vm_cr3_target_value: [u64; 4],
    pub cr0_guest_host_mask: u64,
    pub cr4_guest_host_mask: u64,
    pub cr0_read_shadow: u64,
    pub cr4_read_shadow: u64,
    pub vmcs_link_pointer: u64,
    pub tsc_offset: u64,
    /// Guest-physical address of the 4KiB MSR bitmap when
    /// `VMX_VM_EXEC_CTRL1_MSR_BITMAPS` is set. Bochs `msr_bitmap_addr`.
    pub msr_bitmap_addr: u64,
    /// Guest-physical addresses of the two 4KiB I/O permission bitmaps
    /// (A: ports 0x0000..=0x7FFF, B: ports 0x8000..=0xFFFF) when
    /// `VMX_VM_EXEC_CTRL1_IO_BITMAPS` is set. Bochs `io_bitmap_addr[2]`.
    pub io_bitmap_addr: [u64; 2],
    /// 32-bit countdown value loaded into the VMX preemption timer at
    /// VMENTER and (when STORE_VMX_PREEMPTION_TIMER is set) snapshotted
    /// back at VMEXIT. Bochs reads this from the guest VMCS in
    /// vmlaunch/vmresume; ticking happens through the LAPIC.
    pub vmx_preemption_timer_value: u32,

    // Wire-compat bag kept from earlier scaffolding; some older call sites
    // still reach for these. They stay zero until a VMM populates them.
    pin_vmexec_ctrls: VmxPinBasedVmexecControls,
    vmexec_ctrls1: VmxVmexec1Controls,
    vmexec_ctrls2: VmxVmexec2Controls,
    pub(crate) shadow_stack_prematurely_busy: bool,
}

pub type BxVmxCap = VmxCap;

#[derive(Debug, Default)]
pub struct VmxCap {}

impl<I: BxCpuIdTrait, T: Instrumentation> BxCpuC<'_, I, T> {
    // =========================================================================
    // VMX flag-based result helpers — Bochs cpu.h VMsucceed / VMfailInvalid
    // and vmx.cc BX_CPU_C::VMfail.
    // =========================================================================

    /// Bochs cpu.h VMsucceed — clear OSZAPC.
    pub(super) fn vmsucceed(&mut self) {
        self.oszapc.set_oszapc_logic_32(1);
    }

    /// Bochs cpu.h VMfailInvalid — clear OSZAPC then assert CF.
    pub(super) fn vmfail_invalid(&mut self) {
        self.oszapc.set_oszapc_logic_32(1);
        self.oszapc.set_cf(true);
    }

    /// Bochs vmx.cc BX_CPU_C::VMfail — writes the error code into the current
    /// VMCS (if any) and asserts ZF; otherwise asserts CF.
    pub(super) fn vmfail(&mut self, error: VmxErr) {
        self.oszapc.set_oszapc_logic_32(1);
        if self.vmcsptr != super::vmcs::BX_INVALID_VMCSPTR {
            self.oszapc.set_zf(true);
            // Bochs VMwrite32(VMCS_32BIT_INSTRUCTION_ERROR, error).
            self.vmcs.vm_instruction_error = error as u32;
        } else {
            self.oszapc.set_cf(true);
        }
    }

    // =========================================================================
    // VMXON — enter VMX operation mode (opcode F3 0F C7 /6 m64)
    // Bochs vmx.cc BX_CPU_C::VMXON.
    // =========================================================================

    pub(super) fn vmxon(&mut self, instr: &Instruction) -> Result<()> {
        // Bochs vmx.cc: UD if CR4.VMXE clear, not in protected mode, or in
        // long-compat mode.
        if !self.cr4.vmxe() || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }

        if !self.in_vmx {
            // Entering VMX root from outside VMX.
            let cpl = self.cs_rpl();
            if cpl != 0
                || !self.cr0.ne()
                || !self.cr0.pe()
                || !self.a20_enabled()
                || (self.msr.ia32_feature_ctrl & BX_IA32_FEATURE_CONTROL_LOCK_BIT) == 0
                || (self.msr.ia32_feature_ctrl & BX_IA32_FEATURE_CONTROL_VMX_ENABLE_BIT) == 0
            {
                tracing::trace!("VMXON: preconditions not met → #GP(0)");
                return self.exception(Exception::Gp, 0);
            }

            // Operand is a 64-bit physical address of the VMXON region.
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            let paddr = if self.long64_mode() {
                self.read_virtual_qword_64(seg, eaddr)?
            } else {
                self.read_virtual_qword(seg, eaddr as u32)?
            };

            // Must be 4 KiB-aligned and within the physical-address width
            // Bochs advertises (BX_PHY_ADDRESS_WIDTH = 40 bits in our config).
            const BX_PHY_ADDRESS_WIDTH: u32 = 40;
            if paddr == 0
                || (paddr & 0xFFF) != 0
                || (paddr >> BX_PHY_ADDRESS_WIDTH) != 0
            {
                tracing::trace!("VMXON: invalid or misaligned paddr {:#x}", paddr);
                self.vmfail_invalid();
                return Ok(());
            }

            // Check revision ID at paddr matches the emulator's.
            let rev = self.vmx_read_revision_id(paddr);
            if rev != BX_VMCS_REVISION_ID {
                tracing::trace!(
                    "VMXON: VMCS revision mismatch at {:#x}: have {:#x} want {:#x}",
                    paddr, rev, BX_VMCS_REVISION_ID
                );
                self.vmfail_invalid();
                return Ok(());
            }

            self.vmcsptr = super::vmcs::BX_INVALID_VMCSPTR;
            self.vmxonptr = paddr;
            self.in_vmx = true;
            self.mask_event(Self::BX_EVENT_INIT);
            self.monitor.reset_monitor();
            self.vmsucceed();
            return Ok(());
        }

        // Already in VMX non-root → VMEXIT (deferred until Session 4 wires the
        // VMX exit path). For now, surface as #GP so guests observe a failure.
        if self.in_vmx_guest {
            tracing::trace!("VMXON: in VMX guest — VMEXIT reason VMXON (stub #GP)");
            return self.exception(Exception::Gp, 0);
        }

        // Already in VMX root operation.
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        self.vmfail(VmxErr::VmxonInVmxRootOperation);
        Ok(())
    }

    // =========================================================================
    // VMXOFF — leave VMX operation mode (opcode 0F 01 C4)
    // Bochs vmx.cc BX_CPU_C::VMXOFF.
    // =========================================================================

    pub(super) fn vmxoff(&mut self, _instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }

        if self.in_vmx_guest {
            // Bochs VMexit(VMX_VMEXIT_VMXOFF, 0) — full VM-exit path ships in
            // Session 5. For Session 3, collapse to #GP so guest-mode VMXOFF
            // doesn't silently succeed.
            return self.exception(Exception::Gp, 0);
        }

        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        self.vmxonptr = super::vmcs::BX_INVALID_VMCSPTR;
        self.in_vmx = false;
        self.unmask_event(Self::BX_EVENT_INIT);
        self.monitor.reset_monitor();
        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMCLEAR — initialise a VMCS in memory, mark launch-state clear.
    // Bochs vmx.cc BX_CPU_C::VMCLEAR.
    // =========================================================================

    pub(super) fn vmclear(&mut self, instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            // VMEXIT path lands in Session 5 — for now surface as #GP so guest
            // VMCLEAR doesn't silently succeed.
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let paddr = if self.long64_mode() {
            self.read_virtual_qword_64(seg, eaddr)?
        } else {
            self.read_virtual_qword(seg, eaddr as u32)?
        };

        const BX_PHY_ADDRESS_WIDTH: u32 = 40;
        if paddr == 0
            || (paddr & 0xFFF) != 0
            || (paddr >> BX_PHY_ADDRESS_WIDTH) != 0
        {
            self.vmfail(VmxErr::VmclearWithInvalidAddr);
            return Ok(());
        }

        if paddr == self.vmxonptr {
            self.vmfail(VmxErr::VmclearWithVmxonVmcsPtr);
            return Ok(());
        }

        // Clear the VMCS launch-state flag in guest-physical memory.
        self.mem_write_dword(paddr + VMCS_LAUNCH_STATE_OFFSET, VMCS_STATE_CLEAR);

        // If we were using this VMCS as the current one, drop it.
        if paddr == self.vmcsptr {
            self.vmcsptr = super::vmcs::BX_INVALID_VMCSPTR;
        }

        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMPTRLD — load VMCS pointer from memory operand.
    // Bochs vmx.cc BX_CPU_C::VMPTRLD.
    // =========================================================================

    pub(super) fn vmptrld(&mut self, instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let paddr = if self.long64_mode() {
            self.read_virtual_qword_64(seg, eaddr)?
        } else {
            self.read_virtual_qword(seg, eaddr as u32)?
        };

        const BX_PHY_ADDRESS_WIDTH: u32 = 40;
        if paddr == 0
            || (paddr & 0xFFF) != 0
            || (paddr >> BX_PHY_ADDRESS_WIDTH) != 0
        {
            self.vmfail(VmxErr::VmptrldInvalidPhysicalAddress);
            return Ok(());
        }

        if paddr == self.vmxonptr {
            self.vmfail(VmxErr::VmptrldWithVmxonPtr);
            return Ok(());
        }

        let revision = self.vmx_read_revision_id(paddr);
        if revision != BX_VMCS_REVISION_ID {
            tracing::trace!(
                "VMPTRLD: revision mismatch at {:#x}: {:#x} vs {:#x}",
                paddr, revision, BX_VMCS_REVISION_ID
            );
            self.vmfail(VmxErr::VmptrldIncorrectVmcsRevisionId);
            return Ok(());
        }

        self.vmcsptr = paddr;
        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMPTRST — store current VMCS pointer to memory operand.
    // Bochs vmx.cc BX_CPU_C::VMPTRST.
    // =========================================================================

    pub(super) fn vmptrst(&mut self, instr: &Instruction) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let seg = BxSegregs::from(instr.seg());
        let eaddr = self.resolve_addr(instr);
        let val = self.vmcsptr;
        if self.long64_mode() {
            self.write_virtual_qword_64(seg, eaddr, val)?;
        } else {
            self.write_virtual_qword(seg, eaddr as u32, val)?;
        }
        self.vmsucceed();
        Ok(())
    }

    // =========================================================================
    // VMREAD / VMWRITE — VMCS field access, dispatched by encoding through
    // the named fields on `BxVmcs`. Matches Bochs' VMread16/32/64/natural +
    // VMwrite*. Field encodings mirror Bochs cpu/vmx.h. Unhandled encodings
    // raise VMXERR_UNSUPPORTED_VMCS_COMPONENT_ACCESS so nothing corrupts
    // silently.
    // =========================================================================

    pub(super) fn vmread_impl(&mut self, encoding: u32) -> Result<u64> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            self.exception(Exception::Ud, 0)?;
            unreachable!();
        }
        if self.in_vmx_guest {
            self.exception(Exception::Gp, 0)?;
            unreachable!();
        }
        if self.cs_rpl() != 0 {
            self.exception(Exception::Gp, 0)?;
            unreachable!();
        }
        if self.vmcsptr == super::vmcs::BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(0);
        }

        let v = self.vmcs_read_field(encoding);
        match v {
            Some(val) => {
                self.vmsucceed();
                Ok(val)
            }
            None => {
                self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
                Ok(0)
            }
        }
    }

    pub(super) fn vmwrite_impl(&mut self, encoding: u32, value: u64) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if self.vmcsptr == super::vmcs::BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }

        if self.vmcs_write_field(encoding, value) {
            self.vmsucceed();
            Ok(())
        } else {
            self.vmfail(VmxErr::UnsupportedVmcsComponentAccess);
            Ok(())
        }
    }

    /// Dispatch a VMCS field encoding → named field in `self.vmcs`.
    /// Returns `None` if the encoding is not recognised.
    fn vmcs_read_field(&self, encoding: u32) -> Option<u64> {
        let v = &self.vmcs;
        Some(match encoding {
            // 16-bit guest selectors.
            VMCS_16BIT_GUEST_ES_SELECTOR => v.guest_es_selector as u64,
            VMCS_16BIT_GUEST_CS_SELECTOR => v.guest_cs_selector as u64,
            VMCS_16BIT_GUEST_SS_SELECTOR => v.guest_ss_selector as u64,
            VMCS_16BIT_GUEST_DS_SELECTOR => v.guest_ds_selector as u64,
            VMCS_16BIT_GUEST_FS_SELECTOR => v.guest_fs_selector as u64,
            VMCS_16BIT_GUEST_GS_SELECTOR => v.guest_gs_selector as u64,
            VMCS_16BIT_GUEST_LDTR_SELECTOR => v.guest_ldtr_selector as u64,
            VMCS_16BIT_GUEST_TR_SELECTOR => v.guest_tr_selector as u64,
            // 16-bit host selectors.
            VMCS_16BIT_HOST_ES_SELECTOR => v.host_es_selector as u64,
            VMCS_16BIT_HOST_CS_SELECTOR => v.host_cs_selector as u64,
            VMCS_16BIT_HOST_SS_SELECTOR => v.host_ss_selector as u64,
            VMCS_16BIT_HOST_DS_SELECTOR => v.host_ds_selector as u64,
            VMCS_16BIT_HOST_FS_SELECTOR => v.host_fs_selector as u64,
            VMCS_16BIT_HOST_GS_SELECTOR => v.host_gs_selector as u64,
            VMCS_16BIT_HOST_TR_SELECTOR => v.host_tr_selector as u64,
            // 64-bit control / guest / host.
            VMCS_64BIT_GUEST_LINK_POINTER => v.vmcs_link_pointer,
            VMCS_64BIT_CONTROL_IO_BITMAP_A => v.io_bitmap_addr[0],
            VMCS_64BIT_CONTROL_IO_BITMAP_B => v.io_bitmap_addr[1],
            VMCS_64BIT_CONTROL_MSR_BITMAPS => v.msr_bitmap_addr,
            VMCS_64BIT_CONTROL_TSC_OFFSET => v.tsc_offset,
            VMCS_64BIT_GUEST_IA32_EFER => v.guest_ia32_efer,
            VMCS_64BIT_GUEST_IA32_PAT => v.guest_ia32_pat,
            VMCS_64BIT_HOST_IA32_PAT => v.host_ia32_pat,
            VMCS_64BIT_HOST_IA32_EFER => v.host_ia32_efer,
            // 32-bit control fields.
            VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS => v.pin_based_ctls as u64,
            VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS => v.proc_based_ctls as u64,
            VMCS_32BIT_CONTROL_EXECUTION_BITMAP => v.exception_bitmap as u64,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MASK => v.vm_pf_mask as u64,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MATCH => v.vm_pf_match as u64,
            VMCS_32BIT_CONTROL_CR3_TARGET_COUNT => v.vm_cr3_target_cnt as u64,
            VMCS_CR3_TARGET0 => v.vm_cr3_target_value[0],
            VMCS_CR3_TARGET1 => v.vm_cr3_target_value[1],
            VMCS_CR3_TARGET2 => v.vm_cr3_target_value[2],
            VMCS_CR3_TARGET3 => v.vm_cr3_target_value[3],
            VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS => v.secondary_proc_based_ctls as u64,
            VMCS_32BIT_CONTROL_VMEXIT_CONTROLS => v.vm_exit_ctls as u64,
            VMCS_32BIT_CONTROL_VMENTRY_CONTROLS => v.vm_entry_ctls as u64,
            VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO => v.vm_entry_intr_info as u64,
            VMCS_32BIT_CONTROL_VMENTRY_EXCEPTION_ERR_CODE => v.vm_entry_exception_error_code as u64,
            VMCS_32BIT_CONTROL_VMENTRY_INSTRUCTION_LENGTH => v.vm_entry_instruction_length as u64,
            // 32-bit read-only exit data.
            VMCS_32BIT_INSTRUCTION_ERROR => v.vm_instruction_error as u64,
            VMCS_32BIT_VMEXIT_REASON => v.exit_reason as u64,
            VMCS_32BIT_VMEXIT_INTERRUPTION_INFO => v.exit_intr_info as u64,
            VMCS_32BIT_VMEXIT_INTERRUPTION_ERR_CODE => v.exit_intr_error_code as u64,
            VMCS_32BIT_IDT_VECTORING_INFO => v.idt_vectoring_info as u64,
            VMCS_32BIT_IDT_VECTORING_ERR_CODE => v.idt_vectoring_error_code as u64,
            VMCS_32BIT_VMEXIT_INSTRUCTION_LENGTH => v.exit_instruction_length as u64,
            VMCS_32BIT_VMEXIT_INSTRUCTION_INFO => v.exit_instruction_info as u64,
            VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE => v.vmx_preemption_timer_value as u64,
            // 32-bit guest state.
            VMCS_32BIT_GUEST_ES_LIMIT => v.guest_es_limit as u64,
            VMCS_32BIT_GUEST_CS_LIMIT => v.guest_cs_limit as u64,
            VMCS_32BIT_GUEST_SS_LIMIT => v.guest_ss_limit as u64,
            VMCS_32BIT_GUEST_DS_LIMIT => v.guest_ds_limit as u64,
            VMCS_32BIT_GUEST_FS_LIMIT => v.guest_fs_limit as u64,
            VMCS_32BIT_GUEST_GS_LIMIT => v.guest_gs_limit as u64,
            VMCS_32BIT_GUEST_LDTR_LIMIT => v.guest_ldtr_limit as u64,
            VMCS_32BIT_GUEST_TR_LIMIT => v.guest_tr_limit as u64,
            VMCS_32BIT_GUEST_GDTR_LIMIT => v.guest_gdtr_limit as u64,
            VMCS_32BIT_GUEST_IDTR_LIMIT => v.guest_idtr_limit as u64,
            VMCS_32BIT_GUEST_ES_ACCESS_RIGHTS => v.guest_es_ar as u64,
            VMCS_32BIT_GUEST_CS_ACCESS_RIGHTS => v.guest_cs_ar as u64,
            VMCS_32BIT_GUEST_SS_ACCESS_RIGHTS => v.guest_ss_ar as u64,
            VMCS_32BIT_GUEST_DS_ACCESS_RIGHTS => v.guest_ds_ar as u64,
            VMCS_32BIT_GUEST_FS_ACCESS_RIGHTS => v.guest_fs_ar as u64,
            VMCS_32BIT_GUEST_GS_ACCESS_RIGHTS => v.guest_gs_ar as u64,
            VMCS_32BIT_GUEST_LDTR_ACCESS_RIGHTS => v.guest_ldtr_ar as u64,
            VMCS_32BIT_GUEST_TR_ACCESS_RIGHTS => v.guest_tr_ar as u64,
            VMCS_32BIT_GUEST_INTERRUPTIBILITY_STATE => v.guest_interruptibility_state as u64,
            VMCS_32BIT_GUEST_ACTIVITY_STATE => v.guest_activity_state as u64,
            VMCS_32BIT_GUEST_IA32_SYSENTER_CS_MSR => v.guest_ia32_sysenter_cs as u64,
            // 32-bit host state.
            VMCS_32BIT_HOST_IA32_SYSENTER_CS_MSR => v.host_sysenter_cs as u64,
            // Natural-width control.
            VMCS_CONTROL_CR0_GUEST_HOST_MASK => v.cr0_guest_host_mask,
            VMCS_CONTROL_CR4_GUEST_HOST_MASK => v.cr4_guest_host_mask,
            VMCS_CONTROL_CR0_READ_SHADOW => v.cr0_read_shadow,
            VMCS_CONTROL_CR4_READ_SHADOW => v.cr4_read_shadow,
            // Natural-width read-only.
            VMCS_VMEXIT_QUALIFICATION => v.exit_qualification,
            VMCS_VMEXIT_GUEST_LINEAR_ADDR => v.guest_linear_addr,
            // Natural-width guest state.
            VMCS_GUEST_CR0 => v.guest_cr0,
            VMCS_GUEST_CR3 => v.guest_cr3,
            VMCS_GUEST_CR4 => v.guest_cr4,
            VMCS_GUEST_ES_BASE => v.guest_es_base,
            VMCS_GUEST_CS_BASE => v.guest_cs_base,
            VMCS_GUEST_SS_BASE => v.guest_ss_base,
            VMCS_GUEST_DS_BASE => v.guest_ds_base,
            VMCS_GUEST_FS_BASE => v.guest_fs_base,
            VMCS_GUEST_GS_BASE => v.guest_gs_base,
            VMCS_GUEST_LDTR_BASE => v.guest_ldtr_base,
            VMCS_GUEST_TR_BASE => v.guest_tr_base,
            VMCS_GUEST_GDTR_BASE => v.guest_gdtr_base,
            VMCS_GUEST_IDTR_BASE => v.guest_idtr_base,
            VMCS_GUEST_DR7 => v.guest_dr7,
            VMCS_GUEST_RSP => v.guest_rsp,
            VMCS_GUEST_RIP => v.guest_rip,
            VMCS_GUEST_RFLAGS => v.guest_rflags,
            VMCS_GUEST_IA32_SYSENTER_ESP_MSR => v.guest_ia32_sysenter_esp,
            VMCS_GUEST_IA32_SYSENTER_EIP_MSR => v.guest_ia32_sysenter_eip,
            // Natural-width host state.
            VMCS_HOST_CR0 => v.host_cr0,
            VMCS_HOST_CR3 => v.host_cr3,
            VMCS_HOST_CR4 => v.host_cr4,
            VMCS_HOST_FS_BASE => v.host_fs_base,
            VMCS_HOST_GS_BASE => v.host_gs_base,
            VMCS_HOST_TR_BASE => v.host_tr_base,
            VMCS_HOST_GDTR_BASE => v.host_gdtr_base,
            VMCS_HOST_IDTR_BASE => v.host_idtr_base,
            VMCS_HOST_IA32_SYSENTER_ESP_MSR => v.host_sysenter_esp,
            VMCS_HOST_IA32_SYSENTER_EIP_MSR => v.host_sysenter_eip,
            VMCS_HOST_RSP => v.host_rsp,
            VMCS_HOST_RIP => v.host_rip,
            _ => return None,
        })
    }

    /// Dispatch VMWRITE encoding → named field. Returns `false` if the
    /// encoding isn't supported (callers `VMfail` in that case).
    fn vmcs_write_field(&mut self, encoding: u32, value: u64) -> bool {
        let v = &mut self.vmcs;
        match encoding {
            VMCS_16BIT_GUEST_ES_SELECTOR => v.guest_es_selector = value as u16,
            VMCS_16BIT_GUEST_CS_SELECTOR => v.guest_cs_selector = value as u16,
            VMCS_16BIT_GUEST_SS_SELECTOR => v.guest_ss_selector = value as u16,
            VMCS_16BIT_GUEST_DS_SELECTOR => v.guest_ds_selector = value as u16,
            VMCS_16BIT_GUEST_FS_SELECTOR => v.guest_fs_selector = value as u16,
            VMCS_16BIT_GUEST_GS_SELECTOR => v.guest_gs_selector = value as u16,
            VMCS_16BIT_GUEST_LDTR_SELECTOR => v.guest_ldtr_selector = value as u16,
            VMCS_16BIT_GUEST_TR_SELECTOR => v.guest_tr_selector = value as u16,
            VMCS_16BIT_HOST_ES_SELECTOR => v.host_es_selector = value as u16,
            VMCS_16BIT_HOST_CS_SELECTOR => v.host_cs_selector = value as u16,
            VMCS_16BIT_HOST_SS_SELECTOR => v.host_ss_selector = value as u16,
            VMCS_16BIT_HOST_DS_SELECTOR => v.host_ds_selector = value as u16,
            VMCS_16BIT_HOST_FS_SELECTOR => v.host_fs_selector = value as u16,
            VMCS_16BIT_HOST_GS_SELECTOR => v.host_gs_selector = value as u16,
            VMCS_16BIT_HOST_TR_SELECTOR => v.host_tr_selector = value as u16,
            VMCS_64BIT_GUEST_LINK_POINTER => v.vmcs_link_pointer = value,
            VMCS_64BIT_CONTROL_IO_BITMAP_A => v.io_bitmap_addr[0] = value,
            VMCS_64BIT_CONTROL_IO_BITMAP_B => v.io_bitmap_addr[1] = value,
            VMCS_64BIT_CONTROL_MSR_BITMAPS => v.msr_bitmap_addr = value,
            VMCS_64BIT_CONTROL_TSC_OFFSET => v.tsc_offset = value,
            VMCS_64BIT_GUEST_IA32_EFER => v.guest_ia32_efer = value,
            VMCS_64BIT_GUEST_IA32_PAT => v.guest_ia32_pat = value,
            VMCS_64BIT_HOST_IA32_PAT => v.host_ia32_pat = value,
            VMCS_64BIT_HOST_IA32_EFER => v.host_ia32_efer = value,
            VMCS_32BIT_CONTROL_PIN_BASED_EXEC_CONTROLS => v.pin_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_PROCESSOR_BASED_VMEXEC_CONTROLS => v.proc_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_EXECUTION_BITMAP => v.exception_bitmap = value as u32,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MASK => v.vm_pf_mask = value as u32,
            VMCS_32BIT_CONTROL_PAGE_FAULT_ERR_CODE_MATCH => v.vm_pf_match = value as u32,
            VMCS_32BIT_CONTROL_CR3_TARGET_COUNT => v.vm_cr3_target_cnt = value as u32,
            VMCS_CR3_TARGET0 => v.vm_cr3_target_value[0] = value,
            VMCS_CR3_TARGET1 => v.vm_cr3_target_value[1] = value,
            VMCS_CR3_TARGET2 => v.vm_cr3_target_value[2] = value,
            VMCS_CR3_TARGET3 => v.vm_cr3_target_value[3] = value,
            VMCS_32BIT_CONTROL_SECONDARY_VMEXEC_CONTROLS => v.secondary_proc_based_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMEXIT_CONTROLS => v.vm_exit_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_CONTROLS => v.vm_entry_ctls = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_INTERRUPTION_INFO => v.vm_entry_intr_info = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_EXCEPTION_ERR_CODE => v.vm_entry_exception_error_code = value as u32,
            VMCS_32BIT_CONTROL_VMENTRY_INSTRUCTION_LENGTH => v.vm_entry_instruction_length = value as u32,
            // Read-only VMCS exit-data fields: Bochs VMwriteReadOnlyVmcsComponent
            // returns false. Being lenient would let VMMs that pre-zero these
            // keep going, but Bochs is strict.
            VMCS_32BIT_INSTRUCTION_ERROR
            | VMCS_32BIT_VMEXIT_REASON
            | VMCS_32BIT_VMEXIT_INTERRUPTION_INFO
            | VMCS_32BIT_VMEXIT_INTERRUPTION_ERR_CODE
            | VMCS_32BIT_IDT_VECTORING_INFO
            | VMCS_32BIT_IDT_VECTORING_ERR_CODE
            | VMCS_32BIT_VMEXIT_INSTRUCTION_LENGTH
            | VMCS_32BIT_VMEXIT_INSTRUCTION_INFO
            | VMCS_VMEXIT_QUALIFICATION
            | VMCS_VMEXIT_GUEST_LINEAR_ADDR => return false,
            VMCS_32BIT_GUEST_PREEMPTION_TIMER_VALUE => v.vmx_preemption_timer_value = value as u32,
            VMCS_32BIT_GUEST_ES_LIMIT => v.guest_es_limit = value as u32,
            VMCS_32BIT_GUEST_CS_LIMIT => v.guest_cs_limit = value as u32,
            VMCS_32BIT_GUEST_SS_LIMIT => v.guest_ss_limit = value as u32,
            VMCS_32BIT_GUEST_DS_LIMIT => v.guest_ds_limit = value as u32,
            VMCS_32BIT_GUEST_FS_LIMIT => v.guest_fs_limit = value as u32,
            VMCS_32BIT_GUEST_GS_LIMIT => v.guest_gs_limit = value as u32,
            VMCS_32BIT_GUEST_LDTR_LIMIT => v.guest_ldtr_limit = value as u32,
            VMCS_32BIT_GUEST_TR_LIMIT => v.guest_tr_limit = value as u32,
            VMCS_32BIT_GUEST_GDTR_LIMIT => v.guest_gdtr_limit = value as u32,
            VMCS_32BIT_GUEST_IDTR_LIMIT => v.guest_idtr_limit = value as u32,
            VMCS_32BIT_GUEST_ES_ACCESS_RIGHTS => v.guest_es_ar = value as u32,
            VMCS_32BIT_GUEST_CS_ACCESS_RIGHTS => v.guest_cs_ar = value as u32,
            VMCS_32BIT_GUEST_SS_ACCESS_RIGHTS => v.guest_ss_ar = value as u32,
            VMCS_32BIT_GUEST_DS_ACCESS_RIGHTS => v.guest_ds_ar = value as u32,
            VMCS_32BIT_GUEST_FS_ACCESS_RIGHTS => v.guest_fs_ar = value as u32,
            VMCS_32BIT_GUEST_GS_ACCESS_RIGHTS => v.guest_gs_ar = value as u32,
            VMCS_32BIT_GUEST_LDTR_ACCESS_RIGHTS => v.guest_ldtr_ar = value as u32,
            VMCS_32BIT_GUEST_TR_ACCESS_RIGHTS => v.guest_tr_ar = value as u32,
            VMCS_32BIT_GUEST_INTERRUPTIBILITY_STATE => v.guest_interruptibility_state = value as u32,
            VMCS_32BIT_GUEST_ACTIVITY_STATE => v.guest_activity_state = value as u32,
            VMCS_32BIT_GUEST_IA32_SYSENTER_CS_MSR => v.guest_ia32_sysenter_cs = value as u32,
            VMCS_32BIT_HOST_IA32_SYSENTER_CS_MSR => v.host_sysenter_cs = value as u32,
            VMCS_CONTROL_CR0_GUEST_HOST_MASK => v.cr0_guest_host_mask = value,
            VMCS_CONTROL_CR4_GUEST_HOST_MASK => v.cr4_guest_host_mask = value,
            VMCS_CONTROL_CR0_READ_SHADOW => v.cr0_read_shadow = value,
            VMCS_CONTROL_CR4_READ_SHADOW => v.cr4_read_shadow = value,
            VMCS_GUEST_CR0 => v.guest_cr0 = value,
            VMCS_GUEST_CR3 => v.guest_cr3 = value,
            VMCS_GUEST_CR4 => v.guest_cr4 = value,
            VMCS_GUEST_ES_BASE => v.guest_es_base = value,
            VMCS_GUEST_CS_BASE => v.guest_cs_base = value,
            VMCS_GUEST_SS_BASE => v.guest_ss_base = value,
            VMCS_GUEST_DS_BASE => v.guest_ds_base = value,
            VMCS_GUEST_FS_BASE => v.guest_fs_base = value,
            VMCS_GUEST_GS_BASE => v.guest_gs_base = value,
            VMCS_GUEST_LDTR_BASE => v.guest_ldtr_base = value,
            VMCS_GUEST_TR_BASE => v.guest_tr_base = value,
            VMCS_GUEST_GDTR_BASE => v.guest_gdtr_base = value,
            VMCS_GUEST_IDTR_BASE => v.guest_idtr_base = value,
            VMCS_GUEST_DR7 => v.guest_dr7 = value,
            VMCS_GUEST_RSP => v.guest_rsp = value,
            VMCS_GUEST_RIP => v.guest_rip = value,
            VMCS_GUEST_RFLAGS => v.guest_rflags = value,
            VMCS_GUEST_IA32_SYSENTER_ESP_MSR => v.guest_ia32_sysenter_esp = value,
            VMCS_GUEST_IA32_SYSENTER_EIP_MSR => v.guest_ia32_sysenter_eip = value,
            VMCS_HOST_CR0 => v.host_cr0 = value,
            VMCS_HOST_CR3 => v.host_cr3 = value,
            VMCS_HOST_CR4 => v.host_cr4 = value,
            VMCS_HOST_FS_BASE => v.host_fs_base = value,
            VMCS_HOST_GS_BASE => v.host_gs_base = value,
            VMCS_HOST_TR_BASE => v.host_tr_base = value,
            VMCS_HOST_GDTR_BASE => v.host_gdtr_base = value,
            VMCS_HOST_IDTR_BASE => v.host_idtr_base = value,
            VMCS_HOST_IA32_SYSENTER_ESP_MSR => v.host_sysenter_esp = value,
            VMCS_HOST_IA32_SYSENTER_EIP_MSR => v.host_sysenter_eip = value,
            VMCS_HOST_RSP => v.host_rsp = value,
            VMCS_HOST_RIP => v.host_rip = value,
            _ => return false,
        }
        true
    }

    // Top-level VMREAD handlers (32-bit and 64-bit operand size).
    // Bochs vmx.cc BX_CPU_C::VMREAD_EdGd / VMREAD_EqGq.

    pub(super) fn vmread_ed_gd(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr32(instr.src() as usize);
        let val = self.vmread_impl(enc)?;
        if instr.mod_c0() {
            self.set_gpr32(instr.dst() as usize, val as u32);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.write_virtual_dword(seg, eaddr as u32, val as u32)?;
        }
        Ok(())
    }

    pub(super) fn vmread_eq_gq(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr64(instr.src() as usize) as u32;
        let val = self.vmread_impl(enc)?;
        if instr.mod_c0() {
            self.set_gpr64(instr.dst() as usize, val);
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.write_virtual_qword_64(seg, eaddr, val)?;
        }
        Ok(())
    }

    pub(super) fn vmwrite_gd_ed(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr32(instr.dst() as usize);
        let src = if instr.mod_c0() {
            self.get_gpr32(instr.src() as usize) as u64
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.read_virtual_dword(seg, eaddr as u32)? as u64
        };
        self.vmwrite_impl(enc, src)
    }

    pub(super) fn vmwrite_gq_eq(&mut self, instr: &Instruction) -> Result<()> {
        let enc = self.get_gpr64(instr.dst() as usize) as u32;
        let src = if instr.mod_c0() {
            self.get_gpr64(instr.src() as usize)
        } else {
            let seg = BxSegregs::from(instr.seg());
            let eaddr = self.resolve_addr(instr);
            self.read_virtual_qword_64(seg, eaddr)?
        };
        self.vmwrite_impl(enc, src)
    }

    // =========================================================================
    // VMLAUNCH / VMRESUME — enter VMX non-root (guest) operation.
    //
    // Bochs cpu/vmx.cc BX_CPU_C::VMLAUNCH + VMRESUME share a handler; the
    // launch-vs-resume bit controls which error code the preconditions
    // report. Full VM-entry validation (Bochs VMenterLoadCheckVmControls /
    // HostState / GuestState) would add ~1500 LOC of field-by-field sanity;
    // in this pass we perform the architecturally-required launch-state
    // check, then do a straightforward host/guest state swap covering the
    // control registers, instruction pointer / stack pointer / RFLAGS and
    // EFER / PAT MSRs. Segment descriptor reload, interruptibility state,
    // and the ~60-field host-state-field integrity tests will grow in-place
    // as real VMMs exercise them.
    // =========================================================================

    pub(super) fn vmlaunch(&mut self, instr: &Instruction) -> Result<()> {
        self.vmlaunch_vmresume(instr, false)
    }

    pub(super) fn vmresume(&mut self, instr: &Instruction) -> Result<()> {
        self.vmlaunch_vmresume(instr, true)
    }

    fn vmlaunch_vmresume(&mut self, _instr: &Instruction, is_resume: bool) -> Result<()> {
        if !self.in_vmx || !self.protected_mode() || self.long_compat_mode() {
            return self.exception(Exception::Ud, 0);
        }
        if self.in_vmx_guest {
            // Bochs: VM_exit with reason VMLAUNCH / VMRESUME (Session 6
            // intercept wiring handles the full VMEXIT path).
            return self.exception(Exception::Gp, 0);
        }
        if self.cs_rpl() != 0 {
            return self.exception(Exception::Gp, 0);
        }
        if self.vmcsptr == super::vmcs::BX_INVALID_VMCSPTR {
            self.vmfail_invalid();
            return Ok(());
        }

        // Launch-state gate — Bochs VMXERR_VMLAUNCH_NON_CLEAR_VMCS /
        // VMXERR_VMRESUME_NON_LAUNCHED_VMCS.
        if is_resume && !self.vmcs.launched {
            self.vmfail(VmxErr::VmresumeNonLaunchedVmcs);
            return Ok(());
        }
        if !is_resume && self.vmcs.launched {
            self.vmfail(VmxErr::VmlaunchNonClearVmcs);
            return Ok(());
        }

        // Save host state from the running CPU. RIP is "the instruction after
        // VMLAUNCH / VMRESUME"; Bochs stashes it so VMEXIT_LOAD_HOST_STATE can
        // jump back. The prefetch queue already advanced past this insn, so
        // `self.rip()` points at the next one.
        self.vmcs.host_cr0 = self.cr0.get32() as u64;
        self.vmcs.host_cr3 = self.cr3;
        self.vmcs.host_cr4 = self.cr4.get() as u64;
        self.vmcs.host_rsp = self.rsp();
        self.vmcs.host_rip = self.rip();
        self.vmcs.host_ia32_efer = self.efer.get32() as u64;
        self.vmcs.host_ia32_pat = self.msr.pat.U64();

        // Load guest state into the running CPU.
        self.cr0.set32(self.vmcs.guest_cr0 as u32);
        self.cr3 = self.vmcs.guest_cr3;
        self.cr4.set_val(self.vmcs.guest_cr4);
        self.set_rsp(self.vmcs.guest_rsp);
        self.set_rip(self.vmcs.guest_rip);
        self.write_eflags(self.vmcs.guest_rflags as u32, 0x003FFFFF);
        self.efer.set32(self.vmcs.guest_ia32_efer as u32);
        self.msr.pat.set_U64(self.vmcs.guest_ia32_pat);

        self.vmcs.launched = true;
        self.in_vmx_guest = true;
        // Bochs vmx.cc step 6: arm the preemption timer when the pin-based
        // control is set. Reads vmx_preemption_timer_value from the VMCS.
        self.vmenter_arm_preemption_timer();
        self.vmsucceed();
        // Guest now runs from the loaded RIP — the CPU loop picks up the new
        // prefetch target after this instruction returns.
        self.invalidate_prefetch_q();
        Ok(())
    }

    // =========================================================================
    // VM-exit — return to VMX root with reason + qualification.
    //
    // Bochs' VMexit() is the single entry for every exit reason. It saves
    // guest state into the VMCS, restores host state, clears in_vmx_guest,
    // and returns the CPU loop to the host instruction stream. This is the
    // symmetric counterpart to vmlaunch_vmresume above.
    // =========================================================================

    pub(super) fn vmx_vmexit(
        &mut self,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<()> {
        if !self.in_vmx_guest {
            return Ok(());
        }

        // Bochs vmx.cc VMexit (step 1): snapshot the preemption timer back
        // into the VMCS when STORE_VMX_PREEMPTION_TIMER is set, then disarm.
        self.vmexit_disarm_preemption_timer();

        // Save guest state from the running CPU.
        self.vmcs.guest_cr0 = self.cr0.get32() as u64;
        self.vmcs.guest_cr3 = self.cr3;
        self.vmcs.guest_cr4 = self.cr4.get() as u64;
        self.vmcs.guest_rsp = self.rsp();
        self.vmcs.guest_rip = self.rip();
        self.vmcs.guest_rflags = self.read_eflags() as u64;
        self.vmcs.guest_ia32_efer = self.efer.get32() as u64;
        self.vmcs.guest_ia32_pat = self.msr.pat.U64();

        // Record the exit info the host reads after re-entry.
        self.vmcs.exit_reason = reason as u32;
        self.vmcs.exit_qualification = qualification;

        // Load host state.
        self.cr0.set32(self.vmcs.host_cr0 as u32);
        self.cr3 = self.vmcs.host_cr3;
        self.cr4.set_val(self.vmcs.host_cr4);
        self.set_rsp(self.vmcs.host_rsp);
        self.set_rip(self.vmcs.host_rip);
        self.efer.set32(self.vmcs.host_ia32_efer as u32);
        self.msr.pat.set_U64(self.vmcs.host_ia32_pat);

        self.in_vmx_guest = false;
        self.invalidate_prefetch_q();
        Ok(())
    }

    // =========================================================================
    // Helpers used by VMXON.
    // =========================================================================

    /// Read the VMCS revision ID (first 4 bytes of a VMCS / VMXON region) from
    /// guest-physical memory. Bochs vmx.cc VMXReadRevisionID.
    fn vmx_read_revision_id(&mut self, paddr: u64) -> u32 {
        self.mem_read_dword(paddr)
    }

    /// Bochs cpu.h long_compat_mode — 32-bit compatibility sub-mode of long mode.
    #[inline]
    fn long_compat_mode(&self) -> bool {
        self.long_mode() && !self.long64_mode()
    }

    /// Is A20 masking enabled? Bochs' `BX_GET_ENABLE_A20()` macro pokes
    /// `bx_pc_system.enable_a20`. Our equivalent is `self.a20_mask == !0` —
    /// the mask covers the full address space when A20 is enabled.
    #[inline]
    fn a20_enabled(&self) -> bool {
        // A20 masks bit 20 to 0 when disabled → `a20_mask` lacks bit 20.
        (self.a20_mask & (1u64 << 20)) != 0
    }

    // =========================================================================
    // VM-exit intercept predicates — Bochs cpu/vmx.cc + vmexit.cc.
    //
    // Each `vmexit_check_*` method returns `Ok(true)` if the running guest
    // should trigger a VM-exit for this instruction (and performs the exit
    // as a side effect), `Ok(false)` if the guest keeps executing natively.
    //
    // The caller pattern for wrapped handlers is:
    //
    //     if self.in_vmx_guest && self.vmexit_check_hlt()? { return Ok(()); }
    //
    // Unconditional-intercept opcodes (CPUID, RSM, XSETBV, GETSEC, INVD, all
    // VMX instructions) use `vmexit_unconditional` — there's no control bit
    // to test, the exit is mandated by the SDM.
    // =========================================================================

    #[inline]
    fn proc_based_ctls1(&self) -> u32 {
        self.vmcs.proc_based_ctls
    }

    #[inline]
    fn proc_based_ctls2(&self) -> u32 {
        // The secondary controls are only honoured when the primary
        // ACTIVATE_SECONDARY_CONTROLS bit is set (Bochs gate).
        if self.vmcs.proc_based_ctls & VMX_VM_EXEC_CTRL1_SECONDARY_CONTROLS != 0 {
            self.vmcs.secondary_proc_based_ctls
        } else {
            0
        }
    }

    #[inline]
    fn pin_based_ctls(&self) -> u32 {
        self.vmcs.pin_based_ctls
    }

    /// External-interrupt pin-based VMEXIT — Bochs vmexit.cc
    /// `VMexit_ExtInterrupt`. Called from the event-delivery path BEFORE the
    /// PIC/LAPIC is acknowledged. With `INTA_ON_VMEXIT` cleared, the
    /// interrupt stays pending in the controller; with it set we let the
    /// caller acknowledge first and route through `vmexit_check_event_intr`
    /// below so the vector ends up in `exit_intr_info`.
    ///
    /// Returns `Ok(true)` when the no-ack VMEXIT path was taken.
    pub(super) fn vmexit_check_ext_intr_no_ack(&mut self) -> Result<bool> {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_EXTERNAL_INTERRUPT_VMEXIT == 0 {
            return Ok(false);
        }
        if self.vmcs.vm_exit_ctls & VMX_VMEXIT_CTRL1_INTA_ON_VMEXIT != 0 {
            // Defer to the post-ack path; caller will acknowledge then
            // vmexit_check_event_intr.
            return Ok(false);
        }
        // No INTA on exit: interruption-info field is invalid.
        self.vmcs.exit_intr_info = 0;
        self.vmx_vmexit(VmxVmexitReason::ExternalInterrupt, 0)?;
        Ok(true)
    }

    /// Pin-based NMI VMEXIT — Bochs vmexit.cc `VMexit_Event` with
    /// `type == BX_NMI`. Called immediately before delivering NMI to the
    /// guest. Records interruption info `vector=2 | type=NMI<<8 | valid<<31`.
    pub(super) fn vmexit_check_nmi(&mut self) -> Result<bool> {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_NMI_EXITING == 0 {
            return Ok(false);
        }
        // intr_info: vector=2, type=NMI(2)<<8, valid bit 31.
        self.vmcs.exit_intr_info = 2 | (2 << 8) | (1u32 << 31);
        self.vmcs.exit_intr_error_code = 0;
        self.vmx_vmexit(VmxVmexitReason::ExceptionNmi, 0)?;
        Ok(true)
    }

    /// Arm the VMX preemption timer at VM-entry — Bochs vmx.cc:3520.
    /// Loads `vmx_preemption_timer_value` from the cached VMCS and stores an
    /// absolute deadline in `icount` units. Bochs scales by `IA32_VMX_MISC[4:0]`
    /// which we leave at 0 (no extra division), matching rusty_box's TSC-as-
    /// icount convention.
    pub(super) fn vmenter_arm_preemption_timer(&mut self) {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_VMX_PREEMPTION_TIMER_VMEXIT == 0 {
            self.vmx_preemption_timer_active = false;
            return;
        }
        let val = u64::from(self.vmcs.vmx_preemption_timer_value);
        self.vmx_preemption_timer_active = true;
        self.vmx_preemption_timer_deadline = self.icount.wrapping_add(val);
        // A zero-valued timer signals the expiry event immediately; the
        // event-delivery path will exit on the next async-event boundary.
        if val == 0 {
            self.async_event |= 1;
        }
    }

    /// Disarm the VMX preemption timer at VM-exit — Bochs vmx.cc:2818. When
    /// `STORE_VMX_PREEMPTION_TIMER` is set the remaining value is snapshotted
    /// back into the guest VMCS field. Bochs' value is `lapic->read_vmx_
    /// preemption_timer()`; here we approximate as `deadline - icount` (zero
    /// if expired).
    pub(super) fn vmexit_disarm_preemption_timer(&mut self) {
        if !self.vmx_preemption_timer_active {
            return;
        }
        // STORE control bit per VMX_VMEXIT_CTRL1 (bit 22); plumbed as a raw
        // mask to avoid pulling another import for one bit.
        const VMX_VMEXIT_CTRL1_STORE_PREEMPTION_TIMER: u32 = 1 << 22;
        if self.vmcs.vm_exit_ctls & VMX_VMEXIT_CTRL1_STORE_PREEMPTION_TIMER != 0 {
            let remaining = self
                .vmx_preemption_timer_deadline
                .saturating_sub(self.icount);
            self.vmcs.vmx_preemption_timer_value = remaining as u32;
        }
        self.vmx_preemption_timer_active = false;
    }

    /// Preemption-timer expiry check — Bochs event.cc
    /// `BX_EVENT_VMX_PREEMPTION_TIMER_EXPIRED` branch. Called from
    /// `handle_async_event`; exits with reason
    /// `VMX_VMEXIT_VMX_PREEMPTION_TIMER_EXPIRED` when the deadline is
    /// reached.
    pub(super) fn vmexit_check_preemption_timer(&mut self) -> Result<bool> {
        if !self.vmx_preemption_timer_active {
            return Ok(false);
        }
        if self.icount < self.vmx_preemption_timer_deadline {
            return Ok(false);
        }
        self.vmx_preemption_timer_active = false;
        self.vmx_vmexit(VmxVmexitReason::VmxPreemptionTimerExpired, 0)?;
        Ok(true)
    }

    /// NMI-window VMEXIT — Bochs event.cc `BX_EVENT_VMX_VIRTUAL_NMI` branch.
    /// When `NMI_WINDOW_EXITING` is set in the primary processor controls,
    /// any instruction boundary at which virtual-NMI blocking is clear
    /// triggers a VMEXIT. The caller must verify NMI is not currently
    /// blocked before invoking this predicate.
    pub(super) fn vmexit_check_nmi_window(&mut self) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_NMI_WINDOW_EXITING == 0 {
            return Ok(false);
        }
        self.vmx_vmexit(VmxVmexitReason::NmiWindow, 0)?;
        Ok(true)
    }

    /// Interrupt-window VMEXIT — Bochs event.cc
    /// `BX_EVENT_VMX_INTERRUPT_WINDOW_EXITING` branch. With
    /// `INTERRUPT_WINDOW_VMEXIT` set, VMEXIT fires at any boundary where
    /// `RFLAGS.IF=1` and external-interrupt inhibition is clear. Caller
    /// guarantees the inhibit/IF preconditions.
    pub(super) fn vmexit_check_interrupt_window(&mut self) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_INTERRUPT_WINDOW_VMEXIT == 0 {
            return Ok(false);
        }
        self.vmx_vmexit(VmxVmexitReason::InterruptWindow, 0)?;
        Ok(true)
    }

    /// External-interrupt VMEXIT after the controller has been acknowledged
    /// — Bochs vmexit.cc `VMexit_Event` with `type == BX_EXTERNAL_INTERRUPT`.
    /// Records the acknowledged vector in `exit_intr_info` so the host can
    /// re-deliver it.
    pub(super) fn vmexit_check_event_intr(&mut self, vector: u8) -> Result<bool> {
        if self.pin_based_ctls() & VMX_PIN_BASED_VMEXEC_CTRL_EXTERNAL_INTERRUPT_VMEXIT == 0 {
            return Ok(false);
        }
        // type=ExternalInterrupt(0), vector, valid bit 31.
        self.vmcs.exit_intr_info = u32::from(vector) | (1u32 << 31);
        self.vmcs.exit_intr_error_code = 0;
        self.vmx_vmexit(VmxVmexitReason::ExternalInterrupt, 0)?;
        Ok(true)
    }

    /// Trigger an unconditional VM-exit (caller has already verified
    /// `self.in_vmx_guest`). Returns `Ok(true)` so call sites can chain.
    pub(super) fn vmexit_unconditional(
        &mut self,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<bool> {
        self.vmx_vmexit(reason, qualification)?;
        Ok(true)
    }

    /// Conditional VM-exit: `mask` is the control bit(s) in `ctls`; if any
    /// are set, trigger `reason` with `qualification` and return Ok(true).
    pub(super) fn vmexit_if_ctls_set(
        &mut self,
        ctls: u32,
        mask: u32,
        reason: VmxVmexitReason,
        qualification: u64,
    ) -> Result<bool> {
        if ctls & mask != 0 {
            self.vmx_vmexit(reason, qualification)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn vmexit_check_hlt(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(ctls, VMX_VM_EXEC_CTRL1_HLT_VMEXIT, VmxVmexitReason::Hlt, 0)
    }

    pub(super) fn vmexit_check_invlpg(&mut self, linear_addr: u64) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_INVLPG_VMEXIT,
            VmxVmexitReason::Invlpg,
            linear_addr,
        )
    }

    pub(super) fn vmexit_check_rdtsc(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_RDTSC_VMEXIT,
            VmxVmexitReason::Rdtsc,
            0,
        )
    }

    pub(super) fn vmexit_check_rdtscp(&mut self) -> Result<bool> {
        // RDTSCP exits unconditionally unless secondary RDTSCP bit is set AND
        // the primary RDTSC_VMEXIT bit is clear. Bochs vmexit.cc VMexit_Rdtscp.
        if self.proc_based_ctls2() & VMX_VM_EXEC_CTRL2_RDTSCP == 0 {
            // RDTSCP is disabled in the guest → #UD is raised at decode time;
            // we shouldn't reach here. If we do, fall through to native.
            return Ok(false);
        }
        self.vmexit_check_rdtsc()
    }

    pub(super) fn vmexit_check_rdpmc(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_RDPMC_VMEXIT,
            VmxVmexitReason::Rdpmc,
            0,
        )
    }

    pub(super) fn vmexit_check_monitor(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_MONITOR_VMEXIT,
            VmxVmexitReason::Monitor,
            0,
        )
    }

    pub(super) fn vmexit_check_mwait(&mut self, monitor_armed: bool) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        if ctls & VMX_VM_EXEC_CTRL1_MWAIT_VMEXIT == 0 {
            return Ok(false);
        }
        // Bochs vmexit.cc: qualification[0] = monitor hardware armed.
        let qual = if monitor_armed { 1 } else { 0 };
        self.vmx_vmexit(VmxVmexitReason::Mwait, qual)?;
        Ok(true)
    }

    pub(super) fn vmexit_check_pause(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls1();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL1_PAUSE_VMEXIT,
            VmxVmexitReason::Pause,
            0,
        )
    }

    pub(super) fn vmexit_check_wbinvd(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_WBINVD_VMEXIT,
            VmxVmexitReason::Wbinvd,
            0,
        )
    }

    pub(super) fn vmexit_check_invpcid(&mut self) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_INVPCID,
            VmxVmexitReason::Invpcid,
            0,
        )
    }

    /// MSR bitmap walker — Bochs vmexit.cc VMexit_MSR.
    ///
    /// Layout of the 4 KiB bitmap at `msr_bitmap_addr`:
    /// - Bytes `0x000..0x400` — read bitmap for low MSRs `0x00000000..=0x00001FFF`
    /// - Bytes `0x400..0x800` — read bitmap for high MSRs `0xC0000000..=0xC0001FFF`
    /// - Bytes `0x800..0xC00` — write bitmap for low MSRs
    /// - Bytes `0xC00..0x1000` — write bitmap for high MSRs
    ///
    /// MSR indices outside both ranges always force a VMEXIT.
    fn msr_bitmap_says_vmexit(&mut self, msr: u32, readmsr: bool) -> bool {
        const LO_END: u32 = 0x0000_1FFF;
        const HI_START: u32 = 0xC000_0000;
        const HI_END: u32 = 0xC000_1FFF;

        let bitmap = self.vmcs.msr_bitmap_addr;
        let write_off: u64 = if readmsr { 0 } else { 2048 };

        if msr >= HI_START {
            if msr > HI_END {
                return true;
            }
            let paddr = bitmap
                + u64::from((msr - HI_START) >> 3)
                + 1024
                + write_off;
            let field = self.read_physical_byte(paddr);
            (field & (1 << (msr & 7))) != 0
        } else {
            if msr > LO_END {
                return true;
            }
            let paddr = bitmap + u64::from(msr >> 3) + write_off;
            let field = self.read_physical_byte(paddr);
            (field & (1 << (msr & 7))) != 0
        }
    }

    /// RDMSR intercept — Bochs vmexit.cc VMexit_MSR with `readmsr=true`.
    pub(super) fn vmexit_check_rdmsr(&mut self, msr: u32) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_MSR_BITMAPS == 0 {
            // Without bitmaps every RDMSR exits unconditionally; qualification 0.
            self.vmx_vmexit(VmxVmexitReason::Rdmsr, 0)?;
            return Ok(true);
        }
        if self.msr_bitmap_says_vmexit(msr, true) {
            self.vmx_vmexit(VmxVmexitReason::Rdmsr, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// WRMSR intercept — Bochs vmexit.cc VMexit_MSR with `readmsr=false`.
    pub(super) fn vmexit_check_wrmsr(&mut self, msr: u32) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_MSR_BITMAPS == 0 {
            self.vmx_vmexit(VmxVmexitReason::Wrmsr, 0)?;
            return Ok(true);
        }
        if self.msr_bitmap_says_vmexit(msr, false) {
            self.vmx_vmexit(VmxVmexitReason::Wrmsr, 0)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// LGDT / SGDT / LIDT / SIDT intercept — Bochs protect_ctrl.cc gates each
    /// on `vmexec_ctrls2.DESCRIPTOR_TABLE_VMEXIT()`. Qualification here carries
    /// the resolved displacement / effective address per Bochs vmexit.cc
    /// VMexit_Instruction; the INSTRUCTION_INFO field is left unpopulated until
    /// a VMM needs it.
    pub(super) fn vmexit_check_gdtr_idtr_access(
        &mut self,
        qualification: u64,
    ) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_DESCRIPTOR_TABLE_VMEXIT,
            VmxVmexitReason::GdtrIdtrAccess,
            qualification,
        )
    }

    /// LLDT / SLDT / LTR / STR intercept — same gate as GDTR/IDTR above but
    /// reported as `LdtrTrAccess`.
    pub(super) fn vmexit_check_ldtr_tr_access(
        &mut self,
        qualification: u64,
    ) -> Result<bool> {
        let ctls = self.proc_based_ctls2();
        self.vmexit_if_ctls_set(
            ctls,
            VMX_VM_EXEC_CTRL2_DESCRIPTOR_TABLE_VMEXIT,
            VmxVmexitReason::LdtrTrAccess,
            qualification,
        )
    }

    /// MOV from CR3 intercept — Bochs vmexit.cc VMexit_CR3_Read.
    /// Qualification layout for CR-access VM-exits (Bochs vmexit.cc):
    ///   [3:0]   CR number
    ///   [5:4]   access type: 0 = MOV to CR, 1 = MOV from CR, 2 = CLTS, 3 = LMSW
    ///   [6]     LMSW memory operand flag (not used here)
    ///   [11:8]  source/destination GPR
    ///   [31:16] LMSW source data (cleared for CR access)
    pub(super) fn vmexit_check_cr3_read(&mut self, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR3_READ_VMEXIT == 0 {
            return Ok(false);
        }
        let qual: u64 = 3 | (1 << 4) | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV to CR3 intercept — Bochs vmexit.cc VMexit_CR3_Write. The CR3-target
    /// list provides a fast-path: if the new value matches any enabled target
    /// value, the write is allowed without VMEXIT.
    pub(super) fn vmexit_check_cr3_write(&mut self, val: u64, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR3_WRITE_VMEXIT == 0 {
            return Ok(false);
        }
        let cnt = usize::try_from(self.vmcs.vm_cr3_target_cnt).unwrap_or(0);
        let cnt = cnt.min(self.vmcs.vm_cr3_target_value.len());
        for i in 0..cnt {
            if self.vmcs.vm_cr3_target_value[i] == val {
                return Ok(false);
            }
        }
        let qual: u64 = 3 | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV from CR8 intercept — Bochs vmexit.cc VMexit_CR8_Read. Gated on
    /// `CR8_READ_VMEXIT`; qualification matches the standard CR-access
    /// layout with `CR# = 8`, access type = MOV from CR (1), `gpr` in
    /// `[11:8]`.
    pub(super) fn vmexit_check_cr8_read(&mut self, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR8_READ_VMEXIT == 0 {
            return Ok(false);
        }
        let qual: u64 = 8 | (1 << 4) | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV to CR8 intercept — Bochs vmexit.cc VMexit_CR8_Write. Gated on
    /// `CR8_WRITE_VMEXIT`; qualification has `CR# = 8`, access type = MOV
    /// to CR (0), `gpr` in `[11:8]`.
    pub(super) fn vmexit_check_cr8_write(&mut self, gpr: u8) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_CR8_WRITE_VMEXIT == 0 {
            return Ok(false);
        }
        let qual: u64 = 8 | ((u64::from(gpr) & 0xF) << 8);
        self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
        Ok(true)
    }

    /// MOV to CR0 intercept — Bochs vmexit.cc VMexit_CR0_Write. The guest
    /// cannot touch bits pinned by `cr0_guest_host_mask` (aka `vm_cr0_mask`);
    /// an attempted change triggers a VMEXIT, otherwise the write proceeds
    /// but masked bits keep their hardware value (read shadow merge).
    ///
    /// Returns `(exited, effective_val)`. When `exited` is true the caller
    /// must return `Ok(())` immediately; otherwise it must write
    /// `effective_val` (not the raw `val`) to CR0.
    pub(super) fn vmexit_check_cr0_write(
        &mut self,
        val: u64,
        gpr: u8,
    ) -> Result<(bool, u64)> {
        let mask = self.vmcs.cr0_guest_host_mask;
        let shadow = self.vmcs.cr0_read_shadow;
        if (mask & shadow) != (mask & val) {
            self.vmx_vmexit(VmxVmexitReason::CrAccess, (u64::from(gpr) & 0xF) << 8)?;
            return Ok((true, val));
        }
        // Keep bits set in the mask untouched.
        let cur = u64::from(self.cr0.get32());
        Ok((false, (cur & mask) | (val & !mask)))
    }

    /// MOV to CR4 intercept — Bochs vmexit.cc VMexit_CR4_Write. Same shape
    /// as CR0: VMEXIT when the masked shadow bits change, else merge.
    pub(super) fn vmexit_check_cr4_write(
        &mut self,
        val: u64,
        gpr: u8,
    ) -> Result<(bool, u64)> {
        let mask = self.vmcs.cr4_guest_host_mask;
        let shadow = self.vmcs.cr4_read_shadow;
        if (mask & shadow) != (mask & val) {
            self.vmx_vmexit(
                VmxVmexitReason::CrAccess,
                4 | ((u64::from(gpr) & 0xF) << 8),
            )?;
            return Ok((true, val));
        }
        let cur = self.cr4.get();
        Ok((false, (cur & mask) | (val & !mask)))
    }

    /// CLTS intercept — Bochs vmexit.cc VMexit_CLTS. The TS bit (CR0 bit 3)
    /// is masked: when both the host-mask and read-shadow have TS=1, CLTS
    /// triggers a VMEXIT with `access type = 2`. Independently, if the host
    /// pinned TS=0 in the shadow while masking it, CLTS is suppressed and
    /// CR0.TS is left untouched.
    ///
    /// Returns `(exited, suppress_clear)`:
    /// - `exited`: caller must return `Ok(())` immediately.
    /// - `suppress_clear`: when true (and `exited` is false), the handler
    ///   must skip the CR0.TS clear but otherwise complete normally.
    pub(super) fn vmexit_check_clts(&mut self) -> Result<(bool, bool)> {
        let mask = self.vmcs.cr0_guest_host_mask;
        let shadow = self.vmcs.cr0_read_shadow;
        if (mask & shadow & 0x8) != 0 {
            // Access type 2 (CLTS) << 4. CR# = 0, GPR = 0.
            self.vmx_vmexit(VmxVmexitReason::CrAccess, 2u64 << 4)?;
            return Ok((true, false));
        }
        let suppress = (mask & 0x8) != 0 && (shadow & 0x8) == 0;
        Ok((false, suppress))
    }

    /// LMSW intercept — Bochs vmexit.cc VMexit_LMSW. LMSW touches only the
    /// low 4 bits of CR0; an attempted change to a masked bit (relative to
    /// the read shadow) triggers a VMEXIT. Bit 0 (PE) is one-way: a 0→1
    /// transition is significant only when shadow.PE=0. Bits 1..3 use plain
    /// equality against the masked shadow.
    ///
    /// Returns `(exited, effective_msw)`. When `exited` is false the caller
    /// must build the merged value `(cr0 & mask) | (msw & !mask)` for the
    /// low 4 bits using the returned `effective_msw`.
    pub(super) fn vmexit_check_lmsw(
        &mut self,
        msw: u32,
        is_memory: bool,
        laddr: u64,
    ) -> Result<(bool, u32)> {
        let mask = (self.vmcs.cr0_guest_host_mask as u32) & 0xF;
        let shadow = self.vmcs.cr0_read_shadow as u32;
        let mut vmexit = false;
        if (mask & msw & 0x1) != 0 && (shadow & 0x1) == 0 {
            vmexit = true;
        }
        if (mask & shadow & 0xE) != (mask & msw & 0xE) {
            vmexit = true;
        }
        if vmexit {
            let mut qual: u64 = (3u64 << 4) | (u64::from(msw) << 16);
            if is_memory {
                qual |= 1 << 6;
                self.vmcs.guest_linear_addr = laddr;
            }
            self.vmx_vmexit(VmxVmexitReason::CrAccess, qual)?;
            return Ok((true, msw));
        }
        // Merge: keep masked bits at their CR0 value.
        let cr0_lo = self.cr0.get32() & 0xF;
        let mask_lo = mask & 0xF;
        let merged = (cr0_lo & mask_lo) | (msw & !mask_lo);
        Ok((false, merged & 0xF))
    }

    /// MOV to/from DR intercept — Bochs vmexit.cc VMexit_DR_Access.
    /// Qualification layout for DR-access VM-exits:
    ///   [3:0]   DR number
    ///   [4]     direction: 0 = MOV to DR, 1 = MOV from DR
    ///   [11:8]  source/destination GPR
    pub(super) fn vmexit_check_dr_access(
        &mut self,
        read: bool,
        dr: u8,
        gpr: u8,
    ) -> Result<bool> {
        if self.proc_based_ctls1() & VMX_VM_EXEC_CTRL1_DRX_ACCESS_VMEXIT == 0 {
            return Ok(false);
        }
        let mut qual: u64 = (u64::from(dr) & 0xF) | ((u64::from(gpr) & 0xF) << 8);
        if read {
            qual |= 1 << 4;
        }
        self.vmx_vmexit(VmxVmexitReason::DrAccess, qual)?;
        Ok(true)
    }

    /// Hardware-exception intercept — Bochs vmexit.cc VMexit_Event (the
    /// `BX_HARDWARE_EXCEPTION` branch). Consults `exception_bitmap` for every
    /// vector; for #PF the decision additionally depends on the error-code
    /// mask/match pair. If the guest takes the VMEXIT, the VMCS interruption
    /// info + error code are recorded and the qualification encodes CR2 for
    /// #PF or the masked `debug_trap` for #DB. Returns `Ok(true)` when the
    /// VMEXIT is taken.
    pub(super) fn vmexit_check_exception(
        &mut self,
        vector: u32,
        error_code: u32,
        push_error: bool,
    ) -> Result<bool> {
        // Bochs vmexit.cc VMexit_Event: #PF does `(err & pf_mask) == pf_match`
        // XNOR'd with the bitmap; all other vectors just look up the bit.
        let pf_vector = Exception::Pf as u32;
        let db_vector = Exception::Db as u32;
        let vmexit = if vector == pf_vector {
            let err_match = (error_code & self.vmcs.vm_pf_mask) == self.vmcs.vm_pf_match;
            let bitmap = (self.vmcs.exception_bitmap >> pf_vector) & 1 != 0;
            err_match == bitmap
        } else {
            (self.vmcs.exception_bitmap >> vector) & 1 != 0
        };
        if !vmexit {
            return Ok(false);
        }

        // Qualification per Bochs: CR2 for #PF, masked debug_trap for #DB,
        // 0 otherwise. On #DB, Bochs also clears debug_trap.
        let qualification = if vector == pf_vector {
            self.cr2
        } else if vector == db_vector {
            let q = self.debug_trap & 0x0000_600F;
            self.debug_trap = 0;
            u64::from(q)
        } else {
            0
        };

        // Interruption info layout (Bochs vmexit.cc):
        //   [7:0] vector, [10:8] type (3 = hardware exception), [11] error
        //   code delivered, [31] valid. Bits 12 (NMI-unblock) and 13 (FRED
        //   nested) are not populated yet.
        const BX_HARDWARE_EXCEPTION: u32 = 3;
        let mut intr_info = vector | (BX_HARDWARE_EXCEPTION << 8) | (1 << 31);
        if push_error {
            intr_info |= 1 << 11;
        }
        self.vmcs.exit_intr_info = intr_info;
        self.vmcs.exit_intr_error_code = error_code;

        self.vmx_vmexit(VmxVmexitReason::ExceptionNmi, qualification)?;
        Ok(true)
    }

    /// Task-switch intercept — unconditional in VMX (Bochs vmexit.cc
    /// VMexit_TaskSwitch has no control-bit gate; fires whenever a task
    /// switch is attempted from the guest). Qualification layout matches
    /// Bochs: `tss_selector | (source << 30)`.
    pub(super) fn vmexit_check_task_switch(
        &mut self,
        tss_selector: u16,
        source: u32,
    ) -> Result<bool> {
        let qual = u64::from(tss_selector) | (u64::from(source) << 30);
        self.vmx_vmexit(VmxVmexitReason::TaskSwitch, qual)?;
        Ok(true)
    }

    /// I/O port intercept — Bochs vmx.cc VMexit_IO.
    /// Without I/O bitmaps, any IO_VMEXIT bit triggers exits for every port.
    /// With bitmaps, the per-port bit in io_bitmap_addr decides. Bitmap walk
    /// is deferred; current stub exits-all when IO_VMEXIT is set and bitmaps
    /// are off.
    /// I/O port bitmap walker — Bochs vmexit.cc VMexit_IO. The pair of 4 KiB
    /// bitmaps `io_bitmap_addr[0..1]` covers ports `0x0000..0x7FFF` and
    /// `0x8000..0xFFFF` respectively. A multi-byte access whose port range
    /// straddles the 0x8000 split must consult both bitmaps; access ranges
    /// wrapping past 0xFFFF always force a VMEXIT.
    fn io_bitmap_says_vmexit(&mut self, port: u16, len: u32) -> bool {
        // Wrap-around forces VMEXIT (Bochs guard).
        let end = u32::from(port) + len;
        if end > 0x10000 {
            return true;
        }

        let port_lo = u32::from(port) & 0x7FFF;
        let bitmap = if (port_lo + len) > 0x8000 {
            // Straddles the bitmap-A/B boundary. Bochs reads byte 0xFFF of
            // bitmap A and byte 0x000 of bitmap B.
            let pa = self.vmcs.io_bitmap_addr[0] + 0xFFF;
            let pb = self.vmcs.io_bitmap_addr[1];
            let b0 = u16::from(self.read_physical_byte(pa));
            let b1 = u16::from(self.read_physical_byte(pb));
            (b1 << 8) | b0
        } else {
            // read_physical_byte cannot cross 4 KiB; do two single-byte reads.
            let which = usize::from((port >> 15) & 1);
            let pa = self.vmcs.io_bitmap_addr[which] + u64::from(port_lo / 8);
            let b0 = u16::from(self.read_physical_byte(pa));
            let b1 = u16::from(self.read_physical_byte(pa + 1));
            (b1 << 8) | b0
        };

        let len_mask = (1u16 << len) - 1;
        let mask = len_mask << (port & 7);
        (bitmap & mask) != 0
    }

    /// Decide whether a guest I/O access takes a VMEXIT — Bochs vmexit.cc
    /// VMexit_IO common predicate. Bitmap path beats the unconditional
    /// `IO_VMEXIT` bit when both might apply.
    fn io_should_vmexit(&mut self, port: u16, size: u32) -> bool {
        let ctls = self.proc_based_ctls1();
        if ctls & VMX_VM_EXEC_CTRL1_IO_BITMAPS != 0 {
            self.io_bitmap_says_vmexit(port, size)
        } else if ctls & VMX_VM_EXEC_CTRL1_IO_VMEXIT != 0 {
            true
        } else {
            false
        }
    }

    /// Build the I/O exit qualification — Bochs vmexit.cc VMexit_IO:
    ///   [2:0]   access size - 1
    ///   [3]     direction (0 = OUT, 1 = IN)
    ///   [4]     string instruction
    ///   [5]     REP prefix
    ///   [6]     operand encoding (0 = DX, 1 = immediate)
    ///   [31:16] port number
    fn io_qualification(
        port: u16,
        size: u32,
        direction_in: bool,
        string: bool,
        rep: bool,
        imm: bool,
    ) -> u64 {
        let mut qual: u64 = u64::from(size.saturating_sub(1) & 0x7);
        if direction_in {
            qual |= 1 << 3;
        }
        if string {
            qual |= 1 << 4;
        }
        if rep {
            qual |= 1 << 5;
        }
        if imm {
            qual |= 1 << 6;
        }
        qual |= u64::from(port) << 16;
        qual
    }

    /// IN/OUT (non-string) intercept — Bochs vmexit.cc VMexit_IO with the
    /// `BX_IA_IN_*` / `BX_IA_OUT_*` cases that don't touch GUEST_LINEAR_ADDR
    /// or INSTRUCTION_INFO. Caller passes `imm=true` for the immediate-port
    /// variants (`IN AL, ib`, `OUT ib, AL`, …).
    pub(super) fn vmexit_check_io(
        &mut self,
        port: u16,
        size: u32,
        direction_in: bool,
        imm: bool,
    ) -> Result<bool> {
        if !self.io_should_vmexit(port, size) {
            return Ok(false);
        }
        let qual = Self::io_qualification(port, size, direction_in, false, false, imm);
        self.vmx_vmexit(VmxVmexitReason::IoInstruction, qual)?;
        Ok(true)
    }

    /// INS/OUTS string-form intercept — Bochs vmexit.cc VMexit_IO with the
    /// `BX_IA_REP_INSx` / `BX_IA_REP_OUTSx` cases. Writes the buffer linear
    /// address into `VMCS_GUEST_LINEAR_ADDR` and a packed
    /// `(seg << 15) | (as64 ? 1<<8 : 0) | (as32 ? 1<<7 : 0)` value into
    /// `VMCS_32BIT_VMEXIT_INSTRUCTION_INFO`. `port_in` semantics use ES:RDI;
    /// `port_out` (OUTS) uses the prefix segment with RSI.
    pub(super) fn vmexit_check_io_string(
        &mut self,
        port: u16,
        size: u32,
        direction_in: bool,
        rep: bool,
        linear_addr: u64,
        seg: u8,
        as64: bool,
        as32: bool,
    ) -> Result<bool> {
        if !self.io_should_vmexit(port, size) {
            return Ok(false);
        }
        let qual = Self::io_qualification(port, size, direction_in, true, rep, false);
        self.vmcs.guest_linear_addr = linear_addr;
        let mut info: u32 = u32::from(seg) << 15;
        if as64 {
            info |= 1 << 8;
        } else if as32 {
            info |= 1 << 7;
        }
        self.vmcs.exit_instruction_info = info;
        self.vmx_vmexit(VmxVmexitReason::IoInstruction, qual)?;
        Ok(true)
    }
}
