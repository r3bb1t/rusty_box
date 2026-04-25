// cpu/cpudb/amd/ryzen.cc ported to Rust.
//
// Mirrors Bochs' AMD Ryzen (Zen gen 1) CPU model: "AuthenticAMD" vendor,
// family 0x17, model 0x01, stepping 0x01. Used as the first AMD CPU model
// in rusty_box so SVM / CLZERO / SSE4a / XAPIC_EXT paths have a target
// to run against.

use crate::cpu::cpuid::BxCpuIdTrait;
use crate::cpu::decoder::{features::X86Feature, BX_ISA_EXTENSIONS_ARRAY_SIZE};

use bitflags::bitflags;

// Shared Intel-leaf bitflags live under cpudb/intel — rather than copy them
// we import and reuse, since the flag definitions come from Intel SDM /
// AMD APM and are identical across vendors.
use super::super::intel::core_i7_skylake::{
    CpuIdStd1Ecx, CpuIdStd1Edx, CpuIdStd7Ebx, CpuIdExt1Ecx, CpuIdExt1Edx,
};

// ─── Leaf-1 ECX feature base for Ryzen ────────────────────────────────────
// Bochs ryzen.cc get_std_cpuid_leaf_1_ecx.
const LEAF1_ECX_BASE: CpuIdStd1Ecx = CpuIdStd1Ecx::SSE3
    .union(CpuIdStd1Ecx::PCLMULQDQ)
    .union(CpuIdStd1Ecx::MONITOR_MWAIT)
    .union(CpuIdStd1Ecx::SSSE3)
    .union(CpuIdStd1Ecx::FMA)
    .union(CpuIdStd1Ecx::CMPXCHG16B)
    .union(CpuIdStd1Ecx::SSE4_1)
    .union(CpuIdStd1Ecx::SSE4_2)
    .union(CpuIdStd1Ecx::MOVBE)
    .union(CpuIdStd1Ecx::POPCNT)
    .union(CpuIdStd1Ecx::AES)
    .union(CpuIdStd1Ecx::XSAVE)
    .union(CpuIdStd1Ecx::AVX)
    .union(CpuIdStd1Ecx::AVX_F16C)
    .union(CpuIdStd1Ecx::RDRAND);

// Bochs ryzen.cc get_std_cpuid_leaf_1_edx — AMD omits DEBUG_STORE / ACPI /
// HT / THERMAL_MONITOR / PBE that the Intel Skylake-X table sets.
const LEAF1_EDX_BASE: CpuIdStd1Edx = CpuIdStd1Edx::X87
    .union(CpuIdStd1Edx::VME)
    .union(CpuIdStd1Edx::DEBUG_EXTENSIONS)
    .union(CpuIdStd1Edx::PSE)
    .union(CpuIdStd1Edx::TSC)
    .union(CpuIdStd1Edx::MSR)
    .union(CpuIdStd1Edx::PAE)
    .union(CpuIdStd1Edx::MCE)
    .union(CpuIdStd1Edx::CMPXCHG8B)
    .union(CpuIdStd1Edx::APIC)
    .union(CpuIdStd1Edx::SYSENTER_SYSEXIT)
    .union(CpuIdStd1Edx::MTRR)
    .union(CpuIdStd1Edx::GLOBAL_PAGES)
    .union(CpuIdStd1Edx::MCA)
    .union(CpuIdStd1Edx::CMOV)
    .union(CpuIdStd1Edx::PAT)
    .union(CpuIdStd1Edx::PSE36)
    .union(CpuIdStd1Edx::CLFLUSH)
    .union(CpuIdStd1Edx::MMX)
    .union(CpuIdStd1Edx::FXSAVE_FXRSTOR)
    .union(CpuIdStd1Edx::SSE)
    .union(CpuIdStd1Edx::SSE2);

