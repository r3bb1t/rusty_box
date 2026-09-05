//! Windows Hypervisor Platform backing for the rusty_box emulator.
//!
//! This is the safe wrapper over the platform. The host FFI lives one crate
//! down, in `rusty_box_whp_sys`, which is where the workspace's `deny` on
//! `unsafe_code` is lifted; this crate performs no host call of its own. Its
//! four `unsafe` tokens are all about ownership of host memory and handles:
//! [`Partition::map_borrowed`] is a signature rather than a block, stating a
//! contract no lifetime can express, and the other three are where this crate
//! discharges what the seam asks of a caller — the two destructors that delete
//! a partition and its processors exactly once, and the single private door
//! onto the platform's one retaining verb (R1, R5).
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
//! The seam's own vocabulary — the error type, the register names, the exit
//! shapes — is re-exported here under the names a caller of this crate uses, so
//! the split between wrapper and seam is not something a caller has to know.
//!
//! # Provenance
//!
//! Every constant, bitfield position and structure layout is transcribed from
//! the Windows SDK's `WinHvPlatformDefs.h`, and each is attributed to the
//! union or enum it came from at the point of use. The bindings themselves are
//! Microsoft's `windows-sys`, linked with `raw-dylib`, so building needs no
//! SDK installed.

mod caps;
mod partition;

// The platform seam, under the one name this crate reaches it by. Every host
// call `caps` and `partition` make goes through here.
use rusty_box_whp_sys as sys;

pub use caps::{
    capabilities, hypervisor_present, Capabilities, ExtendedVmExits, Features, MsrExits,
    SyntheticFeatures,
};
pub use partition::{
    Canceller, HostPages, InterceptCounter, InterceptCounters, InterruptRequester, LateProperty,
    LocalApicMode, Partition, PartitionConfig, RuntimeCounters, PAGE_SIZE,
};
// The seam's three vocabularies — register shapes, errors, and the exits a
// processor produces — under the names a caller of this crate uses them by.
pub use rusty_box_whp_sys::{shape_of, FeatureBanks, GvaTranslation, RegisterValue};
pub use rusty_box_whp_sys::{WhpError, WhpErrorKind, WhpResult};
pub use rusty_box_whp_sys::{
    AccessType, ApicRegister, ApicStatePage, ApicVector, ApicWriteType, CpuidAccess,
    DestinationMode, Exit, ExitReason, InternalActivity, InterruptKind, InterruptRequest,
    InterruptionType, IoPortAccess, MemoryAccess, MsrAccess, PendingExtIntEvent,
    PendingInterruption, Reg, SegmentRegister, TableRegister, TriggerMode, VpContext, ALL_REGS,
    UNEXCHANGED_REGS,
};

/// Re-exported so a caller mapping memory into a partition need not also name
/// the core crate. It is core's type, not this crate's: a permission on a
/// guest-physical window means the same thing to every engine, and the WHP
/// flags are one encoding of it rather than its definition.
pub use rusty_box_core::GpaPerms;

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
