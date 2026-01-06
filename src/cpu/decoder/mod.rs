pub mod error;
pub use error::{DecodeError, DecodeResult};

pub mod disasm;
pub mod features;
pub mod fetchdecode;
pub mod fetchdecode32;
pub mod fetchdecode64;
pub mod fetchdecode_generated;
pub mod fetchdecode_opmap;
pub mod fetchdecode_opmap_0f38;
pub mod fetchdecode_opmap_0f3a;
//pub mod fetchdecode_opmap_after_sed;
pub mod instr;

pub(super) mod fetchdecode_x87;

pub mod instr_generated;

pub mod simple_decoder;

pub mod ia_opcodes;

// Const-compatible decoders for compile-time instruction decoding
pub mod const_fetchdecode64;
pub mod const_fetchdecode32;

#[cfg(test)]
mod tests;

pub const BX_ISA_EXTENSIONS_ARRAY_SIZE: usize = 5;

#[allow(unused)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum X86FeatureName {
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
    /// SYSCALL/SYSRET in legacy mode (AMD) ,
    IsaSyscallSysretLegacy,
    /// SYSENTER/SYSEXIT instruction
    IsaSysenterSysexit,
    /// CLFLUSH instruction
    IsaClflush,
    /// CLFLUSHOPT instruction
    IsaClflushopt,
    /// CLWB instruction
    IsaClwb,
    /// SSE  instruction
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
    /// Long Mode LAH,F/SAHF instruction
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
    /// AVX encoded IFMA Instructions ,
    IsaAvxIfma,
    /// AVX encoded VNNI Instructions ,
    IsaAvxVnni,
    /// AVX encoded VNNI-INT8 Instructions ,
    IsaAvxVnniInt8,
    /// AVX encoded VNNI-INT16 Instructions ,
    IsaAvxVnniInt16,
    /// AVX-NE-CONVERT Instructions
    IsaAvxNeConvert,
    /// AVX-512 instruction
    IsaAvx512,
    /// AVX-512DQ instruction
    IsaAvx512Dq,
    /// AVX-512 Byte/Word instruction ,
    IsaAvx512Bw,
    /// AVX-512 Conflict Detection instruction
    IsaAvx512Cd,
    //BX_ISA_AVX512_PF, "avx512pf")                             /* AVX-512 Sparse Prefetch instruction */
    //BX_ISA_AVX512_ER, "avx512er")                             /* AVX-512 Exponential/Reciprocal instruction */
    /* AVX-512 VBMI : Vector Bit Manipulation Instructions */
    IsaAvx512Vbmi,
    /// AVX-512 VBMI2 : Vector Bit Manipulation Instructions
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

/// segment register encoding
#[derive(Debug, Clone, Copy)]
pub(crate) enum BxSegregs {
    Es = 0,
    Cs = 1,
    Ss = 2,
    Ds = 3,
    Fs = 4,
    Gs = 5,
    // NULL now has to fit in 3 bits.
    Null = 7,
}

pub fn is_null_seg_reg(seg: u8) -> bool {
    seg == BxSegregs::Null as _
}

#[derive(Debug)]
enum BxRegs8L {
    Bx8bitRegAl,
    Bx8bitRegCl,
    Bx8bitRegDl,
    Bx8bitRegBl,
    Bx8bitRegSpl,
    Bx8bitRegBpl,
    Bx8bitRegSil,
    Bx8bitRegDil,

    Bx32bitRegR8,
    Bx32bitRegR9,
    Bx32bitRegR10,
    Bx32bitRegR11,
    Bx32bitRegR12,
    Bx32bitRegR13,
    Bx32bitRegR14,
    Bx32bitRegR15,
}

#[derive(Debug)]
enum BxRegs8H {
    Ah,
    Ch,
    Dh,
    Bh,
}

#[derive(Debug, Clone, Copy)]
enum BxRegs16 {
    Ax,
    Cx,
    Dx,
    Bx,
    Sp,
    Bp,
    Si,
    Di,

    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

#[derive(Debug)]
enum BxRegs32 {
    Eax,
    Ecx,
    Edx,
    Ebx,
    Esp,
    Ebp,
    Esi,
    Edi,

    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

#[derive(Debug)]
enum BxRegs64 {
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

pub const BX_GENERAL_REGISTERS: usize = 16;

pub(super) const BX_16BIT_REG_IP: usize = BX_GENERAL_REGISTERS;
pub(super) const BX_32BIT_REG_EIP: usize = BX_GENERAL_REGISTERS;
pub(super) const BX_64BIT_REG_RIP: usize = BX_GENERAL_REGISTERS;

pub(super) const BX_32BIT_REG_SSP: usize = BX_GENERAL_REGISTERS + 1;
pub(super) const BX_64BIT_REG_SSP: usize = BX_GENERAL_REGISTERS + 1;

pub(super) const BX_TMP_REGISTER: usize = BX_GENERAL_REGISTERS + 2;
pub(super) const BX_NIL_REGISTER: usize = (BX_GENERAL_REGISTERS + 3) as _;

#[derive(Debug)]
pub(super) enum OpmaskRegs {
    K0,
    K1,
    K2,
    K3,
    K4,
    K5,
    K6,
    K7,
}

// AVX Registers
#[derive(Debug)]
pub(super) enum BxAvxVectorLength {
    NoVl,
    Vl128 = 1,
    Vl256 = 2,
    Vl512 = 4,
}

pub const BX_SUPPORT_EVEX: u8 = BxAvxVectorLength::Vl512 as _;

pub(crate) const BX_XMM_REGISTERS: usize = 32;

const BX_VECTOR_TMP_REGISTER: usize = BX_XMM_REGISTERS;