// Bochs ryzen.cc get_std_cpuid_leaf_7_ebx — AMD Zen gen1 does not advertise
// AVX-512 / INVPCID / SGX / RTM / MPX / CLWB / PROCESSOR_TRACE.
const LEAF7_EBX_BASE: CpuIdStd7Ebx = CpuIdStd7Ebx::FSGSBASE
    .union(CpuIdStd7Ebx::BMI1)
    .union(CpuIdStd7Ebx::AVX2)
    .union(CpuIdStd7Ebx::SMEP)
    .union(CpuIdStd7Ebx::BMI2)
    .union(CpuIdStd7Ebx::ERMS)
    .union(CpuIdStd7Ebx::RDSEED)
    .union(CpuIdStd7Ebx::ADX)
    .union(CpuIdStd7Ebx::SMAP)
    .union(CpuIdStd7Ebx::CLFLUSHOPT)
    .union(CpuIdStd7Ebx::SHA);

// Bochs ryzen.cc get_ext_cpuid_leaf_1_ecx.
const LEAF8_0000_0001_ECX_BASE: CpuIdExt1Ecx = CpuIdExt1Ecx::LAHF_SAHF
    .union(CpuIdExt1Ecx::CMP_LEGACY)
    .union(CpuIdExt1Ecx::SVM)
    .union(CpuIdExt1Ecx::EXT_APIC_SPACE)
    .union(CpuIdExt1Ecx::ALT_MOV_CR8)
    .union(CpuIdExt1Ecx::LZCNT)
    .union(CpuIdExt1Ecx::SSE4A)
    .union(CpuIdExt1Ecx::MISALIGNED_SSE)
    .union(CpuIdExt1Ecx::PREFETCHW);

// AMD extends EDX with SYSCALL, NX, MMX_EXT, MMX_AMD, FFXSR, PAGE_1G,
// RDTSCP, LONG_MODE, 3DNOW_EXT, 3DNOW. Skylake-X only declares SYSCALL /
// NX / 1GB / RDTSCP / LONG_MODE in CpuIdExt1Edx — add the remaining bits
// inline as raw flag values when the feature bitflags doesn't carry them.
const LEAF8_0000_0001_EDX_BASE: CpuIdExt1Edx = CpuIdExt1Edx::SYSCALL_SYSRET
    .union(CpuIdExt1Edx::NX)
    .union(CpuIdExt1Edx::PAGES_1G)
    .union(CpuIdExt1Edx::RDTSCP)
    .union(CpuIdExt1Edx::LONG_MODE);

// AMD-specific bits not represented in the Intel bitflags (MMX_EXT, MMX,
// FXSR, PSE36, MCA, CMOV, PAT, SYSCALL, NX, FFXSR, 1GB, RDTSCP, LongMode,
// 3DNOW_EXT, 3DNOW). Bochs' get_ext_cpuid_leaf_1_edx returns roughly the
// STD leaf-1 EDX low bits ORed with these. Construct the raw u32 here.
const LEAF8_0000_0001_EDX_RAW_EXTRA: u32 =
    (1 << 11) |  // SYSCALL (also in CpuIdExt1Edx::SYSCALL_SYSRET)
    (1 << 19) |  // MMX_EXT (AMD-specific)
    (1 << 22) |  // MMX_EXT_AMD
    (1 << 25) |  // FFXSR
    (1 << 31);   // 3DNOW (legacy, still reported on AMD)

// Helper mirroring Bochs bx_cpuid_t::enable_cpu_extension.
fn enable_extension(bitmask: &mut [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE], feature: X86Feature) {
    let idx = feature as usize;
    bitmask[idx / 32] |= 1 << (idx % 32);
}

// Re-expose additional extended-feature bitflags local to this model so the
// CPUID caller has named constants. Could be added to the shared intel
// module later, but keeping AMD-local avoids polluting Intel's file.
bitflags! {
    /// CPUID Leaf 0x8000000A EDX — SVM feature identification. Bochs cpuid.h
    /// BX_CPUID_SVM_*. Used to build the EDX return value for leaf 0x8000000A.
    #[derive(Debug, Clone, Copy)]
    pub struct SvmLeafEdx: u32 {
        const NESTED_PAGING         = 1 << 0;
        const LBR_VIRTUALIZATION    = 1 << 1;
        const SVM_LOCK              = 1 << 2;
        const NRIP_SAVE             = 1 << 3;
        const TSCRATE               = 1 << 4;
        const VMCB_CLEAN_BITS       = 1 << 5;
        const FLUSH_BY_ASID         = 1 << 6;
        const DECODE_ASSIST         = 1 << 7;
        const PAUSE_FILTER          = 1 << 10;
        const PAUSE_FILTER_THRESHOLD = 1 << 12;
    }
}

