//! The seam on every target that is not Windows.
//!
//! This exists so this crate's types are nameable everywhere. A machine
//! parameterised over a hypervisor engine has to type-check on a host that has
//! no hypervisor, and a caller has to be able to ask whether one is available
//! without a `cfg` of its own; both need the crate to compile, not to vanish.
//!
//! Every verb answers [`crate::WhpErrorKind::Unsupported`], which is exactly
//! the kind a caller is expected to handle by choosing another engine. The one
//! exception is [`hypervisor_present`], which answers `false` — the honest
//! result, and the reason the other verbs are never reached here.

use crate::error::{WhpError, WhpResult};
use crate::{
    CapabilityCode, CounterSet, GpaPerms, GvaTranslation, PropertyCode, RawPartition,
    RegisterValue,
};
use crate::vcpu::{Exit, InterruptRequest, Reg, SegmentRegister, TableRegister};

/// The name every refusal below carries, since on this target the platform
/// itself is what is missing rather than any particular call.
const CALL: &str = "WinHvPlatform";

pub fn hypervisor_present() -> WhpResult<bool> {
    Ok(false)
}

pub fn capability(_code: CapabilityCode) -> WhpResult<u64> {
    Err(WhpError::unsupported(CALL))
}

pub fn create_partition() -> WhpResult<RawPartition> {
    Err(WhpError::unsupported(CALL))
}

/// # Safety
/// As the Windows implementation states it. This body does nothing, but the
/// two implementations must present one signature, and an obligation that
/// disappears on the target where it happens to be vacuous is an obligation a
/// caller can be written against and then broken by a rebuild.
// UNSAFETY: a signature, not an operation — nothing here is unsafe on this
// target, and the `expect` is targeted at the one item rather than lifting the
// workspace `deny` for the file (R1).
#[expect(
    unsafe_code,
    reason = "the seam presents one signature on every target, obligations included"
)]
pub unsafe fn delete_partition(_partition: RawPartition) {
    // Unreachable: a `RawPartition` can only come from `create_partition`,
    // which never succeeds here.
}

pub fn set_property(
    _partition: RawPartition,
    _code: PropertyCode,
    _value: u64,
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn set_cpuid_exit_list(_partition: RawPartition, _leaves: &[u32]) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn setup(_partition: RawPartition) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

/// # Safety
/// As the Windows implementation states it, and declared here for the reason
/// [`delete_partition`] gives.
#[expect(
    unsafe_code,
    reason = "the seam presents one signature on every target, obligations included"
)]
pub unsafe fn map_gpa(
    _partition: RawPartition,
    _host: &mut [u8],
    _gpa: u64,
    _perms: GpaPerms,
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn unmap_gpa(_partition: RawPartition, _gpa: u64, _len: u64) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn dirty_bitmap(
    _partition: RawPartition,
    _gpa: u64,
    _len: u64,
    _out: &mut [u64],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn create_vp(_partition: RawPartition, _index: u32) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

/// # Safety
/// As the Windows implementation states it, and declared here for the reason
/// [`delete_partition`] gives.
#[expect(
    unsafe_code,
    reason = "the seam presents one signature on every target, obligations included"
)]
pub unsafe fn delete_vp(_partition: RawPartition, _index: u32) {
    // Unreachable, for the same reason as `delete_partition`.
}

pub fn run_vp(_partition: RawPartition, _index: u32) -> WhpResult<Exit> {
    Err(WhpError::unsupported(CALL))
}

pub fn cancel_vp(_partition: RawPartition, _index: u32) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn request_interrupt(
    _partition: RawPartition,
    _request: InterruptRequest,
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn get_counters(
    _partition: RawPartition,
    _index: u32,
    _set: CounterSet,
    _out: &mut [u64],
) -> WhpResult<usize> {
    Err(WhpError::unsupported(CALL))
}

pub fn get_xsave(
    _partition: RawPartition,
    _index: u32,
    _out: &mut [u8],
) -> WhpResult<usize> {
    Err(WhpError::unsupported(CALL))
}

pub fn set_xsave(_partition: RawPartition, _index: u32, _area: &[u8]) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn get_words(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _out: &mut [u64],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn get_registers(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _out: &mut [RegisterValue],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn set_registers(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _values: &[RegisterValue],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn set_words(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _words: &[u64],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn get_segments(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _out: &mut [SegmentRegister],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn set_segments(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _segments: &[SegmentRegister],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn get_tables(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _out: &mut [TableRegister],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn set_tables(
    _partition: RawPartition,
    _index: u32,
    _regs: &[Reg],
    _tables: &[TableRegister],
) -> WhpResult<()> {
    Err(WhpError::unsupported(CALL))
}

pub fn translate_gva(
    _partition: RawPartition,
    _index: u32,
    _gva: u64,
) -> WhpResult<GvaTranslation> {
    Err(WhpError::unsupported(CALL))
}
