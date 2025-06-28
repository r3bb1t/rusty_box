#[allow(non_camel_case_types)]
#[derive(Debug)]
pub(crate) enum X86Feature {
    Isa386,                   /* 386 or earlier instruction */
    IsaX87,                   /* FPU (x87) instruction */
    Isa486,                   /* 486 new instruction */
    IsaPENTIUM,               /* Pentium new instruction */
    IsaP6,                    /* P6 new instruction */
    IsaMMX,                   /* MMX instruction */
    Isa3DNOW,                 /* 3DNow! Instructions (AMD) */
    Isa3DNOW_EXT,             /* 3DNow! Extensions (AMD) */
    IsaDEBUG_EXTENSIONS,      /* Debug Extensions support */
    IsaVME,                   /* VME support */
    IsaPSE,                   /* PSE support */
    IsaPAE,                   /* PAE support */
    IsaPGE,                   /* Global Pages support */
    IsaMTRR,                  /* MTRR support */
    IsaPAT,                   /* PAT support */
    IsaSYSCALL_SYSRET_LEGACY, /* SYSCALL/SYSRET in legacy mode (AMD) */
    IsaSYSENTER_SYSEXIT,      /* SYSENTER/SYSEXIT instruction */
    IsaCLFLUSH,               /* CLFLUSH instruction */
    IsaCLFLUSHOPT,            /* CLFLUSHOPT instruction */
    IsaCLWB,                  /* CLWB instruction */
    IsaSSE,                   /* SSE  instruction */
    IsaSSE2,                  /* SSE2 instruction */
    IsaSSE3,                  /* SSE3 instruction */
    IsaSSSE3,                 /* SSSE3 instruction */
    IsaSSE4_1,                /* SSE4_1 instruction */
    IsaSSE4_2,                /* SSE4_2 instruction */
    IsaPOPCNT,                /* POPCNT instruction */
    IsaMONITOR_MWAIT,         /* MONITOR/MWAIT instruction */
    IsaWAITPKG,               /* TPAUSE/UMONITOR/UMWAIT instructions */
    IsaMONITORLESS_MWAIT,     /* MONITOR-less MWAIT extension */
    IsaMONITORX_MWAITX,       /* MONITORX/MWAITX instruction (AMD) */
    IsaLONG_MODE,             /* Long Mode (x86-64) support */
    IsaLM_LAHF_SAHF,          /* Long Mode LAHF/SAHF instruction */
    IsaNX,                    /* No-Execute Pages support */
    Isa1G_PAGES,              /* 1Gb pages support */
    IsaCMPXCHG16B,            /* CMPXCHG16B instruction */
    IsaRDTSCP,                /* RDTSCP instruction */
    IsaFFXSR,                 /* EFER.FFXSR support (AMD) */
    IsaXSAVE,                 /* XSAVE/XRSTOR extensions instruction */
    IsaXSAVEOPT,              /* XSAVEOPT instruction */
    IsaXSAVEC,                /* XSAVEC instruction */
    IsaXSAVES,                /* XSAVES instruction */
    IsaAES_PCLMULQDQ,         /* AES+PCLMULQDQ instructions */
    IsaVAES_VPCLMULQDQ,       /* Wide vector versions of AES+PCLMULQDQ instructions */
    IsaMOVBE,                 /* MOVBE instruction */
    IsaFSGSBASE,              /* FS/GS BASE access instruction */
    IsaAVX,                   /* AVX instruction */
    IsaAVX2,                  /* AVX2 instruction */
    IsaAVX_F16C,              /* AVX F16 convert instruction */
    IsaAVX_FMA,               /* AVX FMA instruction */
    IsaSSE4A,                 /* SSE4A instruction (AMD) */
    IsaMISALIGNED_SSE,        /* Misaligned SSE (AMD) */
    IsaALT_MOV_CR8,           /* LOCK CR0 access CR8 (AMD) */
    IsaLZCNT,                 /* LZCNT instruction */
    IsaBMI1,                  /* BMI1 instruction */
    IsaBMI2,                  /* BMI2 instruction */
    IsaFMA4,                  /* FMA4 instruction (AMD) */
    IsaXOP,                   /* XOP instruction (AMD) */
    IsaTBM,                   /* TBM instruction (AMD) */
    IsaSVM,                   /* SVM instruction (AMD) */
    IsaVMX,                   /* VMX instruction */
    IsaSMX,                   /* SMX instruction */
    IsaRDRAND,                /* RDRAND instruction */
    IsaRDSEED,                /* RDSEED instruction */
    IsaADX,                   /* ADCX/ADOX instruction */
    IsaSMAP,                  /* SMAP support */
    IsaSMEP,                  /* SMEP support */
    IsaSHA,                   /* SHA instruction */
    IsaSHA512,                /* SHA-512 instruction */
    IsaGFNI,                  /* GFNI instruction */
    IsaSM3,                   /* SM3 instruction */
    IsaSM4,                   /* SM4 instruction */
    IsaAVX_IFMA,              /* AVX encoded IFMA Instructions */
    IsaAVX_VNNI,              /* AVX encoded VNNI Instructions */
    IsaAVX_VNNI_INT8,         /* AVX encoded VNNI-INT8 Instructions */
    IsaAVX_VNNI_INT16,        /* AVX encoded VNNI-INT16 Instructions */
    IsaAVX_NE_CONVERT,        /* AVX-NE-CONVERT Instructions */
    IsaAVX512,                /* AVX-512 instruction */
    IsaAVX512_DQ,             /* AVX-512DQ instruction */
    IsaAVX512_BW,             /* AVX-512 Byte/Word instruction */
    IsaAVX512_CD,             /* AVX-512 Conflict Detection instruction */
    //                             ,/* AVX-512 Sparse Prefetch instruction */
    //                             ,/* AVX-512 Exponential/Reciprocal instruction */
    IsaAVX512_VBMI,         /* AVX-512 VBMI : Vector Bit Manipulation Instructions */
    IsaAVX512_VBMI2,        /* AVX-512 VBMI2 : Vector Bit Manipulation Instructions */
    IsaAVX512_IFMA52,       /* AVX-512 IFMA52 Instructions */
    IsaAVX512_VPOPCNTDQ,    /* AVX-512 VPOPCNTD/VPOPCNTQ Instructions */
    IsaAVX512_VNNI,         /* AVX-512 VNNI Instructions */
    IsaAVX512_BITALG,       /* AVX-512 BITALG Instructions */
    IsaAVX512_VP2INTERSECT, /* AVX-512 VP2INTERSECT Instructions */
    IsaAVX512_BF16,         /* AVX-512 BF16 Instructions */
    IsaAVX512_FP16,         /* AVX-512 FP16 Instructions */
    IsaAMX,                 /* AMX Instructions */
    IsaAMX_INT8,            /* AMX-INT8 Instructions */
    IsaAMX_BF16,            /* AMX-BF16 Instructions */
    IsaAMX_FP16,            /* AMX-FP16 Instructions */
    IsaAMX_TF32,            /* AMX-TF32 Instructions */
    IsaAMX_COMPLEX,         /* AMX-COMPLEX Instructions */
    IsaAMX_MOVRS,           /* AMX-MOVRS Instructions */
    IsaAMX_AVX512,          /* AMX-AVX512 Instructions */
    IsaAVX10_1,             /* AVX10.1 Instructions */
    IsaAVX10_2,             /* AVX10.2 Instructions */
    IsaAVX10_2_MOVRS,       /* AVX10.2 MOVRS Instructions */
    IsaXAPIC,               /* XAPIC support */
    IsaX2APIC,              /* X2APIC support */
    IsaXAPIC_EXT,           /* XAPIC Extensions support (AMD) */
    IsaPCID,                /* PCID support */
    IsaINVPCID,             /* INVPCID instruction */
    IsaTSC_ADJUST,          /* TSC-Adjust MSR */
    IsaTSC_DEADLINE,        /* TSC-Deadline */
    IsaFOPCODE_DEPRECATION, /* FOPCODE Deprecation - FOPCODE update on unmasked x87 exception only */
    IsaFCS_FDS_DEPRECATION, /* FCS/FDS Deprecation */
    IsaFDP_DEPRECATION,     /* FDP Deprecation - FDP update on unmasked x87 exception only */
    IsaPKU,                 /* User-Mode Protection Keys */
    IsaPKS,                 /* Supervisor-Mode Protection Keys */
    IsaUMIP,                /* User-Mode Instructions Prevention */
    IsaRDPID,               /* RDPID Support */
    IsaTCE,                 /* Translation Cache Extensions (TCE) support (AMD) */
    IsaCLZERO,              /* CLZERO instruction support (AMD) */
    IsaSCA_MITIGATIONS,     /* Report SCA Mitigations in CPUID */
    IsaCET,                 /* Control Flow Enforcement */
    IsaWRMSRNS,             /* Non-Serializing version of WRMSR */
    IsaMSR_IMM,             /* Immediate forms of RDMSR and WRMSRNS */
    IsaCMPCCXADD,           /* CMPccXADD instructions */
    IsaSERIALIZE,           /* SERIALIZE instruction */
    IsaLASS,                /* Linear Address Space Separation support */
    IsaLA57,                /* 57-bit Virtual Address and 5-level paging support */
    IsaUINTR,               /* User Level Interrupts support */
    IsaFLEXIBLE_UIRET,      /* Flexible UIRET support */
    IsaMOVDIRI,             /* MOVDIRI instruction support */
    IsaMOVDIR64B,           /* MOVDIR64B instruction support */
    IsaMSRLIST,             /* RDMSRLIST/WRMSRLIST instructions support */
    IsaRAO_INT,             /* RAO-INT instructions support */
    IsaMOVRS,               /* MOVRS instructions support */
}
