//! The platform seam.
//!
//! Everything below this line is host FFI; everything above it is ordinary
//! Rust. The two implementations expose the identical function set, so no
//! caller ever carries a `cfg` — the selection happens once, here.

use rusty_box_core::GpaPerms;

use crate::error::{WhpError, WhpResult};
use crate::vcpu::{Exit, InterruptRequest, Reg, SegmentRegister, TableRegister};

#[cfg(windows)]
#[path = "sys/windows.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "sys/unsupported.rs"]
mod imp;

/// A live `WHV_PARTITION_HANDLE`.
///
/// The SDK's handle is a signed word whose two invalid spellings are `0` and
/// `-1`; the niche keeps one of those unrepresentable and the constructor
/// rejects the other, so a value of this type is a handle the platform vouched
/// for. Being a plain integer is also what lets `Send` and `Sync` derive
/// rather than be promised (R6).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub(crate) struct RawPartition(core::num::NonZeroIsize);

impl RawPartition {
    /// Accept a handle the platform just returned, or refuse it as a contract
    /// violation.
    pub(crate) fn new(handle: isize, call: &'static str) -> WhpResult<Self> {
        match core::num::NonZeroIsize::new(handle) {
            Some(nz) if nz.get() != -1 => Ok(Self(nz)),
            _ => Err(WhpError::contract(call)),
        }
    }

    pub(crate) const fn get(self) -> isize {
        self.0.get()
    }
}

/// The `WHV_CAPABILITY_CODE` values this port asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CapabilityCode {
    HypervisorPresent,
    Features,
    ExtendedVmExits,
    PhysicalAddressWidth,
}

/// The `WHV_PARTITION_PROPERTY_CODE` values this port sets, restricted to the
/// ones whose payload is a single word. `CpuidExitList` is a list and gets its
/// own verb.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PropertyCode {
    ProcessorCount,
    ExtendedVmExits,
    SeparateSecurityDomain,
    LocalApicEmulationMode,
}

/// How far a `WHvTranslateGva` got.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GvaTranslation {
    /// The `WHV_TRANSLATE_GVA_RESULT_CODE`, unmapped so an unexpected value
    /// still reaches a report rather than being lost to a match arm.
    pub result_code: i32,
    /// The guest-physical address, meaningful when `result_code` is zero.
    pub gpa: u64,
}

pub(crate) use imp::{
    cancel_vp, capability, create_partition, create_vp, delete_partition, delete_vp,
    dirty_bitmap, get_segments, get_tables, get_words, hypervisor_present, map_gpa,
    request_interrupt, run_vp, set_cpuid_exit_list, set_property, set_segments, set_tables,
    set_words, setup, translate_gva, unmap_gpa,
};

/// Every function `imp` must provide, stated once so the two implementations
/// cannot drift apart silently: a missing or mis-typed one fails this
/// coercion rather than only failing on the platform nobody built today.
const _IMP_IS_COMPLETE: ImpSignatures = ImpSignatures {
    hypervisor_present: imp::hypervisor_present,
    capability: imp::capability,
    create_partition: imp::create_partition,
    delete_partition: imp::delete_partition,
    set_property: imp::set_property,
    set_cpuid_exit_list: imp::set_cpuid_exit_list,
    setup: imp::setup,
    map_gpa: imp::map_gpa,
    unmap_gpa: imp::unmap_gpa,
    dirty_bitmap: imp::dirty_bitmap,
    create_vp: imp::create_vp,
    delete_vp: imp::delete_vp,
    run_vp: imp::run_vp,
    cancel_vp: imp::cancel_vp,
    request_interrupt: imp::request_interrupt,
    get_words: imp::get_words,
    set_words: imp::set_words,
    get_segments: imp::get_segments,
    set_segments: imp::set_segments,
    get_tables: imp::get_tables,
    set_tables: imp::set_tables,
    translate_gva: imp::translate_gva,
};

/// The shape `_IMP_IS_COMPLETE` pins. Its fields exist to be type-checked
/// against `imp`'s functions, never to be called through — naming each one at
/// its required signature is the whole point, so `dead_code` is answered here
/// by name rather than by switching the lint off for the file.
#[allow(
    dead_code,
    reason = "a field of this struct is a signature assertion, not a value anyone reads"
)]
struct ImpSignatures {
    hypervisor_present: fn() -> WhpResult<bool>,
    capability: fn(CapabilityCode) -> WhpResult<u64>,
    create_partition: fn() -> WhpResult<RawPartition>,
    delete_partition: fn(RawPartition),
    set_property: fn(RawPartition, PropertyCode, u64) -> WhpResult<()>,
    set_cpuid_exit_list: fn(RawPartition, &[u32]) -> WhpResult<()>,
    setup: fn(RawPartition) -> WhpResult<()>,
    map_gpa: fn(RawPartition, &mut [u8], u64, GpaPerms) -> WhpResult<()>,
    unmap_gpa: fn(RawPartition, u64, u64) -> WhpResult<()>,
    dirty_bitmap: fn(RawPartition, u64, u64, &mut [u64]) -> WhpResult<()>,
    create_vp: fn(RawPartition, u32) -> WhpResult<()>,
    delete_vp: fn(RawPartition, u32),
    run_vp: fn(RawPartition, u32) -> WhpResult<Exit>,
    cancel_vp: fn(RawPartition, u32) -> WhpResult<()>,
    request_interrupt: fn(RawPartition, InterruptRequest) -> WhpResult<()>,
    get_words: fn(RawPartition, u32, &[Reg], &mut [u64]) -> WhpResult<()>,
    set_words: fn(RawPartition, u32, &[Reg], &[u64]) -> WhpResult<()>,
    get_segments: fn(RawPartition, u32, &[Reg], &mut [SegmentRegister]) -> WhpResult<()>,
    set_segments: fn(RawPartition, u32, &[Reg], &[SegmentRegister]) -> WhpResult<()>,
    get_tables: fn(RawPartition, u32, &[Reg], &mut [TableRegister]) -> WhpResult<()>,
    set_tables: fn(RawPartition, u32, &[Reg], &[TableRegister]) -> WhpResult<()>,
    translate_gva: fn(RawPartition, u32, u64) -> WhpResult<GvaTranslation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_handles_the_platform_calls_invalid_are_both_refused() {
        assert!(RawPartition::new(0, "test").is_err());
        assert!(RawPartition::new(-1, "test").is_err());
        assert_eq!(RawPartition::new(7, "test").map(RawPartition::get), Ok(7));
    }

    /// A handle is the only thing a partition holds, so this is what makes
    /// `Partition` movable between threads without a promise (R6).
    #[test]
    fn a_partition_handle_is_send_and_sync_by_derivation() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RawPartition>();
    }
}
