pub mod error;
pub use error::{CpuError, Result};

pub(super) mod access;
pub(super) mod apic;
pub(super) mod avx;
pub mod builder;
#[allow(clippy::module_inception)]
pub mod cpu;
mod cpu_getters_and_setters;
pub(super) mod cpu_macros;
pub(super) mod cpudb;
pub(super) mod cpuid;
pub(super) mod cpustats;
pub(super) mod crregs;
pub mod decoder;
pub(super) mod descriptor;
pub(super) mod event;
pub(super) mod exception;
pub(super) mod i387;
pub(super) mod icache;
pub(super) mod init;
pub(super) mod lazy_flags;
pub(super) mod msr;
pub(super) mod mwait;
pub(super) mod paging;
pub(super) mod rusty_box;
pub(super) mod segment_ctrl_pro;
pub(super) mod smm;
pub(super) mod softfloat3e;
pub(super) mod svm;
pub(super) mod tlb;
pub(super) mod vmcs;
pub(super) mod vmx;
pub(super) mod vmx_ctrls;
pub(super) mod xmm;

pub use cpu::BxCpuC;
pub use cpuid::BxCpuIdTrait;

pub use cpudb::intel::*;
