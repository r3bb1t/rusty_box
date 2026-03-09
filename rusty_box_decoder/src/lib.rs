#![forbid(unsafe_code)]

pub mod error;
pub use error::{DecodeError, DecodeResult};

pub mod features;

/// x86 instruction decoder pipeline — mirrors Bochs `cpu/decoder/` layout.
///
/// Internal modules:
/// - `decode32` / `decode64` — 32-bit and 64-bit fetch-decode implementations
/// - `tables` — generated constants, attributes, and decoding masks
/// - `opmap` / `opmap_0f38` / `opmap_0f3a` — opcode lookup tables
/// - `x87` — x87 FPU opcode tables
pub mod decoder;

/// Core instruction representation — flattened struct with named fields.
pub mod instruction;

/// x86 opcode enumeration — one variant per distinct instruction form.
pub mod opcode;

/// Type-safe instruction enum — each opcode variant carries exactly its operands.
pub mod typed;

// Re-export key public types and functions at crate root for convenience.
pub use decoder::{decode32, decode64};
pub use decoder::decode32::fetch_decode32;
pub use decoder::decode64::fetch_decode64;
pub use decoder::tables::{BxDecodeError, SsePrefix};

#[cfg(test)]
mod tests;

pub const BX_ISA_EXTENSIONS_ARRAY_SIZE: usize = 5;

