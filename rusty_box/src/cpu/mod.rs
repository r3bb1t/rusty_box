// Always available (no alloc needed)
pub mod instrumentation;
pub mod decoder;

pub use instrumentation::{
    BranchEvent, BranchType, CacheCntrl, CodeSize, CpuSetupMode, CpuSnapshot, EmuStopReason,
    ExitSet, HookMask, HwInterruptEvent, Instrumentation,
    InvEptType, InvPcidType, IoHookEvent, MemAccessRW, MemHookEvent,
    HookCtx, InstrAction, MemPerms, MemType, MwaitFlags, PrefetchHint, ResetType,
    LinAccess, MemPermViolation, MemUnmapped, MwaitEvent, OpcodeEvent, PhyAccess,
    PrefetchEvent, TlbCntrl, X86Reg,
};
#[cfg(feature = "instrumentation")]
pub use instrumentation::{HookHandle, InstrumentationError, IoHookType, MemHookType};

/// Reason for CPU reset (always available, no alloc needed).
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ResetReason {
    Software = 10,
    Hardware = 11,
}

// Core CPU emulation modules (no alloc needed)
pub mod error;
pub use error::{CpuError, Result};

pub(crate) mod api_bridge;

pub(super) mod access;
pub(super) mod aes;
pub(super) mod apic;
pub(super) mod arith16;
pub(super) mod arith32;
pub(super) mod arith64;
pub(super) mod arith8;
pub(super) mod avx;
pub(super) mod avx512;
pub(super) mod avx512_bcast;
pub(super) mod avx512_bw;
pub(super) mod avx512_cmp;
pub(super) mod avx512_cvt;
pub(super) mod avx512_fma;
pub(super) mod avx512_gather;
pub(super) mod avx512_insert;
pub(super) mod avx512_int;
pub(super) mod avx512_mask;
pub(super) mod avx512_misc;
pub(super) mod avx512_perm;
pub(super) mod avx512_round;
pub(super) mod avx512_scalar;
pub(super) mod bcd;
pub(super) mod bit;
pub(super) mod bit16;
pub(super) mod bit32;
pub(super) mod bit64;
pub(super) mod bmi32;
pub(super) mod bmi64;
pub mod builder;
#[allow(clippy::module_inception)]
pub mod cpu;
mod cpu_getters_and_setters;
pub(super) mod cpu_macros;
pub(super) mod crc32;
pub(super) mod cet;
pub(super) mod cpudb;
pub(super) mod cpuid;
pub(super) mod cpustats;
pub(super) mod crregs;
pub(super) mod ctrl_xfer16;
pub(super) mod ctrl_xfer32;
pub(super) mod ctrl_xfer64;
pub(super) mod data_xfer16;
pub(super) mod data_xfer32;
pub(super) mod data_xfer64;
pub(super) mod data_xfer8;
pub(super) mod data_xfer_ext;
pub(super) mod descriptor;
pub(super) mod dispatcher;
pub mod eflags;
pub(super) mod event;
pub(super) mod exception;
pub(super) mod flag_ctrl;
pub(super) mod flag_ctrl_pro;
pub(super) mod fred;
pub(super) mod fpu;
pub(super) mod gf2;
pub(super) mod i387;
pub(super) mod icache;
pub(super) mod init;
pub(super) mod io;
pub(super) mod lazy_flags;
pub(super) mod logical16;
pub(super) mod logical32;
pub(super) mod logical64;
pub(super) mod logical8;
pub(super) mod mmx;
pub(super) mod msr;
pub(super) mod mult16;
pub(super) mod mult32;
pub(super) mod mult64;
pub(super) mod mult8;
pub(super) mod mwait;
pub(super) mod opcodes_table;
pub(super) mod paging;
pub(super) mod proc_ctrl;
pub(super) mod protect_ctrl;
pub(super) mod protected_interrupts;
pub(super) mod rdrand;
pub(super) mod rusty_box;
pub(super) mod segment_ctrl_pro;
pub(super) mod sha;
#[cfg(feature = "std")]
pub mod snapshot;
pub(super) mod shift16;
pub(super) mod shift32;
pub(super) mod shift64;
pub(super) mod shift8;
pub(super) mod smm;
pub(super) mod soft_int;
pub(super) mod softfloat3e;
pub(super) mod sse;
pub(super) mod sse_move;
pub(super) mod sse_pfp;
pub(super) mod sse_rcp;
pub(super) mod sse_string;
pub(super) mod stack;
pub(super) mod stack16;
pub(super) mod stack32;
pub(super) mod stack64;
pub(super) mod string;
pub(super) mod svm;
pub(super) mod tasking;
pub(super) mod tlb;
pub(super) mod uintr;
pub(super) mod vm8086;
pub(super) mod vmx;
pub(super) mod xmm;

pub use cpu::BxCpuC;
pub use cpuid::BxCpuIdTrait;

pub use cpudb::amd::amd_ryzen::AmdRyzen;
pub use cpudb::intel::*;