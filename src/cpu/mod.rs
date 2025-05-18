use super::Result;

pub(super) mod apic;
pub(super) mod avx;
#[allow(clippy::module_inception)]
pub mod cpu;
pub(super) mod cpuid;
pub(super) mod cpustats;
pub(super) mod crregs;
pub mod decoder;
pub(super) mod descriptor;
pub(super) mod i387;
pub(super) mod icache;
pub(super) mod lazy_flags;
pub(super) mod mwait;
pub(super) mod paging;
pub(super) mod segment_ctrl_pro;
pub(super) mod softfloat3e;
pub(super) mod svm;
pub(super) mod tlb;
pub(super) mod vmx;
pub(super) mod xmm;