/// Complete x86 ISA feature enumeration matching Bochs `cpu/decoder/features.h`.
/// Variant order must match `X86Feature` in `features.rs` exactly.
/// All variants defined for completeness — not all used selectively.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum X86FeatureName {
    /// 386 or earlier instruction
    Isa386,
    /// FPU (x87) instruction
    IsaX87,
    /// 486 new instruction
    Isa486,
    /// Pentium new instruction
    IsaPentium,
    /// P6 new instruction
    IsaP6,
    /// MMX instruction
    IsaMmx,
    /// 3DNow! Instructions (AMD)
    Isa3dnow,
    /// 3DNow! Extensions (AMD)
    Isa3dnowExt,
    /// Debug Extensions support
    IsaDebugExtensions,
    /// VME support
    IsaVme,
    /// PSE support
    IsaPse,
    /// PAE support
    IsaPae,
    /// Global Pages support
    IsaPge,
    /// MTRR support
    IsaMtrr,
    /// PAT support
    IsaPat,
    /// SYSCALL/SYSRET in legacy mode (AMD)
    IsaSyscallSysretLegacy,
    /// SYSENTER/SYSEXIT instruction
    IsaSysenterSysexit,
    /// CLFLUSH instruction
    IsaClflush,
    /// CLFLUSHOPT instruction
    IsaClflushopt,
    /// CLWB instruction
    IsaClwb,
    /// SSE instruction
    IsaSse,
    /// SSE2 instruction
    IsaSse2,
    /// SSE3 instruction
    IsaSse3,
    /// SSSE3 instruction
    IsaSsse3,
    /// SSE4_1 instruction
    IsaSse4_1,
    /// SSE4_2 instruction
    IsaSse4_2,
    /// POPCNT instruction
    IsaPopcnt,
    /// MONITOR/MWAIT instruction
    IsaMonitorMwait,
    /// TPAUSE/UMONITOR/UMWAIT instructions
    IsaWaitpkg,
    /// MONITOR-less MWAIT extension
    IsaMonitorlessMwait,
    /// MONITORX/MWAITX instruction (AMD)
    IsaMonitorxMwaitx,
    /// Long Mode (x86-64) support
    IsaLongMode,
    /// Long Mode LAHF/SAHF instruction
    IsaLmLahfSahf,
    /// No-Execute Pages support
    IsaNx,
    /// 1Gb pages support
    Isa1gPages,
    /// CMPXCHG16B instruction
    IsaCmpxchg16b,
    /// RDTSCP instruction
    IsaRdtscp,
    /// EFER.FFXSR support (AMD)
    IsaFfxsr,
    /// XSAVE/XRSTOR extensions instruction
    IsaXsave,
    /// XSAVEOPT instruction
    IsaXsaveopt,
    /// XSAVEC instruction
    IsaXsavec,
    /// XSAVES instruction
    IsaXsaves,
    /// AES+PCLMULQDQ instructions
    IsaAesPclmulqdq,
    /// Wide vector versions of AES+PCLMULQDQ instructions
    IsaVaesVpclmulqdq,
    /// MOVBE instruction
    IsaMovbe,
    /// FS/GS BASE access instruction
    IsaFsgsbase,
    /// AVX instruction
    IsaAvx,
    /// AVX2 instruction
    IsaAvx2,
    /// AVX F16 convert instruction
    IsaAvxF16c,
    /// AVX FMA instruction
    IsaAvxFma,
    /// SSE4A instruction (AMD)
    IsaSse4a,
    /// Misaligned SSE (AMD)
    IsaMisalignedSse,
    /// LOCK CR0 access CR8 (AMD)
    IsaAltMovCr8,
    /// LZCNT instruction
    IsaLzcnt,
    /// BMI1 instruction
    IsaBmi1,
    /// BMI2 instruction
    IsaBmi2,
    /// FMA4 instruction (AMD)
    IsaFma4,
    /// XOP instruction (AMD)
    IsaXop,
    /// TBM instruction (AMD)
    IsaTbm,
    /// SVM instruction (AMD)
    IsaSvm,
    /// VMX instruction
    IsaVmx,
    /// SMX instruction
    IsaSmx,
    /// RDRAND instruction
    IsaRdrand,
    /// RDSEED instruction
    IsaRdseed,
    /// ADCX/ADOX instruction
    IsaAdx,
    /// SMAP support
    IsaSmap,
    /// SMEP support
    IsaSmep,
    /// SHA instruction
    IsaSha,
    /// SHA-512 instruction
    IsaSha512,
    /// GFNI instruction
    IsaGfni,
    /// SM3 instruction
    IsaSm3,
    /// SM4 instruction
    IsaSm4,
    /// AVX encoded IFMA Instructions
    IsaAvxIfma,
    /// AVX encoded VNNI Instructions
    IsaAvxVnni,
    /// AVX encoded VNNI-INT8 Instructions
    IsaAvxVnniInt8,
    /// AVX encoded VNNI-INT16 Instructions
    IsaAvxVnniInt16,
    /// AVX-NE-CONVERT Instructions
    IsaAvxNeConvert,
    /// AVX-512 instruction
    IsaAvx512,
    /// AVX-512DQ instruction
    IsaAvx512Dq,
    /// AVX-512 Byte/Word instruction
    IsaAvx512Bw,
    /// AVX-512 Conflict Detection instruction
    IsaAvx512Cd,
    // BX_ISA_AVX512_PF — AVX-512 Sparse Prefetch instruction (removed from Bochs)
    // BX_ISA_AVX512_ER — AVX-512 Exponential/Reciprocal instruction (removed from Bochs)
    /// AVX-512 VBMI: Vector Bit Manipulation Instructions
    IsaAvx512Vbmi,
    /// AVX-512 VBMI2: Vector Bit Manipulation Instructions
    IsaAvx512Vbmi2,
    /// AVX-512 IFMA52 Instructions
    IsaAvx512Ifma52,
    /// AVX-512 VPOPCNTD/VPOPCNTQ Instructions
    IsaAvx512Vpopcntdq,
    /// AVX-512 VNNI Instructions
    IsaAvx512Vnni,
    /// AVX-512 BITALG Instructions
    IsaAvx512Bitalg,
    /// AVX-512 VP2INTERSECT Instructions
    IsaAvx512Vp2intersect,
    /// AVX-512 BF16 Instructions
    IsaAvx512Bf16,
    /// AVX-512 FP16 Instructions
    IsaAvx512Fp16,
    /// AMX Instructions
    IsaAmx,
    /// AMX-INT8 Instructions
    IsaAmxInt8,
    /// AMX-BF16 Instructions
    IsaAmxBf16,
    /// AMX-FP16 Instructions
    IsaAmxFp16,
    /// AMX-TF32 Instructions
    IsaAmxTf32,
    /// AMX-COMPLEX Instructions
    IsaAmxComplex,
    /// AMX-MOVRS Instructions
    IsaAmxMovrs,
    /// AMX-AVX512 Instructions
    IsaAmxAvx512,
    /// AVX10.1 Instructions
    IsaAvx10_1,
    /// AVX10.2 Instructions
    IsaAvx10_2,
    /// AVX10.2 MOVRS Instructions
    IsaAvx10_2Movrs,
    /// XAPIC support
    IsaXapic,
    /// X2APIC support
    IsaX2apic,
    /// XAPIC Extensions support (AMD)
    IsaXapicExt,
    /// PCID support
    IsaPcid,
    /// INVPCID instruction
    IsaInvpcid,
    /// TSC-Adjust MSR
    IsaTscAdjust,
    /// TSC-Deadline
    IsaTscDeadline,
    /// FOPCODE Deprecation - FOPCODE update on unmasked x87 exception only
    IsaFopcodeDeprecation,
    /// FCS/FDS Deprecation
    IsaFcsFdsDeprecation,
    /// FDP Deprecation - FDP update on unmasked x87 exception only
    IsaFdpDeprecation,
    /// User-Mode Protection Keys
    IsaPku,
    /// Supervisor-Mode Protection Keys
    IsaPks,
    /// User-Mode Instructions Prevention
    IsaUmip,
    /// RDPID Support
    IsaRdpid,
    /// Translation Cache Extensions (TCE) support (AMD)
    IsaTce,
    /// CLZERO instruction support (AMD)
    IsaClzero,
    /// Report SCA Mitigations in CPUID
    IsaScaMitigations,
    /// Control Flow Enforcement
    IsaCet,
    /// Non-Serializing version of WRMSR
    IsaWrmsrns,
    /// Immediate forms of RDMSR and WRMSRNS
    IsaMsrImm,
    /// CMPccXADD instructions
    IsaCmpccxadd,
    /// SERIALIZE instruction
    IsaSerialize,
    /// Linear Address Space Separation support
    IsaLass,
    /// 57-bit Virtual Address and 5-level paging support
    IsaLa57,
    /// User Level Interrupts support
    IsaUintr,
    /// Flexible UIRET support
    IsaFlexibleUiret,
    /// MOVDIRI instruction support
    IsaMovdiri,
    /// MOVDIR64B instruction support
    IsaMovdir64b,
    /// RDMSRLIST/WRMSRLIST instructions support
    IsaMsrlist,
    /// RAO-INT instructions support
    IsaRaoInt,
    /// MOVRS instructions support
    IsaMovrs,
}

