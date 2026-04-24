// Many SvmVmexit variants and SVM_INTERCEPT* bits are declared up-front for
// Bochs parity but are consumed incrementally as intercept call-sites land
// (Sessions 2+). Permit unused items until the full wire-up is complete.
#![allow(dead_code)]

use crate::config::BxPhyAddress;

use super::crregs::{BxCr0, BxCr4, BxEfer};
use super::descriptor::{BxGlobalSegmentReg, BxSegmentReg};
use super::i387::BxPackedRegister;


// =====================
//  SVM intercept codes
// =====================

/// Mirrors Bochs svm.h enum `SVM_intercept_codes`. Discriminants are the raw
/// VMCB exit codes the host reads after a VMEXIT.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SvmVmexit {
    // CR reads (0x0-0x8).
    Cr0Read = 0x00,
    Cr2Read = 0x02,
    Cr3Read = 0x03,
    Cr4Read = 0x04,
    Cr8Read = 0x08,
    // CR writes (0x10-0x18).
    Cr0Write = 0x10,
    Cr2Write = 0x12,
    Cr3Write = 0x13,
    Cr4Write = 0x14,
    Cr8Write = 0x18,
    // DR reads (0x20) / writes (0x30) — Bochs uses DR0 as the base; exit code
    // carries the specific DR index in bits [0:3].
    Dr0Read = 0x20,
    Dr0Write = 0x30,
    // Exception intercept (0x40 + vector). PF_EXCEPTION = 0x4E in Bochs.
    Exception = 0x40,
    PfException = 0x4E,
    // External events (0x60-0x65).
    Intr = 0x60,
    Nmi = 0x61,
    Smi = 0x62,
    Init = 0x63,
    Vintr = 0x64,
    Cr0SelWrite = 0x65,
    // Descriptor-table reads/writes (0x66-0x6d).
    IdtrRead = 0x66,
    GdtrRead = 0x67,
    LdtrRead = 0x68,
    TrRead = 0x69,
    IdtrWrite = 0x6a,
    GdtrWrite = 0x6b,
    LdtrWrite = 0x6c,
    TrWrite = 0x6d,
    // Counters / flags (0x6e-0x71).
    Rdtsc = 0x6e,
    Rdpmc = 0x6f,
    Pushf = 0x70,
    Popf = 0x71,
    // System events (0x72-0x79).
    Cpuid = 0x72,
    Rsm = 0x73,
    Iret = 0x74,
    SoftwareInterrupt = 0x75,
    Invd = 0x76,
    Pause = 0x77,
    Hlt = 0x78,
    Invlpg = 0x79,
    // Privileged / IO (0x7a-0x7f).
    Invlpga = 0x7a,
    Io = 0x7b,
    Msr = 0x7c,
    TaskSwitch = 0x7d,
    FerrFreeze = 0x7e,
    Shutdown = 0x7f,
    // SVM-specific instructions (0x80-0x88).
    Vmrun = 0x80,
    Vmmcall = 0x81,
    Vmload = 0x82,
    Vmsave = 0x83,
    Stgi = 0x84,
    Clgi = 0x85,
    Skinit = 0x86,
    Rdtscp = 0x87,
    Icebp = 0x88,
    // Advanced (0x89-0x8e).
    Wbinvd = 0x89,
    Monitor = 0x8a,
    Mwait = 0x8b,
    MwaitConditional = 0x8c,
    Xsetbv = 0x8d,
    Rdpru = 0x8e,
    // Write-trap variants (0x8f-0x94).
    EferWriteTrap = 0x8f,
    Cr0WriteTrap = 0x90,
    Cr3WriteTrap = 0x93,
    Cr4WriteTrap = 0x94,
    // Post-SVM extensions (0xa0-0xa6).
    Invlpgb = 0xa0,
    InvlpgbIllegal = 0xa1,
    Invpcid = 0xa2,
    Mcommit = 0xa3,
    Tlbsync = 0xa4,
    Buslock = 0xa5,
    IdleHlt = 0xa6,
    // Nested paging / AVIC / SEV-GHCB (0x400-0x403).
    Npf = 0x400,
    AvicIncompleteIpi = 0x401,
    AvicNoaccel = 0x402,
    Vmgexit = 0x403,
}

pub const SVM_VMEXIT_INVALID: i32 = -1;

// =====================
//  VMCB control fields
// =====================

pub const SVM_CONTROL16_INTERCEPT_CR_READ: u32 = 0x000;
pub const SVM_CONTROL16_INTERCEPT_CR_WRITE: u32 = 0x002;
pub const SVM_CONTROL16_INTERCEPT_DR_READ: u32 = 0x004;
pub const SVM_CONTROL16_INTERCEPT_DR_WRITE: u32 = 0x006;
pub const SVM_CONTROL32_INTERCEPT_EXCEPTIONS: u32 = 0x008;
pub const SVM_CONTROL32_INTERCEPT1: u32 = 0x00c;
pub const SVM_CONTROL32_INTERCEPT2: u32 = 0x010;
pub const SVM_CONTROL32_INTERCEPT3: u32 = 0x014;

pub const SVM_CONTROL16_PAUSE_FILTER_THRESHOLD: u32 = 0x03c;
pub const SVM_CONTROL16_PAUSE_FILTER_COUNT: u32 = 0x03e;
pub const SVM_CONTROL64_IOPM_BASE_PHY_ADDR: u32 = 0x040;
pub const SVM_CONTROL64_MSRPM_BASE_PHY_ADDR: u32 = 0x048;
pub const SVM_CONTROL64_TSC_OFFSET: u32 = 0x050;
pub const SVM_CONTROL32_GUEST_ASID: u32 = 0x058;
pub const SVM_CONTROL_VTPR: u32 = 0x060;
pub const SVM_CONTROL_VIRQ: u32 = 0x061;
pub const SVM_CONTROL_VINTR_PRIO_IGN_TPR: u32 = 0x062;
pub const SVM_CONTROL_VINTR_MASKING: u32 = 0x063;
pub const SVM_CONTROL_VINTR_VECTOR: u32 = 0x064;
pub const SVM_CONTROL_INTERRUPT_SHADOW: u32 = 0x068;
pub const SVM_CONTROL64_EXITCODE: u32 = 0x070;
pub const SVM_CONTROL64_EXITINFO1: u32 = 0x078;
pub const SVM_CONTROL64_EXITINFO2: u32 = 0x080;
pub const SVM_CONTROL32_EXITINTINFO: u32 = 0x088;
pub const SVM_CONTROL32_EXITINTINFO_ERROR_CODE: u32 = 0x08c;
pub const SVM_CONTROL_NESTED_PAGING_ENABLE: u32 = 0x090;

pub const SVM_CONTROL32_EVENT_INJECTION: u32 = 0x0a8;
pub const SVM_CONTROL32_EVENT_INJECTION_ERRORCODE: u32 = 0x0ac;
pub const SVM_CONTROL64_NESTED_PAGING_HOST_CR3: u32 = 0x0b0;
pub const SVM_CONTROL64_NRIP: u32 = 0x0c8;
pub const SVM_CONTROL64_GUEST_INSTR_BYTES: u32 = 0x0d0;

// ======================
//  VMCB save state area
// ======================

pub const SVM_GUEST_ES_SELECTOR: u32 = 0x400;

pub const SVM_GUEST_FS_SELECTOR: u32 = 0x440;

pub const SVM_GUEST_GS_SELECTOR: u32 = 0x450;

pub const SVM_GUEST_GDTR_LIMIT: u32 = 0x464;
pub const SVM_GUEST_GDTR_BASE: u32 = 0x468;

pub const SVM_GUEST_LDTR_SELECTOR: u32 = 0x470;

pub const SVM_GUEST_IDTR_LIMIT: u32 = 0x484;
pub const SVM_GUEST_IDTR_BASE: u32 = 0x488;

pub const SVM_GUEST_TR_SELECTOR: u32 = 0x490;


pub const SVM_GUEST_CPL: u32 = 0x4cb;
pub const SVM_GUEST_EFER_MSR: u32 = 0x4d0;
pub const SVM_GUEST_EFER_MSR_HI: u32 = 0x4d4;