#[derive(Debug)]
pub struct AmdRyzen {}

impl BxCpuIdTrait for AmdRyzen {
    fn get_name(&self) -> &'static str {
        "amd_ryzen"
    }

    /// AMD has no VMX.
    fn get_vmx_extensions_bitmask(&self) -> Option<crate::cpu::cpuid::VMXExtensions> {
        None
    }

    /// Bochs ryzen.cc get_svm_extensions_bitmask.
    fn get_svm_extensions_bitmask(&self) -> Option<crate::cpu::cpuid::SVMExtensions> {
        use crate::cpu::cpuid::SVMExtensions;
        Some(
            SVMExtensions::NestedPaging
                | SVMExtensions::LbrVirtualization
                | SVMExtensions::SvmLock
                | SVMExtensions::NripSave
                | SVMExtensions::FlushByAsid
                | SVMExtensions::PauseFilter
                | SVMExtensions::PauseFilterThreshold,
        )
    }

    fn sanity_checks(&self) -> crate::cpu::error::Result<()> {
        Ok(())
    }

    fn new() -> Self {
        Self {}
    }

    /// Mirrors Bochs ryzen.cc enable_cpu_extension calls in ryzen_t::ryzen_t().
    fn get_isa_extensions_bitmask(&self) -> [u32; BX_ISA_EXTENSIONS_ARRAY_SIZE] {
        let mut b = [0u32; BX_ISA_EXTENSIONS_ARRAY_SIZE];
        enable_extension(&mut b, X86Feature::Isa386);
        enable_extension(&mut b, X86Feature::IsaX87);
        enable_extension(&mut b, X86Feature::Isa486);
        enable_extension(&mut b, X86Feature::IsaPentium);
        enable_extension(&mut b, X86Feature::IsaP6);
        enable_extension(&mut b, X86Feature::IsaMmx);
        enable_extension(&mut b, X86Feature::IsaSyscallSysretLegacy);
        enable_extension(&mut b, X86Feature::IsaSysenterSysexit);
        enable_extension(&mut b, X86Feature::IsaClflush);
        enable_extension(&mut b, X86Feature::IsaDebugExtensions);
        enable_extension(&mut b, X86Feature::IsaVme);
        enable_extension(&mut b, X86Feature::IsaPse);
        enable_extension(&mut b, X86Feature::IsaPae);
        enable_extension(&mut b, X86Feature::IsaPge);
        enable_extension(&mut b, X86Feature::IsaMtrr);
        enable_extension(&mut b, X86Feature::IsaPat);
        enable_extension(&mut b, X86Feature::IsaXapic);
        enable_extension(&mut b, X86Feature::IsaLongMode);
        enable_extension(&mut b, X86Feature::IsaLmLahfSahf);
        enable_extension(&mut b, X86Feature::IsaCmpxchg16b);
        enable_extension(&mut b, X86Feature::IsaNx);
        enable_extension(&mut b, X86Feature::IsaSse);
        enable_extension(&mut b, X86Feature::IsaSse2);
        enable_extension(&mut b, X86Feature::IsaSse3);
        enable_extension(&mut b, X86Feature::IsaSsse3);
        enable_extension(&mut b, X86Feature::IsaSse4_1);
        enable_extension(&mut b, X86Feature::IsaSse4_2);
        enable_extension(&mut b, X86Feature::IsaLzcnt);
        enable_extension(&mut b, X86Feature::IsaPopcnt);
        enable_extension(&mut b, X86Feature::IsaSse4a);
        enable_extension(&mut b, X86Feature::IsaMonitorMwait);
        enable_extension(&mut b, X86Feature::IsaRdtscp);
        enable_extension(&mut b, X86Feature::IsaXsave);
        enable_extension(&mut b, X86Feature::IsaXsaveopt);
        enable_extension(&mut b, X86Feature::IsaXsavec);
        enable_extension(&mut b, X86Feature::IsaXsaves);
        enable_extension(&mut b, X86Feature::IsaAesPclmulqdq);
        enable_extension(&mut b, X86Feature::IsaAvx);
        enable_extension(&mut b, X86Feature::IsaAvxF16c);
        enable_extension(&mut b, X86Feature::IsaAvx2);
        enable_extension(&mut b, X86Feature::IsaAvxFma);
        enable_extension(&mut b, X86Feature::IsaMovbe);
        enable_extension(&mut b, X86Feature::IsaRdrand);
        enable_extension(&mut b, X86Feature::IsaRdseed);
        enable_extension(&mut b, X86Feature::IsaBmi1);
        enable_extension(&mut b, X86Feature::IsaBmi2);
        enable_extension(&mut b, X86Feature::IsaFsgsbase);
        enable_extension(&mut b, X86Feature::IsaSmep);
        enable_extension(&mut b, X86Feature::IsaAdx);
        enable_extension(&mut b, X86Feature::IsaSmap);
        enable_extension(&mut b, X86Feature::IsaSha);
        enable_extension(&mut b, X86Feature::IsaClflushopt);
        enable_extension(&mut b, X86Feature::IsaSvm);
        enable_extension(&mut b, X86Feature::IsaFfxsr);
        enable_extension(&mut b, X86Feature::Isa1gPages);
        enable_extension(&mut b, X86Feature::IsaMisalignedSse);
        enable_extension(&mut b, X86Feature::IsaAltMovCr8);
        enable_extension(&mut b, X86Feature::IsaXapicExt);
        enable_extension(&mut b, X86Feature::IsaClzero);
        enable_extension(&mut b, X86Feature::IsaMonitorxMwaitx);
        b
    }

    /// CPUID leaves mirroring Bochs ryzen.cc. Brand string
    /// "AMD Ryzen 7 1700 Eight-Core Processor" (48 bytes).
    fn get_cpuid_leaf(&self, eax: u32, ecx: u32) -> (u32, u32, u32, u32) {
        match eax {
            // ── Basic leaf 0 ─────────────────────────────────────────────
            0x00000000 => (
                0x0000000D, // max basic leaf
                0x68747541, // "Auth"
                0x444D4163, // "cAMD"
                0x69746E65, // "enti" → "AuthenticAMD"
            ),

            // ── Leaf 1: family=0x17 (Zen), model=0x01, stepping=0x01 ─────
            0x00000001 => (
                0x00800F11,
                // EBX: brand ID | CLFLUSH size 8 * 8 = 64 bytes | 1 log proc | APIC 0
                0x00010800,
                LEAF1_ECX_BASE.bits(),
                LEAF1_EDX_BASE.bits(),
            ),

            // ── Leaf 5 MONITOR/MWAIT — Bochs ryzen.cc uses the default zero-latency leaf. ──
            0x00000005 => (0x00000040, 0x00000040, 0x00000003, 0x00000000),

            // ── Leaf 6 Thermal/Power — Bochs ryzen.cc fixed value. ──
            0x00000006 => (0x00000004, 0x00000000, 0x00000001, 0x00000000),

            // ── Leaf 7 Structured Extended Features ─────────────────────
            0x00000007 => match ecx {
                0 => (0, LEAF7_EBX_BASE.bits(), 0, 0),
                _ => (0, 0, 0, 0),
            },

            // ── Leaf 0xD XSAVE — defer to generic reserved leaf for now;
            //    full XSAVE area sizes live in the CPUID-leaf handler in
            //    soft_int.rs / proc_ctrl.rs when XSAVE is exercised.
            0x0000000D => (0, 0, 0, 0),

            // ── Extended leaves ──────────────────────────────────────────
            0x80000000 => (
                0x8000001F, // max extended leaf
                0x68747541, // "Auth"
                0x444D4163, // "cAMD"
                0x69746E65, // "enti"
            ),

            0x80000001 => (
                0x00800F11,
                0x20000000, // Package Type = Zen AM4 (bits 31:28)
                LEAF8_0000_0001_ECX_BASE.bits(),
                LEAF8_0000_0001_EDX_BASE.bits() | LEAF8_0000_0001_EDX_RAW_EXTRA,
            ),

            // Brand string — "AMD Ryzen 7 1700 Eight-Core Processor          "
            // 48 bytes split into three 16-byte leaves in little-endian order.
            0x80000002 => (0x20444D41, 0x657A7952, 0x2037206E, 0x30303731),
            0x80000003 => (0x67694520, 0x432D7468, 0x2065726F, 0x636F7250),
            0x80000004 => (0x6F737365, 0x20202072, 0x20202020, 0x00202020),

            // Leaf 0x80000005 L1 cache and TLB — Bochs ryzen.cc fixed.
            0x80000005 => (0xFF40FF40, 0xFF40FF40, 0x20080140, 0x20080140),
            // Leaf 0x80000006 L2 cache — Bochs ryzen.cc fixed.
            0x80000006 => (0x26006400, 0x66006400, 0x02006140, 0x0040F040),
            // Leaf 0x80000007 APM — invariant TSC present.
            0x80000007 => (0, 0, 0, 0x00000100),
            // Leaf 0x80000008 address sizes.
            0x80000008 => (
                0x00003028, // 48-bit phys, 48-bit linear
                0x00001007, // CLZERO/IBPB/IBRS flags per Bochs ryzen
                0x00000000,
                0x00000000,
            ),
            // Leaf 0x8000000A SVM feature identification.
            0x8000000A => {
                let svm_features = SvmLeafEdx::NESTED_PAGING
                    | SvmLeafEdx::LBR_VIRTUALIZATION
                    | SvmLeafEdx::SVM_LOCK
                    | SvmLeafEdx::NRIP_SAVE
                    | SvmLeafEdx::FLUSH_BY_ASID
                    | SvmLeafEdx::PAUSE_FILTER
                    | SvmLeafEdx::PAUSE_FILTER_THRESHOLD;
                (
                    0x00000001, // SVM revision
                    0x00008000, // # of ASIDs
                    0,
                    svm_features.bits(),
                )
            }

            // Leaf 0x80000019 — 1G Page TLB Identifiers (Bochs ryzen.cc).
            0x80000019 => (0xF040F040, 0, 0, 0),

            // Leaf 0x8000001A — Performance Optimization Identifiers
            // (Bochs ryzen.cc). EAX bit 0 = FP128 ops native, bit 1 =
            // MOVU faster than MOVL/MOVH.
            0x8000001A => (0x00000003, 0, 0, 0),

            // Leaf 0x8000001B — Instruction Based Sampling Identifiers
            // (Bochs ryzen.cc). IBS itself is not implemented in
            // rusty_box; we return Bochs's static feature mask so guests
            // that probe for IBS see the same answer they would on real
            // Bochs.
            0x8000001B => (0x000003FF, 0, 0, 0),

            // Leaf 0x8000001C — Lightweight Profiling Capabilities
            // (Bochs ryzen.cc returns all zeros — LWP not implemented).
            0x8000001C => (0, 0, 0, 0),

            // Leaf 0x8000001D — Cache Properties (multi-subleaf, Bochs
            // ryzen.cc). Sub-leaves 0..=3 describe L1D / L1I / L2 / L3.
            0x8000001D => match ecx {
                0 => (0x00004121, 0x01C0003F, 0x0000003F, 0x00000000),
                1 => (0x00004122, 0x00C0003F, 0x000000FF, 0x00000000),
                2 => (0x00004143, 0x01C0003F, 0x000003FF, 0x00000002),
                3 => (0x0001C163, 0x03C0003F, 0x00001FFF, 0x00000001),
                _ => (0, 0, 0, 0),
            },

            // Leaf 0x8000001E — Topology Extensions (Bochs ryzen.cc).
            // EBX [15:8] = (ncores - 1); single-core configuration here.
            0x8000001E => (0, 0x00000000, 0, 0),

            // Leaf 0x8000001F — Encrypted Memory Capabilities (Bochs
            // ryzen.cc). EAX = SME/SEV/SEV-ES/Secure-NPT/SME-Coherent
            // / Hardware-Enforced-Cache-Coherency feature mask;
            // EBX = bit-position info; ECX = number of encrypted guests.
            0x8000001F => (0x00000007, 0x0000016F, 0x0000000F, 0),

            // Other ext leaves reserved / zero — Bochs ryzen.cc falls back to get_reserved_leaf.
            _ => (0, 0, 0, 0),
        }
    }
}