/// Segment register encoding
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BxSegregs {
    Es = 0,
    Cs = 1,
    Ss = 2,
    Ds = 3,
    Fs = 4,
    Gs = 5,
    // NULL now has to fit in 3 bits.
    Null = 7,
}

/// Returns `true` if `seg` encodes the null segment register (value 7).
pub fn is_null_seg_reg(seg: u8) -> bool {
    seg == BxSegregs::Null as _
}

impl BxSegregs {
    /// Convert from raw u8 to BxSegregs (const-compatible).
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => BxSegregs::Es,
            1 => BxSegregs::Cs,
            2 => BxSegregs::Ss,
            3 => BxSegregs::Ds,
            4 => BxSegregs::Fs,
            5 => BxSegregs::Gs,
            7 => BxSegregs::Null,
            _ => BxSegregs::Ds,
        }
    }
}

impl From<u8> for BxSegregs {
    fn from(val: u8) -> Self {
        BxSegregs::from_u8(val)
    }
}

pub const BX_GENERAL_REGISTERS: usize = 16;

pub const BX_16BIT_REG_IP: usize = BX_GENERAL_REGISTERS;
pub const BX_32BIT_REG_EIP: usize = BX_GENERAL_REGISTERS;
pub const BX_64BIT_REG_RIP: usize = BX_GENERAL_REGISTERS;

pub const BX_32BIT_REG_SSP: usize = BX_GENERAL_REGISTERS + 1;
pub const BX_64BIT_REG_SSP: usize = BX_GENERAL_REGISTERS + 1;

pub const BX_TMP_REGISTER: usize = BX_GENERAL_REGISTERS + 2;
pub const BX_NIL_REGISTER: usize = BX_GENERAL_REGISTERS + 3;

pub const BX_XMM_REGISTERS: usize = 32;

#[cfg(test)]
mod test_call_decode;
