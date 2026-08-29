//! Windows Hypervisor Platform backing for the rusty_box emulator.
//!
//! This is a host-FFI leaf: the one crate in the workspace that calls an
//! operating system directly, and therefore the one allowed to say `unsafe` —
//! confined to `sys/windows.rs`, which is where the workspace's `deny` on
//! `unsafe_code` is lifted and nowhere else.
//!
//! The crate compiles on every target. On anything but Windows every verb
//! answers [`WhpErrorKind::Unsupported`] and [`hypervisor_present`] answers
//! `false`, so a machine parameterised over an execution engine still
//! type-checks on a host that has no hypervisor, and a caller can ask whether
//! one exists without a `cfg` of its own.
//!
//! # Shape
//!
//! - [`hypervisor_present`] and [`capabilities`] ask the host what it can do.
//! - [`PartitionConfig`] is a partition before `WHvSetupPartition`, when
//!   properties may be set; [`PartitionConfig::setup`] turns it into a
//!   [`Partition`], which accepts memory and processors. The platform's two
//!   lifecycle states are two types, so the mistake cannot be spelled.
//! - [`HostPages`] is a page-aligned allocation a [`Partition`] takes
//!   ownership of when mapping, which is what ties host memory's lifetime to
//!   the mapping that reads it.
//! - [`Exit`] and [`ExitReason`] are this port's exhaustive reading of
//!   `WHV_RUN_VP_EXIT_CONTEXT`; the platform union is decoded once, behind the
//!   seam.
//!
//! # Provenance
//!
//! Every constant, bitfield position and structure layout is transcribed from
//! the Windows SDK's `WinHvPlatformDefs.h`, and each is attributed to the
//! union or enum it came from at the point of use. The bindings themselves are
//! Microsoft's `windows-sys`, linked with `raw-dylib`, so building needs no
//! SDK installed.

mod caps;
mod error;
mod partition;
mod sys;
mod vcpu;

pub use caps::{
    capabilities, hypervisor_present, Capabilities, ExtendedVmExits, Features, MsrExits,
};
pub use error::{WhpError, WhpErrorKind, WhpResult};
pub use partition::{
    Canceller, HostPages, InterceptCounter, InterceptCounters, InterruptRequester, LateProperty,
    LocalApicMode, Partition, PartitionConfig, RuntimeCounters, PAGE_SIZE,
};
pub use sys::GvaTranslation;

/// Re-exported so a caller mapping memory into a partition need not also name
/// the core crate. It is core's type, not this crate's: a permission on a
/// guest-physical window means the same thing to every engine, and the WHP
/// flags are one encoding of it rather than its definition.
pub use rusty_box_core::GpaPerms;
pub use vcpu::{
    AccessType, CpuidAccess, DestinationMode, Exit, ExitReason, InternalActivity, InterruptKind,
    InterruptRequest, InterruptionType, IoPortAccess, MemoryAccess, MsrAccess,
    PendingInterruption, Reg, SegmentRegister, TableRegister, TriggerMode, VpContext, ALL_REGS,
};

/// A partition handle is a plain integer and the pages behind a mapping are
/// owned, so a partition moves between threads by derivation rather than by
/// promise (R6). The platform's own contract is that its calls are safe from
/// any thread; what Rust adds is that only one place at a time holds the
/// `&mut` needed to run a processor.
const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send::<Partition>();
    assert_send::<PartitionConfig>();
    assert_send::<HostPages>();
    // The two values meant to cross threads while a processor runs.
    assert_send_sync::<Canceller>();
    assert_send_sync::<InterruptRequester>();
};