pub const SVM_GUEST_CR4: u32 = 0x548;
pub const SVM_GUEST_CR4_HI: u32 = 0x54c;
pub const SVM_GUEST_CR3: u32 = 0x550;
pub const SVM_GUEST_CR0: u32 = 0x558;
pub const SVM_GUEST_CR0_HI: u32 = 0x55c;
pub const SVM_GUEST_DR7: u32 = 0x560;
pub const SVM_GUEST_DR7_HI: u32 = 0x564;
pub const SVM_GUEST_DR6: u32 = 0x568;
pub const SVM_GUEST_DR6_HI: u32 = 0x56c;
pub const SVM_GUEST_RFLAGS: u32 = 0x570;
pub const SVM_GUEST_RIP: u32 = 0x578;
pub const SVM_GUEST_RSP: u32 = 0x5d8;
pub const SVM_GUEST_RAX: u32 = 0x5f8;
pub const SVM_GUEST_STAR_MSR: u32 = 0x600;
pub const SVM_GUEST_LSTAR_MSR: u32 = 0x608;
pub const SVM_GUEST_CSTAR_MSR: u32 = 0x610;
pub const SVM_GUEST_FMASK_MSR: u32 = 0x618;
pub const SVM_GUEST_KERNEL_GSBASE_MSR: u32 = 0x620;
pub const SVM_GUEST_SYSENTER_CS_MSR: u32 = 0x628;
pub const SVM_GUEST_SYSENTER_ESP_MSR: u32 = 0x630;
pub const SVM_GUEST_SYSENTER_EIP_MSR: u32 = 0x638;
pub const SVM_GUEST_CR2: u32 = 0x640;

pub const SVM_GUEST_PAT: u32 = 0x668;



// ========================
//  SVM intercept controls
// ========================

// vector0[15:00]: intercept reads of CR0-CR15
// vector0[31:16]: intercept writes of CR0-CR15
// vector1[15:00]: intercept reads of DR0-DR15
// vector1[31:16]: intercept writes of DR0-DR15
// vector2[31:00]: intercept exception vectors 0-31
// vector3[31:00]:

// intercept_vector[0] — bits 0..31 (Bochs svm.h SVM_INTERCEPT0_*).
#[allow(non_camel_case_types)]
pub const SVM_INTERCEPT0_INTR: u32 = 0;
pub const SVM_INTERCEPT0_NMI: u32 = 1;
pub const SVM_INTERCEPT0_SMI: u32 = 2;
pub const SVM_INTERCEPT0_INIT: u32 = 3;
pub const SVM_INTERCEPT0_VINTR: u32 = 4;
pub const SVM_INTERCEPT0_CR0_WRITE_NO_TS_MP: u32 = 5;
pub const SVM_INTERCEPT0_IDTR_READ: u32 = 6;
pub const SVM_INTERCEPT0_GDTR_READ: u32 = 7;
pub const SVM_INTERCEPT0_LDTR_READ: u32 = 8;
pub const SVM_INTERCEPT0_TR_READ: u32 = 9;
pub const SVM_INTERCEPT0_IDTR_WRITE: u32 = 10;
pub const SVM_INTERCEPT0_GDTR_WRITE: u32 = 11;
pub const SVM_INTERCEPT0_LDTR_WRITE: u32 = 12;
pub const SVM_INTERCEPT0_TR_WRITE: u32 = 13;
pub const SVM_INTERCEPT0_RDTSC: u32 = 14;
pub const SVM_INTERCEPT0_RDPMC: u32 = 15;
pub const SVM_INTERCEPT0_PUSHF: u32 = 16;
pub const SVM_INTERCEPT0_POPF: u32 = 17;
pub const SVM_INTERCEPT0_CPUID: u32 = 18;
pub const SVM_INTERCEPT0_RSM: u32 = 19;
pub const SVM_INTERCEPT0_IRET: u32 = 20;
pub const SVM_INTERCEPT0_SOFTINT: u32 = 21;
pub const SVM_INTERCEPT0_INVD: u32 = 22;
pub const SVM_INTERCEPT0_PAUSE: u32 = 23;
pub const SVM_INTERCEPT0_HLT: u32 = 24;
pub const SVM_INTERCEPT0_INVLPG: u32 = 25;
pub const SVM_INTERCEPT0_INVLPGA: u32 = 26;
pub const SVM_INTERCEPT0_IO: u32 = 27;
pub const SVM_INTERCEPT0_MSR: u32 = 28;
pub const SVM_INTERCEPT0_TASK_SWITCH: u32 = 29;
pub const SVM_INTERCEPT0_FERR_FREEZE: u32 = 30;
pub const SVM_INTERCEPT0_SHUTDOWN: u32 = 31;

// intercept_vector[1] — bits 32..63 (Bochs svm.h SVM_INTERCEPT1_*).
pub const SVM_INTERCEPT1_VMRUN: u32 = 32;
pub const SVM_INTERCEPT1_VMMCALL: u32 = 33;
pub const SVM_INTERCEPT1_VMLOAD: u32 = 34;
pub const SVM_INTERCEPT1_VMSAVE: u32 = 35;
pub const SVM_INTERCEPT1_STGI: u32 = 36;
pub const SVM_INTERCEPT1_CLGI: u32 = 37;
pub const SVM_INTERCEPT1_SKINIT: u32 = 38;
pub const SVM_INTERCEPT1_RDTSCP: u32 = 39;
pub const SVM_INTERCEPT1_ICEBP: u32 = 40;
pub const SVM_INTERCEPT1_WBINVD: u32 = 41;
pub const SVM_INTERCEPT1_MONITOR: u32 = 42;
pub const SVM_INTERCEPT1_MWAIT: u32 = 43;
pub const SVM_INTERCEPT1_MWAIT_ARMED: u32 = 44;
pub const SVM_INTERCEPT1_XSETBV: u32 = 45;
pub const SVM_INTERCEPT1_RDPRU: u32 = 46;
pub const SVM_INTERCEPT1_EFER_WRITE_TRAP: u32 = 47;
pub const SVM_INTERCEPT1_CR0_WRITE_TRAP: u32 = 48;

// intercept_vector[2] — bits 64..95 (Bochs svm.h SVM_INTERCEPT2_*).
pub const SVM_INTERCEPT2_INVLPGB: u32 = 64;
pub const SVM_INTERCEPT2_INVLPGB_ILLEGAL: u32 = 65;
pub const SVM_INTERCEPT2_INVPCID: u32 = 66;
pub const SVM_INTERCEPT2_MCOMMIT: u32 = 67;
pub const SVM_INTERCEPT2_TLBSYNC: u32 = 68;
pub const SVM_INTERCEPT2_BUSLOCK: u32 = 69;
pub const SVM_INTERCEPT2_IDLE_HLT: u32 = 70;

// ========================
//  SVM data structures
// ========================

#[derive(Default, Clone)]
pub struct SvmHostState {
    pub sregs: [BxSegmentReg; 4],
    pub gdtr: BxGlobalSegmentReg,
    pub idtr: BxGlobalSegmentReg,
    pub efer: BxEfer,
    pub cr0: BxCr0,
    pub cr4: BxCr4,
    pub cr3: BxPhyAddress,
    pub eflags: u32,
    pub rip: u64,
    pub rsp: u64,
    pub rax: u64,
    pub pat_msr: BxPackedRegister,
}


#[derive(Debug, Default, Clone)]
pub struct SvmControls {
    pub cr_rd_ctrl: u16,
    pub cr_wr_ctrl: u16,
    pub dr_rd_ctrl: u16,
    pub dr_wr_ctrl: u16,
    pub exceptions_intercept: u32,

    pub intercept_vector: [u32; 3],

    pub exitintinfo: u32,
    pub exitintinfo_error_code: u32,

    pub eventinj: u32,

    pub iopm_base: BxPhyAddress,
    pub msrpm_base: BxPhyAddress,

    pub v_tpr: u8,
    pub v_intr_prio: u8,
    pub v_ignore_tpr: bool,
    pub v_intr_masking: bool,
    pub v_intr_vector: u8,

    pub nested_paging: bool,
    pub ncr3: u64,

    pub pause_filter_count: u16,
    pub pause_filter_threshold: u16,
    pub last_pause_time: u64,
}

#[derive(Default, Clone)]
pub struct VmcbCache {
    pub host_state: SvmHostState,
    pub ctrls: SvmControls,
}

/// Check if a specific SVM intercept bit is set.
/// intercept_bitnum values are SVM_INTERCEPT0_*, SVM_INTERCEPT1_*, SVM_INTERCEPT2_*.
#[inline]
pub fn svm_intercept(ctrls: &SvmControls, intercept_bitnum: u32) -> bool {
    let vector_idx = (intercept_bitnum / 32) as usize;
    let bit = intercept_bitnum & 31;
    (ctrls.intercept_vector[vector_idx] & (1 << bit)) != 0
}

/// Check if an exception vector is intercepted.
#[inline]
pub fn svm_exception_intercepted(ctrls: &SvmControls, vector: u32) -> bool {
    (ctrls.exceptions_intercept & (1 << vector)) != 0
}


// ========================
//  VM_CR MSR bitmasks
// ========================

pub const BX_VM_CR_MSR_LOCK_MASK: u32         = 1 << 3;
pub const BX_VM_CR_MSR_SVMDIS_MASK: u32       = 1 << 4;

// SVM MSR addresses
pub const BX_SVM_VM_CR_MSR: u32      = 0xc001_0114;
pub const BX_SVM_IGNNE_MSR: u32      = 0xc001_0115;
pub const BX_SVM_SMM_CTL_MSR: u32    = 0xc001_0116;
pub const BX_SVM_VM_HSAVE_PA_MSR: u32 = 0xc001_0117;

/// SVM VIRQ event pending — bit 8 to match Bochs convention.
pub const BX_EVENT_SVM_VIRQ_PENDING: u32 = 1 << 8;


use super::{
    cpu::{BxCpuC, Exception},
    cpuid::BxCpuIdTrait,
    cet::canonicalize_address,
    decoder::{BxSegregs, Instruction},
    eflags::EFlags,
    exception::InterruptType,
    segment_ctrl_pro::parse_selector,
};

impl<I: BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> BxCpuC<'_, I, T> {

    // =====================================================================
    //  VMCB physical-memory access helpers
    // =====================================================================

    /// Read a u8 from the VMCB at `offset`.
    fn vmcb_read8(&mut self, offset: u32) -> u8 {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            // Fast path: host pointer available
            let host = (self.vmcbhostptr | offset as u64) as *const u8;
            // SAFETY: vmcbhostptr validated by set_vmcbptr; single-threaded
            unsafe { *host }
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = [0u8; 1];
            let _ = mem.read_physical_page(&[cpu_ref], paddr, 1, &mut data);
            data[0]
        } else {
            0
        }
    }

    /// Read a u16 from the VMCB at `offset`.
    fn vmcb_read16(&mut self, offset: u32) -> u16 {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *const [u8; 2];
            u16::from_le_bytes(unsafe { *host })
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = [0u8; 2];
            let _ = mem.read_physical_page(&[cpu_ref], paddr, 2, &mut data);
            u16::from_le_bytes(data)
        } else {
            0
        }
    }

    /// Read a u32 from the VMCB at `offset`.
    fn vmcb_read32(&mut self, offset: u32) -> u32 {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *const [u8; 4];
            u32::from_le_bytes(unsafe { *host })
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = [0u8; 4];
            let _ = mem.read_physical_page(&[cpu_ref], paddr, 4, &mut data);
            u32::from_le_bytes(data)
        } else {
            0
        }
    }

    /// Read a u64 from the VMCB at `offset`.
    fn vmcb_read64(&mut self, offset: u32) -> u64 {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *const [u8; 8];
            u64::from_le_bytes(unsafe { *host })
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = [0u8; 8];
            let _ = mem.read_physical_page(&[cpu_ref], paddr, 8, &mut data);
            u64::from_le_bytes(data)
        } else {
            0
        }
    }

    /// Write a u8 to the VMCB at `offset`.
    fn vmcb_write8(&mut self, offset: u32, val: u8) {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *mut u8;
            // SAFETY: vmcbhostptr validated; single-threaded
            unsafe { *host = val; }
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = [val];
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = super::icache::BxPageWriteStampTable { fine_granularity_mapping: &mut dummy_mapping };
            let _ = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 1, &mut data);
        }
    }

    /// Write a u16 to the VMCB at `offset`.
    fn vmcb_write16(&mut self, offset: u32, val: u16) {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *mut [u8; 2];
            unsafe { *host = val.to_le_bytes(); }
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = val.to_le_bytes();
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = super::icache::BxPageWriteStampTable { fine_granularity_mapping: &mut dummy_mapping };
            let _ = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 2, &mut data);
        }
    }

    /// Write a u32 to the VMCB at `offset`.
    fn vmcb_write32(&mut self, offset: u32, val: u32) {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *mut [u8; 4];
            unsafe { *host = val.to_le_bytes(); }
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = val.to_le_bytes();
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = super::icache::BxPageWriteStampTable { fine_granularity_mapping: &mut dummy_mapping };
            let _ = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 4, &mut data);
        }
    }

    /// Write a u64 to the VMCB at `offset`.
    fn vmcb_write64(&mut self, offset: u32, val: u64) {
        let paddr = self.vmcbptr + offset as u64;
        if self.vmcbhostptr != 0 {
            let host = (self.vmcbhostptr | offset as u64) as *mut [u8; 8];
            unsafe { *host = val.to_le_bytes(); }
        } else if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = val.to_le_bytes();
            let mut dummy_mapping: [u32; 0] = [];
            let mut stamp = super::icache::BxPageWriteStampTable { fine_granularity_mapping: &mut dummy_mapping };
            let _ = mem.write_physical_page(&[cpu_ref], &mut stamp, paddr, 8, &mut data);
        }
    }

    // =====================================================================
    //  Segment helpers: read/write segment register from/to VMCB
    // =====================================================================

    /// Read a segment register from VMCB at the given `offset`.
    /// Populates selector, attributes, limit, and base.
    fn svm_segment_read(&mut self, offset: u32) -> super::descriptor::BxSegmentReg {
        let selector = self.vmcb_read16(offset);
        let attr = self.vmcb_read16(offset + 2);
        let limit = self.vmcb_read32(offset + 4);
        let base = canonicalize_address(self.vmcb_read64(offset + 8));
        let valid = (attr >> 7) & 1;

        // Reconstruct the descriptor from SVM-format attributes
        // SVM attr format: [7:0] = access byte, [11:8] = upper flags (G, D/B, L, AVL)
        let ar_byte = (attr & 0xff) as u8;
        let upper = ((attr & 0xf00) << 4) as u32;

        let mut seg = super::descriptor::BxSegmentReg::default();
        parse_selector(selector, &mut seg.selector);
        seg.cache.valid = if valid != 0 { 1 } else { 0 };
        seg.cache.set_ar_byte(ar_byte);
        seg.cache.u.set_segment_base(base);
        seg.cache.u.set_segment_limit_scaled(limit);
        seg.cache.u.set_segment_g(upper & 0x0080_0000 != 0);
        seg.cache.u.set_segment_d_b(upper & 0x0040_0000 != 0);
        seg.cache.u.set_segment_l(upper & 0x0020_0000 != 0);
        seg.cache.u.set_segment_avl(upper & 0x0010_0000 != 0);
        seg
    }

    /// Write a segment register to VMCB at the given `offset`.
    fn svm_segment_write(&mut self, seg: &super::descriptor::BxSegmentReg, offset: u32) {
        let selector = seg.selector.value;
        let base = seg.cache.u.segment_base();
        let limit = seg.cache.u.segment_limit_scaled();
        // Encode descriptor into SVM attribute format
        let ar_byte = seg.cache.get_ar_byte() as u16;
        let g    = if seg.cache.u.segment_g()   { 1u16 } else { 0 };
        let d_b  = if seg.cache.u.segment_d_b() { 1u16 } else { 0 };
        let l    = if seg.cache.u.segment_l()    { 1u16 } else { 0 };
        let avl  = if seg.cache.u.segment_avl()  { 1u16 } else { 0 };
        let valid = if seg.cache.valid != 0 { 1u16 } else { 0 };
        let attr = ar_byte
            | (valid << 7)
            | (avl  << 8)
            | (l    << 9)
            | (d_b  << 10)
            | (g    << 11);

        self.vmcb_write16(offset, selector);
        self.vmcb_write16(offset + 2, attr);
        self.vmcb_write32(offset + 4, limit);
        self.vmcb_write64(offset + 8, base);
    }

    // =====================================================================
    //  VMCB pointer management
    // =====================================================================

    /// Set the VMCB pointer and cache the host memory address.
    /// Bochs svm.cc set_VMCBPTR()
    pub(super) fn set_vmcbptr(&mut self, vmcbptr: u64) {
        self.vmcbptr = vmcbptr;
        if vmcbptr != 0 {
            // Try to get a direct host pointer for fast VMCB access
            if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
                use super::rusty_box::MemoryAccessType;
                match mem.get_host_mem_addr(vmcbptr, MemoryAccessType::RW, &[cpu_ref]) {
                    Ok(Some(slice)) => self.vmcbhostptr = slice.as_ptr() as super::tlb::BxHostpageaddr,
                    _ => self.vmcbhostptr = 0,
                }
            } else {
                self.vmcbhostptr = 0;
            }
        } else {
            self.vmcbhostptr = 0;
        }
    }

    // =====================================================================
    //  VMRUN host state save
    // =====================================================================

    /// Save host CPU state into the VMCB host_state area.
    /// Bochs svm.cc SvmEnterSaveHostState()
    fn svm_enter_save_host_state(&mut self) {
        // Read register values before creating mutable borrow of vmcb
        let rip = self.rip();
        let rsp = self.rsp();
        let rax = self.rax();
        let eflags = self.read_eflags();
        let sregs = self.sregs.clone();
        let gdtr = self.gdtr.clone();
        let idtr = self.idtr.clone();
        let efer = self.efer;
        let cr0 = self.cr0;
        let cr3 = self.cr3;
        let cr4 = self.cr4;
        let pat_msr = self.msr.pat;

        let vmcb = self.vmcb.get_or_insert_with(VmcbCache::default);
        for n in 0..4 {
            vmcb.host_state.sregs[n] = sregs[n].clone();
        }
        vmcb.host_state.gdtr = gdtr;
        vmcb.host_state.idtr = idtr;
        vmcb.host_state.efer = efer;
        vmcb.host_state.cr0 = cr0;
        vmcb.host_state.cr3 = cr3;
        vmcb.host_state.cr4 = cr4;
        vmcb.host_state.eflags = eflags;
        vmcb.host_state.rip = rip;
        vmcb.host_state.rsp = rsp;
        vmcb.host_state.rax = rax;
        vmcb.host_state.pat_msr = pat_msr;
    }

    // =====================================================================
    //  VMEXIT host state restore
    // =====================================================================

    /// Restore host CPU state from the VMCB host_state area.
    /// Bochs svm.cc SvmExitLoadHostState()
    fn svm_exit_load_host_state(&mut self) {
        self.tsc_offset = 0;

        let host_state = self.vmcb.as_ref().expect("vmcb must exist during VMEXIT").host_state.clone();

        for n in 0..4 {
            self.sregs[n] = host_state.sregs[n].clone();
            // Re-parse selector after loading (selector details not saved)
            let val = self.sregs[n].selector.value;
            parse_selector(val, &mut self.sregs[n].selector);
        }

        // Set EFLAGS before control registers to avoid false PANIC inside setEFlags
        let eflags_no_vm = host_state.eflags & !EFlags::VM.bits();
        self.set_eflags_internal(eflags_no_vm);

        self.gdtr = host_state.gdtr;
        self.idtr = host_state.idtr;

        self.efer = host_state.efer;
        // Always set CR0.PE when restoring host state
        let cr0_val = host_state.cr0.get32() | 1; // BX_CR0_PE_MASK = 1
        self.cr0.set32(cr0_val);
        self.cr3 = host_state.cr3;
        self.cr4 = host_state.cr4;

        self.msr.pat = host_state.pat_msr;
        self.dr7.set32(0x0000_0400);

        self.set_rip(host_state.rip);
        self.prev_rip = host_state.rip;
        self.set_rsp(host_state.rsp);
        self.set_rax(host_state.rax);

        // CPL = 0 for host mode
        self.sregs[BxSegregs::Cs as usize].selector.rpl = 0;
        self.sregs[BxSegregs::Ss as usize].selector.rpl = 0;
        self.sregs[BxSegregs::Cs as usize].cache.dpl = 0;
        self.sregs[BxSegregs::Ss as usize].cache.dpl = 0;

        self.handle_cpu_context_change();
    }

    // =====================================================================
    //  VMEXIT guest state save
    // =====================================================================

    /// Save guest CPU state to VMCB on VMEXIT.
    /// Bochs svm.cc SvmExitSaveGuestState()
    fn svm_exit_save_guest_state(&mut self) {
        for n in 0..4u32 {
            let seg = self.sregs[n as usize].clone();
            self.svm_segment_write(&seg, SVM_GUEST_ES_SELECTOR + n * 0x10);
        }

        self.vmcb_write64(SVM_GUEST_GDTR_BASE, self.gdtr.base);
        self.vmcb_write16(SVM_GUEST_GDTR_LIMIT, self.gdtr.limit);

        self.vmcb_write64(SVM_GUEST_IDTR_BASE, self.idtr.base);
        self.vmcb_write16(SVM_GUEST_IDTR_LIMIT, self.idtr.limit);

        self.vmcb_write64(SVM_GUEST_EFER_MSR, self.efer.get32() as u64);
        self.vmcb_write64(SVM_GUEST_CR0, self.cr0.get32() as u64);
        self.vmcb_write64(SVM_GUEST_CR2, self.cr2);
        self.vmcb_write64(SVM_GUEST_CR3, self.cr3);
        self.vmcb_write64(SVM_GUEST_CR4, self.cr4.get() as u64);

        self.vmcb_write64(SVM_GUEST_DR6, self.dr6.get32() as u64);
        self.vmcb_write64(SVM_GUEST_DR7, self.dr7.get32() as u64);

        self.vmcb_write64(SVM_GUEST_RFLAGS, self.eflags_materialized() as u64);
        self.vmcb_write64(SVM_GUEST_RAX, self.rax());
        self.vmcb_write64(SVM_GUEST_RSP, self.rsp());
        self.vmcb_write64(SVM_GUEST_RIP, self.rip());

        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        self.vmcb_write8(SVM_GUEST_CPL, cpl);

        let inhibit = self.interrupts_inhibited(Self::BX_INHIBIT_INTERRUPTS);
        self.vmcb_write8(SVM_CONTROL_INTERRUPT_SHADOW, inhibit as u8);

        let nested_paging = self.vmcb.as_ref().map_or(false, |v| v.ctrls.nested_paging);
        if nested_paging {
            self.vmcb_write64(SVM_GUEST_PAT, self.msr.pat.U64());
        }

        // Save virtual interrupt state
        let vmcb = self.vmcb.as_mut().expect("vmcb must exist");
        let v_tpr = vmcb.ctrls.v_tpr;
        self.vmcb_write8(SVM_CONTROL_VTPR, v_tpr);
        let virq_pending = (self.pending_event & BX_EVENT_SVM_VIRQ_PENDING) != 0;
        self.vmcb_write8(SVM_CONTROL_VIRQ, virq_pending as u8);
        self.clear_event(BX_EVENT_SVM_VIRQ_PENDING);
    }

    // =====================================================================
    //  VMRUN: Load and check control fields
    // =====================================================================

    /// Load and validate VMCB control fields.
    /// Returns false if validation fails (VMEXIT_INVALID).
    /// Bochs svm.cc SvmEnterLoadCheckControls()
    fn svm_enter_load_check_controls(&mut self) -> bool {
        let cr_rd_ctrl = self.vmcb_read16(SVM_CONTROL16_INTERCEPT_CR_READ);
        let cr_wr_ctrl = self.vmcb_read16(SVM_CONTROL16_INTERCEPT_CR_WRITE);
        let dr_rd_ctrl = self.vmcb_read16(SVM_CONTROL16_INTERCEPT_DR_READ);
        let dr_wr_ctrl = self.vmcb_read16(SVM_CONTROL16_INTERCEPT_DR_WRITE);

        let intercept0 = self.vmcb_read32(SVM_CONTROL32_INTERCEPT1);
        let intercept1 = self.vmcb_read32(SVM_CONTROL32_INTERCEPT2);
        let intercept2 = self.vmcb_read32(SVM_CONTROL32_INTERCEPT3);

        // VMRUN intercept bit must be set
        if (intercept1 & (1 << (SVM_INTERCEPT1_VMRUN - 32))) == 0 {
            tracing::error!("VMRUN: VMRUN intercept bit is not set!");
            return false;
        }

        let exceptions_intercept = self.vmcb_read32(SVM_CONTROL32_INTERCEPT_EXCEPTIONS);

        // Force 4K page alignment on IOPM base
        let iopm_base = self.vmcb_read64(SVM_CONTROL64_IOPM_BASE_PHY_ADDR) & !0xFFF;
        // Force 4K page alignment on MSRPM base
        let msrpm_base = self.vmcb_read64(SVM_CONTROL64_MSRPM_BASE_PHY_ADDR) & !0xFFF;

        let guest_asid = self.vmcb_read32(SVM_CONTROL32_GUEST_ASID);
        if guest_asid == 0 {
            tracing::error!("VMRUN: attempt to run guest with host ASID!");
            return false;
        }

        let v_tpr = self.vmcb_read8(SVM_CONTROL_VTPR);
        let v_intr_masking = (self.vmcb_read8(SVM_CONTROL_VINTR_MASKING) & 0x1) != 0;
        let v_intr_vector = self.vmcb_read8(SVM_CONTROL_VINTR_VECTOR);

        let vintr_control = self.vmcb_read8(SVM_CONTROL_VINTR_PRIO_IGN_TPR);
        let v_intr_prio = vintr_control & 0xf;
        let v_ignore_tpr = ((vintr_control >> 4) & 0x1) != 0;

        let pause_filter_count = self.vmcb_read16(SVM_CONTROL16_PAUSE_FILTER_COUNT);
        let pause_filter_threshold = self.vmcb_read16(SVM_CONTROL16_PAUSE_FILTER_THRESHOLD);

        let nested_paging = self.vmcb_read8(SVM_CONTROL_NESTED_PAGING_ENABLE) != 0;

        let ncr3 = if nested_paging {
            self.vmcb_read64(SVM_CONTROL64_NESTED_PAGING_HOST_CR3)
        } else {
            0
        };

        // Store into VMCB cache
        let vmcb = self.vmcb.get_or_insert_with(VmcbCache::default);
        vmcb.ctrls.cr_rd_ctrl = cr_rd_ctrl;
        vmcb.ctrls.cr_wr_ctrl = cr_wr_ctrl;
        vmcb.ctrls.dr_rd_ctrl = dr_rd_ctrl;
        vmcb.ctrls.dr_wr_ctrl = dr_wr_ctrl;
        vmcb.ctrls.intercept_vector[0] = intercept0;
        vmcb.ctrls.intercept_vector[1] = intercept1;
        vmcb.ctrls.intercept_vector[2] = intercept2;
        vmcb.ctrls.exceptions_intercept = exceptions_intercept;
        vmcb.ctrls.iopm_base = iopm_base;
        vmcb.ctrls.msrpm_base = msrpm_base;
        vmcb.ctrls.v_tpr = v_tpr;
        vmcb.ctrls.v_intr_masking = v_intr_masking;
        vmcb.ctrls.v_intr_vector = v_intr_vector;
        vmcb.ctrls.v_intr_prio = v_intr_prio;
        vmcb.ctrls.v_ignore_tpr = v_ignore_tpr;
        vmcb.ctrls.pause_filter_count = pause_filter_count;
        vmcb.ctrls.pause_filter_threshold = pause_filter_threshold;
        vmcb.ctrls.last_pause_time = 0;
        vmcb.ctrls.nested_paging = nested_paging;
        vmcb.ctrls.ncr3 = ncr3;

        true
    }

    // =====================================================================
    //  VMRUN: Load and check guest state
    // =====================================================================

    /// Load guest state from VMCB and validate consistency.
    /// Returns false if validation fails (VMEXIT_INVALID).
    /// Bochs svm.cc SvmEnterLoadCheckGuestState()
    fn svm_enter_load_check_guest_state(&mut self) -> bool {
        let guest_eflags = self.vmcb_read32(SVM_GUEST_RFLAGS);
        let guest_rip = self.vmcb_read64(SVM_GUEST_RIP);
        let guest_rsp = self.vmcb_read64(SVM_GUEST_RSP);
        let guest_rax = self.vmcb_read64(SVM_GUEST_RAX);

        // EFER validation
        let efer_lo = self.vmcb_read32(SVM_GUEST_EFER_MSR);
        let efer_hi = self.vmcb_read32(SVM_GUEST_EFER_MSR_HI);
        if efer_hi != 0 {
            tracing::error!("VMRUN: Guest EFER[63:32] is not zero");
            return false;
        }
        let mut guest_efer = BxEfer::default();
        guest_efer.set32(efer_lo);
        if (guest_efer.get32() & !self.efer_suppmask) != 0 {
            tracing::error!("VMRUN: Guest EFER reserved bits set");
            return false;
        }
        if !guest_efer.svme() {
            tracing::error!("VMRUN: Guest EFER.SVME = 0");
            return false;
        }

        // CR0 validation
        let cr0_lo = self.vmcb_read32(SVM_GUEST_CR0);
        let cr0_hi = self.vmcb_read32(SVM_GUEST_CR0_HI);
        if cr0_hi != 0 {
            tracing::error!("VMRUN: Guest CR0[63:32] is not zero");
            return false;
        }
        let mut guest_cr0 = BxCr0::default();
        guest_cr0.set32(cr0_lo);

        // EFER.LMA := EFER.LME & CR0.PG
        guest_efer.set_lma(if guest_cr0.pg() && guest_efer.lme() { 1 } else { 0 });

        let guest_cr2 = self.vmcb_read64(SVM_GUEST_CR2);
        let guest_cr3 = self.vmcb_read64(SVM_GUEST_CR3);

        // CR4 validation
        let cr4_lo = self.vmcb_read32(SVM_GUEST_CR4);
        let cr4_hi = self.vmcb_read32(SVM_GUEST_CR4_HI);
        if cr4_hi != 0 {
            tracing::error!("VMRUN: Guest CR4[63:32] is not zero");
            return false;
        }
        let mut guest_cr4 = BxCr4::default();
        guest_cr4.set_val(cr4_lo as u64);
        if (guest_cr4.get() & !self.cr4_suppmask) != 0 {
            tracing::error!("VMRUN: Guest CR4 reserved bits set");
            return false;
        }

        // DR6/DR7 validation
        let dr6_hi = self.vmcb_read32(SVM_GUEST_DR6_HI);
        if dr6_hi != 0 {
            tracing::error!("VMRUN: Guest DR6[63:32] is not zero");
            return false;
        }
        let guest_dr6 = self.vmcb_read32(SVM_GUEST_DR6);
        let dr7_hi = self.vmcb_read32(SVM_GUEST_DR7_HI);
        if dr7_hi != 0 {
            tracing::error!("VMRUN: Guest DR7[63:32] is not zero");
            return false;
        }
        let guest_dr7 = self.vmcb_read32(SVM_GUEST_DR7);

        let guest_pat = self.vmcb_read64(SVM_GUEST_PAT);

        // Load segment registers (ES, CS, SS, DS)
        let mut guest_sregs = [
            super::descriptor::BxSegmentReg::default(),
            super::descriptor::BxSegmentReg::default(),
            super::descriptor::BxSegmentReg::default(),
            super::descriptor::BxSegmentReg::default(),
        ];
        for n in 0..4u32 {
            guest_sregs[n as usize] = self.svm_segment_read(SVM_GUEST_ES_SELECTOR + n * 0x10);
        }

        // CS.D_B and CS.L cannot both be set
        let cs = &guest_sregs[BxSegregs::Cs as usize];
        if cs.cache.u.segment_d_b() && cs.cache.u.segment_l() {
            tracing::error!("VMRUN: VMCB CS.D_B/L mismatch");
            return false;
        }

        // Real/V8086 mode: make all segments valid
        let mut paged_real_mode = false;
        if !guest_cr0.pe() || (guest_eflags & EFlags::VM.bits()) != 0 {
            for seg in guest_sregs.iter_mut() {
                seg.cache.valid = 1;
            }
            if !guest_cr0.pe() && guest_cr0.pg() {
                // Paged real mode
                paged_real_mode = true;
                guest_cr0.remove(BxCr0::PG); // Clear PG temporarily
            }
        }

        let guest_cpl = self.vmcb_read8(SVM_GUEST_CPL);

        // Patch SS.DPL = CPL
        guest_sregs[BxSegregs::Ss as usize].cache.dpl = guest_cpl;

        let guest_gdtr_base = canonicalize_address(self.vmcb_read64(SVM_GUEST_GDTR_BASE));
        let guest_gdtr_limit = self.vmcb_read16(SVM_GUEST_GDTR_LIMIT);
        let guest_idtr_base = canonicalize_address(self.vmcb_read64(SVM_GUEST_IDTR_BASE));
        let guest_idtr_limit = self.vmcb_read16(SVM_GUEST_IDTR_LIMIT);

        let guest_inhibit = (self.vmcb_read8(SVM_CONTROL_INTERRUPT_SHADOW) & 0x1) != 0;

        // ── Load guest state ──

        self.tsc_offset = self.vmcb_read64(SVM_CONTROL64_TSC_OFFSET) as i64;
        self.efer.set32(guest_efer.get32());

        self.cr0.set32(guest_cr0.get32());
        self.cr4.set_val(guest_cr4.get());
        self.cr3 = guest_cr3;

        if paged_real_mode {
            self.cr0.insert(BxCr0::PG); // Restore PG
        }

        self.cr2 = guest_cr2;
        self.dr6.set32(guest_dr6);
        self.dr7.set32(guest_dr7 | 0x400);

        for n in 0..4 {
            self.sregs[n] = guest_sregs[n].clone();
        }

        self.gdtr.base = guest_gdtr_base;
        self.gdtr.limit = guest_gdtr_limit;
        self.idtr.base = guest_idtr_base;
        self.idtr.limit = guest_idtr_limit;

        self.set_rip(guest_rip);
        self.prev_rip = guest_rip;
        self.set_rsp(guest_rsp);
        self.set_rax(guest_rax);

        // Set CPL
        self.sregs[BxSegregs::Cs as usize].selector.rpl = guest_cpl;
        self.sregs[BxSegregs::Ss as usize].selector.rpl = guest_cpl;
        self.sregs[BxSegregs::Cs as usize].cache.dpl = guest_cpl;
        self.sregs[BxSegregs::Ss as usize].cache.dpl = guest_cpl;

        if guest_inhibit {
            self.inhibit_interrupts(Self::BX_INHIBIT_INTERRUPTS);
        }

        self.async_event = 0;
        self.set_eflags_internal(guest_eflags);

        // Nested paging: load guest PAT
        let nested = self.vmcb.as_ref().map_or(false, |v| v.ctrls.nested_paging);
        if nested {
            self.msr.pat.set_U64(guest_pat);
        }

        // Virtual interrupt injection
        let v_irq = self.vmcb_read8(SVM_CONTROL_VIRQ) & 0x1;
        if v_irq != 0 {
            self.signal_event(BX_EVENT_SVM_VIRQ_PENDING);
        }

        self.handle_cpu_context_change();

        true
    }

    // =====================================================================
    //  Svm_Vmexit — main VMEXIT handler
    // =====================================================================

    /// Execute a VMEXIT. Saves guest state, restores host state, and returns
    /// `Err(CpuLoopRestart)` to unwind back to the decode loop.
    /// Bochs svm.cc Svm_Vmexit()
    pub(super) fn svm_vmexit(
        &mut self,
        reason: i32,
        exitinfo1: u64,
        exitinfo2: u64,
    ) -> super::Result<()> {
        tracing::debug!(
            "SVM VMEXIT reason={} exitinfo1={:#x} exitinfo2={:#x}",
            reason, exitinfo1, exitinfo2
        );

        if !self.in_svm_guest && reason != SVM_VMEXIT_INVALID {
            tracing::error!("PANIC: VMEXIT {} not in SVM guest mode!", reason);
        }

        if reason != SVM_VMEXIT_INVALID {
            // NRIP save (write current RIP as next-RIP)
            self.vmcb_write64(SVM_CONTROL64_NRIP, self.rip());
            // Decode assist: clear guest instruction bytes for non-PF/NPF exits
            self.vmcb_write8(SVM_CONTROL64_GUEST_INSTR_BYTES, 0);
        }

        // VMEXITs are FAULT-like: restore RIP/RSP to pre-instruction values
        self.set_rip(self.prev_rip);
        if self.speculative_rsp {
            self.set_rsp(self.prev_rsp);
        }
        self.speculative_rsp = false;

        self.clear_event(BX_EVENT_SVM_VIRQ_PENDING);
        self.in_svm_guest = false;
        self.svm_gif = false;

        // Write exit reason and info to VMCB
        self.vmcb_write64(SVM_CONTROL64_EXITCODE, reason as i64 as u64);
        self.vmcb_write64(SVM_CONTROL64_EXITINFO1, exitinfo1);
        self.vmcb_write64(SVM_CONTROL64_EXITINFO2, exitinfo2);

        // Clean up event injection field
        let eventinj = self.vmcb.as_ref().map_or(0, |v| v.ctrls.eventinj);
        self.vmcb_write32(SVM_CONTROL32_EVENT_INJECTION, eventinj & !0x8000_0000);

        // If exiting during event delivery, save the interrupted event info
        if self.in_event {
            let vmcb = self.vmcb.as_ref().expect("vmcb must exist");
            let exitintinfo = vmcb.ctrls.exitintinfo;
            let error_code = vmcb.ctrls.exitintinfo_error_code;
            self.vmcb_write32(SVM_CONTROL32_EXITINTINFO, exitintinfo | 0x8000_0000);
            self.vmcb_write32(SVM_CONTROL32_EXITINTINFO_ERROR_CODE, error_code);
            self.in_event = false;
        } else {
            self.vmcb_write32(SVM_CONTROL32_EXITINTINFO, 0);
        }

        // Save guest state
        if reason != SVM_VMEXIT_INVALID {
            self.svm_exit_save_guest_state();
        }

        // Restore host state
        self.svm_exit_load_host_state();

        // Clear EXT and last_exception_type
        self.ext = false;
        self.last_exception_type = -1; // BX_ET_NONE

        // Restart CPU loop (Bochs uses longjmp; we use Err(CpuLoopRestart))
        Err(super::error::CpuError::CpuLoopRestart)
    }

    // =====================================================================
    //  SvmInjectEvents — inject pending events from VMCB
    // =====================================================================

    /// Inject events from VMCB EVENT_INJECTION field into the guest.
    /// Returns false if the injection is invalid (VMEXIT_INVALID).
    /// Bochs svm.cc SvmInjectEvents()
    fn svm_inject_events(&mut self) -> bool {
        let eventinj = self.vmcb_read32(SVM_CONTROL32_EVENT_INJECTION);
        {
            let vmcb = self.vmcb.as_mut().expect("vmcb must exist");
            vmcb.ctrls.eventinj = eventinj;
        }
        if (eventinj & 0x8000_0000) == 0 {
            return true; // No event to inject
        }

        let vector = (eventinj & 0xff) as u8;
        let event_type = ((eventinj >> 8) & 7) as u8;
        let push_error = (eventinj & (1 << 11)) != 0;
        let error_code = if push_error {
            self.vmcb_read32(SVM_CONTROL32_EVENT_INJECTION_ERRORCODE) as u16
        } else {
            0
        };

        // Convert SVM event type to InterruptType
        let int_type = match event_type {
            4 => {
                // NMI
                self.ext = true;
                InterruptType::Nmi
            }
            3 => {
                // External interrupt
                self.ext = true;
                InterruptType::ExternalInterrupt
            }
            5 => {
                // Hardware exception
                if vector == 2 || vector > 31 {
                    tracing::error!("SvmInjectEvents: invalid vector {} for HW exception", vector);
                    return false;
                }
                // #BP and #OF are software exceptions
                if vector == 3 || vector == 4 {
                    self.ext = true;
                    InterruptType::SoftwareException
                } else {
                    self.ext = true;
                    InterruptType::HardwareException
                }
            }
            0 => {
                // Software interrupt
                InterruptType::SoftwareInterrupt
            }
            _ => {
                tracing::error!("SvmInjectEvents: unsupported event type {}", event_type);
                return false;
            }
        };

        tracing::debug!("SvmInjectEvents: vector={:#04x} error_code={:#06x}", vector, error_code);

        // Record exit int info for nested event tracking
        {
            let vmcb = self.vmcb.as_mut().expect("vmcb must exist");
            vmcb.ctrls.exitintinfo = eventinj & !0x8000_0000;
            vmcb.ctrls.exitintinfo_error_code = error_code as u32;
        }

        // Deliver the interrupt (this may unwind via CpuLoopRestart on exception)
        let nmi_vector = if int_type as u8 == InterruptType::Nmi as u8 { 2 } else { vector };
        let soft_int = matches!(int_type, InterruptType::SoftwareInterrupt);
        if self.interrupt(nmi_vector, int_type, soft_int, push_error, error_code).is_err() {
            // If interrupt delivery itself caused an exception, the CpuLoopRestart
            // will propagate up. The event is still recorded in exitintinfo.
        }

        self.last_exception_type = -1; // BX_ET_NONE
        true
    }

    // =====================================================================
    //  Intercept handlers
    // =====================================================================

    /// Check if an SVM intercept bit is set in the current VMCB controls.
    /// `pub(super)` so dispatch sites (proc_ctrl, io, ctrl_xfer, flag_ctrl,
    /// soft_int, crregs, dreg, tasking) can gate their handlers on SVM guest
    /// intercepts — the Bochs svm.h SVM_INTERCEPT(...) macro wraps those
    /// sites identically.
    #[inline]
    pub(super) fn svm_intercept_check(&self, intercept_bitnum: u32) -> bool {
        match &self.vmcb {
            Some(vmcb) => svm_intercept(&vmcb.ctrls, intercept_bitnum),
            None => false,
        }
    }

    /// Check if an exception is intercepted by SVM.
    #[inline]
    fn svm_exception_intercept_check(&self, vector: u32) -> bool {
        match &self.vmcb {
            Some(vmcb) => svm_exception_intercepted(&vmcb.ctrls, vector),
            None => false,
        }
    }

    /// SVM exception intercept handler.
    /// Called from exception() when in SVM guest mode.
    /// Bochs svm.cc SvmInterceptException()
    pub(super) fn svm_intercept_exception(
        &mut self,
        vector: u8,
        errcode: u16,
        errcode_valid: bool,
        qualification: u64,
    ) -> super::Result<()> {
        if !self.in_svm_guest {
            return Ok(());
        }

        if !self.svm_exception_intercept_check(vector as u32) {
            // Not intercepted — record IDT vectoring information for future VMEXIT
            if let Some(vmcb) = self.vmcb.as_mut() {
                vmcb.ctrls.exitintinfo_error_code = errcode as u32;
                let mut info = vector as u32 | (InterruptType::HardwareException as u32) << 8;
                if errcode_valid {
                    info |= 1 << 11;
                }
                vmcb.ctrls.exitintinfo = info;
            }
            return Ok(());
        }

        tracing::debug!(
            "SVM VMEXIT: exception vector={:#04x} errcode={:#06x}",
            vector, errcode
        );

        // #DF clears in_event
        if vector == Exception::Df as u8 {
            self.in_event = false;
        }

        self.debug_trap = 0;
        self.inhibit_mask = 0;

        let exitinfo1 = if errcode_valid { errcode as u64 } else { 0 };
        self.svm_vmexit(
            SvmVmexit::Exception as i32 + vector as i32,
            exitinfo1,
            qualification,
        )
    }

    /// SVM MSR intercept handler.
    /// Checks the MSRPM bitmap and performs VMEXIT if the MSR is intercepted.
    /// Bochs svm.cc SvmInterceptMSR()
    pub(super) fn svm_intercept_msr(
        &mut self,
        op: u32,  // 0 = read, 1 = write
        msr: u32,
    ) -> super::Result<()> {
        if !self.in_svm_guest {
            return Ok(());
        }
        if !self.svm_intercept_check(SVM_INTERCEPT0_MSR) {
            return Ok(());
        }

        let mut vmexit = true;

        // Determine MSR bitmap offset based on MSR range
        let msr_map_offset: i32 = if msr <= 0x1fff {
            0
        } else if msr >= 0xc000_0000 && msr <= 0xc000_1fff {
            2048
        } else if msr >= 0xc001_0000 && msr <= 0xc001_1fff {
            4096
        } else {
            -1 // Not in any range, always intercept
        };

        if msr_map_offset >= 0 {
            let msrpm_base = self.vmcb.as_ref().map_or(0, |v| v.ctrls.msrpm_base);
            let msr_bitmap_addr = msrpm_base + msr_map_offset as u64;
            let msr_offset = (msr & 0x1fff) * 2 + op;

            let paddr = msr_bitmap_addr + (msr_offset / 8) as u64;
            let msr_bitmap = self.read_physical_byte_for_svm(paddr);
            vmexit = (msr_bitmap >> (msr_offset & 7)) & 1 != 0;
        }

        if vmexit {
            self.svm_vmexit(SvmVmexit::Msr as i32, op as u64, 0)?;
        }
        Ok(())
    }

    /// SVM I/O intercept — Bochs svm.cc SvmInterceptIO. Checks the IOPM
    /// bitmap for `port..port+len` and triggers a VMEXIT_IO if any bit is set.
    /// `direction_in` = true for IN*, false for OUT*. String/REP handling is
    /// left out of this first cut (see svm.cc for the exact qualification
    /// bits); the VMEXIT_IO qualification encodes port+len+direction+asize,
    /// which is enough for non-string I/O.
    pub(super) fn svm_intercept_io(
        &mut self,
        port: u16,
        len: u32,
        direction_in: bool,
    ) -> super::Result<()> {
        if !self.in_svm_guest {
            return Ok(());
        }
        if !self.svm_intercept_check(SVM_INTERCEPT0_IO) {
            return Ok(());
        }
        // Read two IOPM bitmap bytes to cover cross-bit accesses (Bochs uses
        // single physical reads since read_physical_byte can't cross 4K).
        let iopm_base = self.vmcb.as_ref().map_or(0, |v| v.ctrls.iopm_base);
        let bit_addr = iopm_base + (port as u64 / 8);
        let b0 = self.read_physical_byte_for_svm(bit_addr);
        let b1 = self.read_physical_byte_for_svm(bit_addr + 1);
        let combined = ((b1 as u16) << 8) | (b0 as u16);
        let mask = ((1u32 << len) - 1) << (port & 7);
        if (combined as u32 & mask) == 0 {
            return Ok(());
        }
        // Qualification (EXITINFO1) layout mirrors Bochs svm.cc — port in bits
        // 16-31, length flag bits 4-6, asize flag bits 1-3, direction bit 0.
        let mut qualification: u64 = (port as u64) << 16;
        if direction_in {
            qualification |= 1; // SVM_VMEXIT_IO_PORTIN
        }
        match len {
            1 => qualification |= 1 << 4, // SVM_VMEXIT_IO_INSTR_LEN8
            2 => qualification |= 1 << 5, // SVM_VMEXIT_IO_INSTR_LEN16
            4 => qualification |= 1 << 6, // SVM_VMEXIT_IO_INSTR_LEN32
            _ => {}
        }
        // EXITINFO2 is the next-instruction RIP. Bochs passes RIP as-is.
        let rip = self.rip();
        self.svm_vmexit(SvmVmexit::Io as i32, qualification, rip)
    }

    /// SVM task switch intercept handler.
    /// Bochs svm.cc SvmInterceptTaskSwitch()
    pub(super) fn svm_intercept_task_switch(
        &mut self,
        tss_selector: u16,
        source: u32,
        push_error: bool,
        error_code: u32,
    ) -> super::Result<()> {
        if !self.in_svm_guest {
            return Ok(());
        }
        if !self.svm_intercept_check(SVM_INTERCEPT0_TASK_SWITCH) {
            return Ok(());
        }

        tracing::debug!("SVM VMEXIT: task switch");

        // Build EXITINFO2 qualification
        let mut qualification: u64 = error_code as u64;

        // Task switch source encoding
        const BX_TASK_FROM_IRET: u32 = 3;
        const BX_TASK_FROM_JUMP: u32 = 2;
        if source == BX_TASK_FROM_IRET {
            qualification |= 1u64 << 36;
        }
        if source == BX_TASK_FROM_JUMP {
            qualification |= 1u64 << 38;
        }
        if push_error {
            qualification |= 1u64 << 44;
        }

        if self.eflags.contains(EFlags::RF) {
            qualification |= 1u64 << 48;
        }

        self.svm_vmexit(SvmVmexit::TaskSwitch as i32, tss_selector as u64, qualification)
    }

    /// SVM PAUSE intercept handler.
    /// Bochs svm.cc SvmInterceptPAUSE()
    pub(super) fn svm_intercept_pause(&mut self) -> super::Result<()> {
        if !self.in_svm_guest {
            return Ok(());
        }

        // Read current time before mutable borrow of vmcb
        let currtime = self.system_ticks();

        // Pause filter logic — check if we should suppress the VMEXIT
        let should_suppress = if let Some(vmcb) = self.vmcb.as_mut() {
            let has_filter = vmcb.ctrls.pause_filter_threshold > 0 || vmcb.ctrls.pause_filter_count > 0;
            if has_filter {
                let time_from_last = currtime.wrapping_sub(vmcb.ctrls.last_pause_time);
                vmcb.ctrls.last_pause_time = currtime;
                if vmcb.ctrls.pause_filter_threshold > 0
                    && time_from_last > vmcb.ctrls.pause_filter_threshold as u64
                {
                    // Gap exceeds threshold — reset counter from VMCB
                    Some(true) // signal: reset counter
                } else if vmcb.ctrls.pause_filter_count > 0 {
                    vmcb.ctrls.pause_filter_count -= 1;
                    Some(false) // suppressed, no reset needed
                } else {
                    None // counter exhausted, do VMEXIT
                }
            } else {
                None // no filter, do VMEXIT
            }
        } else {
            None
        };

        match should_suppress {
            Some(true) => {
                // Reset counter from VMCB physical memory
                let count = self.vmcb_read16(SVM_CONTROL16_PAUSE_FILTER_COUNT);
                if let Some(vmcb) = self.vmcb.as_mut() {
                    vmcb.ctrls.pause_filter_count = count;
                }
                return Ok(());
            }
            Some(false) => return Ok(()), // suppressed
            None => {} // fall through to VMEXIT
        }

        self.svm_vmexit(SvmVmexit::Pause as i32, 0, 0)
    }

    /// VM_CR MSR update handler.
    /// Bochs svm.cc Svm_Update_VM_CR_MSR()
    pub(super) fn svm_update_vm_cr_msr(&mut self, val: u64) -> super::Result<()> {
        if val >> 5 != 0 {
            tracing::error!("VM_CR_MSR: attempt to set reserved bits");
            return self.exception(Exception::Gp, 0);
        }

        if (val as u32) & BX_VM_CR_MSR_SVMDIS_MASK != 0 {
            if self.efer.svme() {
                tracing::error!("VM_CR_MSR: attempt to set SVMDIS when EFER.SVME=1");
                return self.exception(Exception::Gp, 0);
            }
        }

        let mut new_val = val as u32;
        if self.msr.svm_vm_cr & BX_VM_CR_MSR_LOCK_MASK != 0 {
            // When LOCK is set, preserve LOCK and SVMDIS bits
            new_val = (new_val & 0x7)
                | (self.msr.svm_vm_cr & (BX_VM_CR_MSR_LOCK_MASK | BX_VM_CR_MSR_SVMDIS_MASK));
        }

        self.msr.svm_vm_cr = new_val;
        Ok(())
    }

    /// Helper: read a single byte from physical memory (for IOPM/MSRPM bitmaps).
    fn read_physical_byte_for_svm(&mut self, paddr: u64) -> u8 {
        if let Some((mem, cpu_ref)) = self.mem_bus_and_cpu() {
            let mut data = [0u8; 1];
            let _ = mem.read_physical_page(&[cpu_ref], paddr, 1, &mut data);
            data[0]
        } else {
            0xff // Default: all bits set = intercept everything
        }
    }

    // =====================================================================
    //  SVM instruction handlers
    // =====================================================================

    /// VMRUN instruction.
    /// Bochs svm.cc VMRUN()
    pub(super) fn svm_vmrun(&mut self, instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            tracing::error!("VMRUN: CPL != 0");
            return self.exception(Exception::Gp, 0);
        }

        // If already in SVM guest and VMRUN is intercepted, VMEXIT
        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT1_VMRUN) {
                return self.svm_vmexit(SvmVmexit::Vmrun as i32, 0, 0);
            }
        }

        // Get VMCB physical address from RAX
        let asize_mask = if instr.as64_l() != 0 { u64::MAX } else if instr.as32_l() != 0 { 0xFFFF_FFFF } else { 0xFFFF };
        let paddr = self.rax() & asize_mask;
        if (paddr & 0xFFF) != 0 {
            tracing::error!("VMRUN: VMCB address not page-aligned: {:#x}", paddr);
            return self.exception(Exception::Gp, 0);
        }
        self.set_vmcbptr(paddr);

        tracing::debug!("VMRUN VMCB ptr: {:#x}", self.vmcbptr);

        // Step 1: Save host state
        self.svm_enter_save_host_state();

        // Step 2: Load and check control fields
        if !self.svm_enter_load_check_controls() {
            return self.svm_vmexit(SVM_VMEXIT_INVALID, 0, 0);
        }

        // Step 3: Load and check guest state
        if !self.svm_enter_load_check_guest_state() {
            return self.svm_vmexit(SVM_VMEXIT_INVALID, 0, 0);
        }

        self.in_svm_guest = true;
        self.svm_gif = true;
        self.async_event = 1;

        // Step 4: Inject events
        if !self.svm_inject_events() {
            return self.svm_vmexit(SVM_VMEXIT_INVALID, 0, 0);
        }

        // Return CpuLoopRestart to restart the decode loop in guest context
        Err(super::error::CpuError::CpuLoopRestart)
    }

    /// VMMCALL instruction.
    /// Bochs svm.cc VMMCALL()
    pub(super) fn svm_vmmcall(&mut self, _instr: &Instruction) -> super::Result<()> {
        if self.efer.svme() {
            if self.in_svm_guest {
                if self.svm_intercept_check(SVM_INTERCEPT1_VMMCALL) {
                    return self.svm_vmexit(SvmVmexit::Vmmcall as i32, 0, 0);
                }
            }
        }
        self.exception(Exception::Ud, 0)
    }

    /// VMLOAD instruction — load FS/GS/TR/LDTR + MSRs from arbitrary VMCB.
    /// Bochs svm.cc VMLOAD()
    pub(super) fn svm_vmload(&mut self, instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(Exception::Gp, 0);
        }

        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT1_VMLOAD) {
                return self.svm_vmexit(SvmVmexit::Vmload as i32, 0, 0);
            }
        }

        let asize_mask = if instr.as64_l() != 0 { u64::MAX } else if instr.as32_l() != 0 { 0xFFFF_FFFF } else { 0xFFFF };
        let paddr = self.rax() & asize_mask;
        if (paddr & 0xFFF) != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let saved_vmcbptr = self.vmcbptr;
        self.set_vmcbptr(paddr);

        // Read FS, GS, TR, LDTR from target VMCB
        let fs = self.svm_segment_read(SVM_GUEST_FS_SELECTOR);
        let gs = self.svm_segment_read(SVM_GUEST_GS_SELECTOR);
        let tr = self.svm_segment_read(SVM_GUEST_TR_SELECTOR);
        let ldtr = self.svm_segment_read(SVM_GUEST_LDTR_SELECTOR);

        self.sregs[BxSegregs::Fs as usize] = fs;
        self.sregs[BxSegregs::Gs as usize] = gs;
        self.tr = tr;
        self.ldtr = ldtr;

        // Load MSRs
        self.msr.kernelgsbase = canonicalize_address(self.vmcb_read64(SVM_GUEST_KERNEL_GSBASE_MSR));
        self.msr.star = self.vmcb_read64(SVM_GUEST_STAR_MSR);
        self.msr.lstar = canonicalize_address(self.vmcb_read64(SVM_GUEST_LSTAR_MSR));
        self.msr.cstar = canonicalize_address(self.vmcb_read64(SVM_GUEST_CSTAR_MSR));
        self.msr.fmask = self.vmcb_read64(SVM_GUEST_FMASK_MSR) as u32;
        self.msr.sysenter_cs_msr = self.vmcb_read64(SVM_GUEST_SYSENTER_CS_MSR) as u32;
        self.msr.sysenter_eip_msr = canonicalize_address(self.vmcb_read64(SVM_GUEST_SYSENTER_EIP_MSR));
        self.msr.sysenter_esp_msr = canonicalize_address(self.vmcb_read64(SVM_GUEST_SYSENTER_ESP_MSR));

        // Restore original VMCB pointer
        self.set_vmcbptr(saved_vmcbptr);
        Ok(())
    }

    /// VMSAVE instruction — save FS/GS/TR/LDTR + MSRs to arbitrary VMCB.
    /// Bochs svm.cc VMSAVE()
    pub(super) fn svm_vmsave(&mut self, instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(Exception::Gp, 0);
        }

        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT1_VMSAVE) {
                return self.svm_vmexit(SvmVmexit::Vmsave as i32, 0, 0);
            }
        }

        let asize_mask = if instr.as64_l() != 0 { u64::MAX } else if instr.as32_l() != 0 { 0xFFFF_FFFF } else { 0xFFFF };
        let paddr = self.rax() & asize_mask;
        if (paddr & 0xFFF) != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let saved_vmcbptr = self.vmcbptr;
        self.set_vmcbptr(paddr);

        // Write FS, GS, TR, LDTR to target VMCB
        let fs = self.sregs[BxSegregs::Fs as usize].clone();
        let gs = self.sregs[BxSegregs::Gs as usize].clone();
        let tr = self.tr.clone();
        let ldtr = self.ldtr.clone();
        self.svm_segment_write(&fs, SVM_GUEST_FS_SELECTOR);
        self.svm_segment_write(&gs, SVM_GUEST_GS_SELECTOR);
        self.svm_segment_write(&tr, SVM_GUEST_TR_SELECTOR);
        self.svm_segment_write(&ldtr, SVM_GUEST_LDTR_SELECTOR);

        // Write MSRs
        self.vmcb_write64(SVM_GUEST_KERNEL_GSBASE_MSR, self.msr.kernelgsbase);
        self.vmcb_write64(SVM_GUEST_STAR_MSR, self.msr.star);
        self.vmcb_write64(SVM_GUEST_LSTAR_MSR, self.msr.lstar);
        self.vmcb_write64(SVM_GUEST_CSTAR_MSR, self.msr.cstar);
        self.vmcb_write64(SVM_GUEST_FMASK_MSR, self.msr.fmask as u64);
        self.vmcb_write64(SVM_GUEST_SYSENTER_CS_MSR, self.msr.sysenter_cs_msr as u64);
        self.vmcb_write64(SVM_GUEST_SYSENTER_ESP_MSR, self.msr.sysenter_esp_msr);
        self.vmcb_write64(SVM_GUEST_SYSENTER_EIP_MSR, self.msr.sysenter_eip_msr);

        // Restore original VMCB pointer
        self.set_vmcbptr(saved_vmcbptr);
        Ok(())
    }

    /// SKINIT instruction — validate prereqs, raise #UD.
    /// Hardware rarely uses this and it requires TPM support.
    /// Bochs svm.cc SKINIT() — panics with "not implemented".
    pub(super) fn svm_skinit(&mut self, _instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(Exception::Gp, 0);
        }

        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT1_SKINIT) {
                return self.svm_vmexit(SvmVmexit::Skinit as i32, 0, 0);
            }
        }

        // SKINIT requires TPM hardware — raise #UD
        tracing::warn!("SKINIT: not implemented (requires TPM)");
        self.exception(Exception::Ud, 0)
    }

    /// CLGI instruction — clear global interrupt flag.
    /// Bochs svm.cc CLGI()
    pub(super) fn svm_clgi(&mut self, _instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(Exception::Gp, 0);
        }

        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT1_CLGI) {
                return self.svm_vmexit(SvmVmexit::Clgi as i32, 0, 0);
            }
        }

        self.svm_gif = false;
        Ok(())
    }

    /// STGI instruction — set global interrupt flag.
    /// Bochs svm.cc STGI()
    pub(super) fn svm_stgi(&mut self, _instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(Exception::Gp, 0);
        }

        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT1_STGI) {
                return self.svm_vmexit(SvmVmexit::Stgi as i32, 0, 0);
            }
        }

        self.svm_gif = true;
        self.async_event = 1;
        Ok(())
    }

    /// INVLPGA instruction — invalidate TLB entry for linear address.
    /// Bochs svm.cc INVLPGA()
    pub(super) fn svm_invlpga(&mut self, instr: &Instruction) -> super::Result<()> {
        if !self.protected_mode() || !self.efer.svme() {
            return self.exception(Exception::Ud, 0);
        }
        let cpl = self.sregs[BxSegregs::Cs as usize].selector.rpl;
        if cpl != 0 {
            return self.exception(Exception::Gp, 0);
        }

        let asize_mask = if instr.as64_l() != 0 { u64::MAX } else if instr.as32_l() != 0 { 0xFFFF_FFFF } else { 0xFFFF };
        let laddr = self.rax() & asize_mask;

        if self.in_svm_guest {
            if self.svm_intercept_check(SVM_INTERCEPT0_INVLPGA) {
                return self.svm_vmexit(SvmVmexit::Invlpga as i32, laddr, 0);
            }
        }

        // Invalidate TLB entry
        self.dtlb.invlpg(laddr);
        self.itlb.invlpg(laddr);
        Ok(())
    }

}