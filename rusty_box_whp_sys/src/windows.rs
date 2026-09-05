//! The Windows Hypervisor Platform calls.
//!
//! This is the whole host-FFI surface of the workspace's Windows leaf. Every
//! `unsafe` block below names the invariant it rests on and who guarantees it
//! (R1); nothing outside this file may say `unsafe` at all, which the
//! workspace lint table enforces with a `deny`.
//!
//! The one non-obvious platform rule, and the reason [`RegVal`] exists: the SDK
//! declares `WHV_REGISTER_VALUE` with `DECLSPEC_ALIGN(16)` because the union's
//! widest member is an XMM register. The generated Rust union carries the
//! alignment of its widest *field*, which is 8, so an array of them may start
//! at an address the hypervisor will not accept. Every register buffer that
//! crosses the boundary is therefore an array of `RegVal`, never of the raw
//! union — a rule Microsoft's own OpenVMM WHP wrapper arrived at independently.

// UNSAFETY: calling the WinHvPlatform C API. An unfulfilled expectation warns,
// so the day this file no longer needs the exemption the lint says so.
#![expect(
    unsafe_code,
    reason = "the WinHvPlatform C API is this crate's whole reason to exist"
)]

use windows_sys::Win32::System::Hypervisor::*;

use crate::error::{WhpError, WhpResult};
use crate::{
    shape_of, CapabilityCode, CounterSet, FeatureBanks, GpaPerms, GvaTranslation, PropertyCode,
    RawPartition, RegisterValue, VpStateType,
};
use crate::vcpu::{
    AccessType, ApicWriteType, CpuidAccess, Exit, ExitReason, InterruptRequest, IoPortAccess,
    MemoryAccess, MsrAccess, Reg, SegmentRegister, TableRegister, VpContext,
};

/// `WHvCapabilityCodePhysicalAddressWidth`, which the SDK header defines but
/// the generated bindings of this `windows-sys` release do not yet carry.
const CAPABILITY_PHYSICAL_ADDRESS_WIDTH: WHV_CAPABILITY_CODE = 0x0000_100A;

/// A `WHV_REGISTER_VALUE` at the alignment the SDK asks for. See the module
/// comment; never let the bare union reach a buffer the platform reads.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct RegVal(WHV_REGISTER_VALUE);

const _: () = assert!(
    core::mem::size_of::<WHV_REGISTER_VALUE>() == 16,
    "a register value that is not 16 bytes would give a RegVal array the wrong stride"
);
const _: () = assert!(core::mem::align_of::<RegVal>() == 16);
const _: () = assert!(core::mem::size_of::<RegVal>() == 16);

impl RegVal {
    /// The union's widest member is 16 bytes, so writing the 128-bit one as
    /// zero clears every byte — which is the SDK's own initialisation idiom,
    /// reached here without `mem::zeroed` so the constructor stays `const`.
    const fn zeroed() -> Self {
        Self(WHV_REGISTER_VALUE { Reg128: WHV_UINT128 { Dword: [0; 4] } })
    }

    const fn word(value: u64) -> Self {
        Self(WHV_REGISTER_VALUE { Reg64: value })
    }

    const fn segment(seg: SegmentRegister) -> Self {
        Self(WHV_REGISTER_VALUE {
            Segment: WHV_X64_SEGMENT_REGISTER {
                Base: seg.base,
                Limit: seg.limit,
                Selector: seg.selector,
                Anonymous: WHV_X64_SEGMENT_REGISTER_0 { Attributes: seg.attributes },
            },
        })
    }

    const fn table(table: TableRegister) -> Self {
        Self(WHV_REGISTER_VALUE {
            Table: WHV_X64_TABLE_REGISTER {
                Pad: [0; 3],
                Limit: table.limit,
                Base: table.base,
            },
        })
    }

    fn as_word(self) -> u64 {
        // SAFETY: every member of the union is at least 8 bytes wide and this
        // reads the 8 the caller asked a word-shaped register for.
        unsafe { self.0.Reg64 }
    }

    fn as_segment(self) -> SegmentRegister {
        // SAFETY: read back from a register the caller named as segment-shaped,
        // so the platform wrote this member of the union.
        let seg = unsafe { self.0.Segment };
        SegmentRegister {
            base: seg.Base,
            limit: seg.Limit,
            selector: seg.Selector,
            // SAFETY: the attribute union's two members are the bitfield and
            // this `u16`, both two bytes of plain data.
            attributes: unsafe { seg.Anonymous.Attributes },
        }
    }

    fn as_table(self) -> TableRegister {
        // SAFETY: as `as_segment`, for a register named as table-shaped.
        let table = unsafe { self.0.Table };
        TableRegister {
            base: table.Base,
            limit: table.Limit,
        }
    }

    const fn words128(words: [u64; 2]) -> Self {
        Self(WHV_REGISTER_VALUE {
            Reg128: WHV_UINT128 {
                Anonymous: WHV_UINT128_0 { Low64: words[0], High64: words[1] },
            },
        })
    }

    fn as_words128(self) -> [u64; 2] {
        // SAFETY: `Reg128` is the union's widest member, so every byte of the
        // value is part of it whatever the platform wrote; its own two members
        // are this pair of words and the same sixteen bytes as four `u32`s.
        let wide = unsafe { self.0.Reg128.Anonymous };
        [wide.Low64, wide.High64]
    }
}

/// `WHV_E_UNSUPPORTED_HYPERVISOR_CONFIG` from the SDK's `winerror.h`. The
/// platform uses it for "this host's hypervisor cannot do this at all", which
/// is a different answer from "this call was wrong" and deserves the kind a
/// caller handles by choosing another engine — the same kind a non-Windows
/// build reports for everything.
const WHV_E_UNSUPPORTED_HYPERVISOR_CONFIG: windows_sys::core::HRESULT = 0x8037_0303_u32 as i32;

/// Turn an `HRESULT` into this crate's error. WHP reports success as `S_OK`
/// and every failure as a negative code.
fn check(hresult: windows_sys::core::HRESULT, call: &'static str) -> WhpResult<()> {
    match hresult {
        0.. => Ok(()),
        WHV_E_UNSUPPORTED_HYPERVISOR_CONFIG => Err(WhpError::unsupported(call)),
        failed => Err(WhpError::platform(call, failed)),
    }
}

/// The platform value and payload width for each capability this port asks
/// for. Exhaustive, so a capability added to the enum must be sized here (R5).
const fn capability_code(code: CapabilityCode) -> (WHV_CAPABILITY_CODE, u32) {
    match code {
        CapabilityCode::HypervisorPresent => (WHvCapabilityCodeHypervisorPresent, 4),
        CapabilityCode::Features => (WHvCapabilityCodeFeatures, 8),
        CapabilityCode::ExtendedVmExits => (WHvCapabilityCodeExtendedVmExits, 8),
        CapabilityCode::PhysicalAddressWidth => (CAPABILITY_PHYSICAL_ADDRESS_WIDTH, 4),
        CapabilityCode::ProcessorFeatures => (WHvCapabilityCodeProcessorFeatures, 8),
        // `WHV_CAPABILITY.ProcessorClockFrequency` and `InterruptClockFrequency`
        // are `UINT64` hertz counts.
        CapabilityCode::ProcessorClockFrequency => (WHvCapabilityCodeProcessorClockFrequency, 8),
        CapabilityCode::InterruptClockFrequency => (WHvCapabilityCodeInterruptClockFrequency, 8),
        // The two banked answers, sized as the whole structure: a count word, a
        // reserved word, then `FeatureBanks::MAX` banks. `capability` refuses
        // them on the strength of `is_banked` before this table is reached, so
        // the size here is the one `capability_banks` asks for.
        CapabilityCode::ProcessorFeaturesBanks => (WHvCapabilityCodeProcessorFeaturesBanks, 24),
        CapabilityCode::SyntheticProcessorFeaturesBanks => {
            (WHvCapabilityCodeSyntheticProcessorFeaturesBanks, 16)
        }
    }
}

/// Likewise for partition properties. Only the single-word ones live here;
/// `CpuidExitList` is a list and has its own verb.
const fn property_code(code: PropertyCode) -> (WHV_PARTITION_PROPERTY_CODE, u32) {
    match code {
        PropertyCode::ProcessorCount => (WHvPartitionPropertyCodeProcessorCount, 4),
        PropertyCode::ExtendedVmExits => (WHvPartitionPropertyCodeExtendedVmExits, 8),
        PropertyCode::MsrExitBitmap => (WHvPartitionPropertyCodeX64MsrExitBitmap, 8),
        PropertyCode::ExceptionExitBitmap => (WHvPartitionPropertyCodeExceptionExitBitmap, 8),
        PropertyCode::SeparateSecurityDomain => {
            (WHvPartitionPropertyCodeSeparateSecurityDomain, 4)
        }
        PropertyCode::LocalApicEmulationMode => {
            (WHvPartitionPropertyCodeLocalApicEmulationMode, 4)
        }
        PropertyCode::ProcessorFeatures => (WHvPartitionPropertyCodeProcessorFeatures, 8),
        // `WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS`: a count word, a reserved
        // word and the single bank the SDK declares
        // (`WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS_COUNT` is 1).
        PropertyCode::SyntheticProcessorFeaturesBanks => {
            (WHvPartitionPropertyCodeSyntheticProcessorFeaturesBanks, 16)
        }
        PropertyCode::ReferenceTime => (WHvPartitionPropertyCodeReferenceTime, 8),
    }
}

/// The platform value for each per-processor state blob this port moves.
/// Exhaustive (R5), and paired with the buffer size the platform demands for
/// it: the two are decided together because a state type read into a buffer
/// sized for another is a refusal at best.
const fn vp_state_code(state: VpStateType) -> (WHV_VIRTUAL_PROCESSOR_STATE_TYPE, usize) {
    match state {
        VpStateType::InterruptControllerState2 => (
            WHvVirtualProcessorStateTypeInterruptControllerState2,
            crate::ApicStatePage::BYTES,
        ),
    }
}

/// The platform name for each register this port touches. Exhaustive (R5).
const fn register_name(reg: Reg) -> WHV_REGISTER_NAME {
    match reg {
        Reg::Rax => WHvX64RegisterRax,
        Reg::Rbx => WHvX64RegisterRbx,
        Reg::Rcx => WHvX64RegisterRcx,
        Reg::Rdx => WHvX64RegisterRdx,
        Reg::Rsi => WHvX64RegisterRsi,
        Reg::Rdi => WHvX64RegisterRdi,
        Reg::Rsp => WHvX64RegisterRsp,
        Reg::Rbp => WHvX64RegisterRbp,
        Reg::R8 => WHvX64RegisterR8,
        Reg::R9 => WHvX64RegisterR9,
        Reg::R10 => WHvX64RegisterR10,
        Reg::R11 => WHvX64RegisterR11,
        Reg::R12 => WHvX64RegisterR12,
        Reg::R13 => WHvX64RegisterR13,
        Reg::R14 => WHvX64RegisterR14,
        Reg::R15 => WHvX64RegisterR15,
        Reg::Rip => WHvX64RegisterRip,
        Reg::Rflags => WHvX64RegisterRflags,
        Reg::Cs => WHvX64RegisterCs,
        Reg::Ds => WHvX64RegisterDs,
        Reg::Es => WHvX64RegisterEs,
        Reg::Ss => WHvX64RegisterSs,
        Reg::Fs => WHvX64RegisterFs,
        Reg::Gs => WHvX64RegisterGs,
        Reg::Ldtr => WHvX64RegisterLdtr,
        Reg::Tr => WHvX64RegisterTr,
        Reg::Gdtr => WHvX64RegisterGdtr,
        Reg::Idtr => WHvX64RegisterIdtr,
        Reg::Cr0 => WHvX64RegisterCr0,
        Reg::Cr2 => WHvX64RegisterCr2,
        Reg::Cr3 => WHvX64RegisterCr3,
        Reg::Cr4 => WHvX64RegisterCr4,
        Reg::Cr8 => WHvX64RegisterCr8,
        Reg::Dr0 => WHvX64RegisterDr0,
        Reg::Dr1 => WHvX64RegisterDr1,
        Reg::Dr2 => WHvX64RegisterDr2,
        Reg::Dr3 => WHvX64RegisterDr3,
        Reg::Dr6 => WHvX64RegisterDr6,
        Reg::Dr7 => WHvX64RegisterDr7,
        Reg::Efer => WHvX64RegisterEfer,
        Reg::KernelGsBase => WHvX64RegisterKernelGsBase,
        Reg::Star => WHvX64RegisterStar,
        Reg::Lstar => WHvX64RegisterLstar,
        Reg::Cstar => WHvX64RegisterCstar,
        Reg::Sfmask => WHvX64RegisterSfmask,
        Reg::SysenterCs => WHvX64RegisterSysenterCs,
        Reg::SysenterEsp => WHvX64RegisterSysenterEsp,
        Reg::SysenterEip => WHvX64RegisterSysenterEip,
        Reg::Pat => WHvX64RegisterPat,
        Reg::ApicBase => WHvX64RegisterApicBase,
        Reg::Tsc => WHvX64RegisterTsc,
        Reg::Xcr0 => WHvX64RegisterXCr0,
        Reg::PendingInterruption => WHvRegisterPendingInterruption,
        Reg::InterruptState => WHvRegisterInterruptState,
        Reg::InternalActivityState => WHvRegisterInternalActivityState,
        Reg::DeliverabilityNotifications => WHvX64RegisterDeliverabilityNotifications,
        Reg::PendingEvent => WHvRegisterPendingEvent,
        Reg::ApicTpr => WHvX64RegisterApicTpr,
    }
}

const fn map_flags(perms: GpaPerms) -> WHV_MAP_GPA_RANGE_FLAGS {
    let mut flags = WHvMapGpaRangeFlagNone;
    if perms.read {
        flags |= WHvMapGpaRangeFlagRead;
    }
    if perms.write {
        flags |= WHvMapGpaRangeFlagWrite;
    }
    if perms.execute {
        flags |= WHvMapGpaRangeFlagExecute;
    }
    if perms.track_dirty {
        flags |= WHvMapGpaRangeFlagTrackDirtyPages;
    }
    flags
}

pub fn hypervisor_present() -> WhpResult<bool> {
    // A machine without the platform feature installed has no
    // `winhvplatform.dll` to load at all, and raw-dylib linkage turns that
    // into a load failure rather than a return code. The capability query is
    // the first call this crate ever makes, so translating its refusal into
    // "not supported" is what keeps that from looking like a platform bug.
    use crate::WhpErrorKind::{Platform, Unsupported};
    match capability(CapabilityCode::HypervisorPresent) {
        Ok(present) => Ok(present != 0),
        // A refusal of the very first call this crate makes says the platform
        // is not usable here, which is what the caller asked. A short or
        // malformed answer is something else and still propagates.
        Err(err) if matches!(err.kind(), Platform | Unsupported) => Ok(false),
        Err(err) => Err(err),
    }
}

pub fn capability(code: CapabilityCode) -> WhpResult<u64> {
    const CALL: &str = "WHvGetCapability";
    if code.is_banked() {
        // A banked answer does not fit a word, and truncating it here would
        // report a count as a feature set. `capability_banks` is the verb.
        return Err(WhpError::contract(CALL));
    }
    let (raw, len) = capability_code(code);
    let mut buffer = 0u64;
    let mut written = 0u32;
    // SAFETY: the buffer is a live `u64` and `len` is never more than its 8
    // bytes, which `capability_code` guarantees by construction; `written` is
    // a live `u32`. A short write is caught below rather than trusted.
    check(
        unsafe {
            WHvGetCapability(raw, core::ptr::from_mut(&mut buffer).cast(), len, &mut written)
        },
        CALL,
    )?;
    if written < len {
        return Err(WhpError::contract(CALL));
    }
    // The platform writes `len` little-endian bytes, so a 4-byte answer
    // leaves the high half of the word zero and needs no masking.
    Ok(buffer)
}

/// Read a capability whose answer is a `WHV_*_FEATURES_BANKS` structure.
///
/// Separate from [`capability`] because the payload is a count and a run of
/// banks rather than a value: the count says how many banks the host actually
/// wrote, and a caller reading past it would be reading bytes nobody set.
pub fn capability_banks(code: CapabilityCode) -> WhpResult<FeatureBanks> {
    const CALL: &str = "WHvGetCapability(FeaturesBanks)";
    if !code.is_banked() {
        return Err(WhpError::contract(CALL));
    }
    let (raw, len) = capability_code(code);
    // The structure is a `u32` count, a `u32` reserved, then the banks — one
    // `u64`-aligned run, which is what makes this array both correctly aligned
    // for the platform and readable back as words.
    let mut buffer = [0u64; 1 + FeatureBanks::MAX];
    let mut written = 0u32;
    if len as usize > core::mem::size_of_val(&buffer) {
        return Err(WhpError::contract(CALL));
    }
    // SAFETY: the platform writes at most `len` bytes, and the guard above
    // establishes that `len` fits the buffer this borrow keeps alive for the
    // call; `written` is a live, correctly typed out-parameter.
    check(
        unsafe {
            WHvGetCapability(raw, buffer.as_mut_ptr().cast(), len, &mut written)
        },
        CALL,
    )?;
    if written < len {
        return Err(WhpError::contract(CALL));
    }
    // The count is the low half of the first word and the reserved half is the
    // high one, both little-endian, so the banks start at word 1.
    let count = buffer[0] as u32;
    if count as usize > FeatureBanks::MAX {
        // A host reporting more banks than the SDK declares has an ABI this
        // port cannot read; saying so beats silently keeping the first two.
        return Err(WhpError::contract(CALL));
    }
    Ok(FeatureBanks { count, banks: [buffer[1], buffer[2]] })
}

/// Read a partition property that fits in a word.
///
/// The counterpart of [`set_property`], and the only way to reach a property
/// the platform makes read-only — `ReferenceTime` is set by the partition's own
/// clock, never by a caller.
pub fn get_property_word(partition: RawPartition, code: PropertyCode) -> WhpResult<u64> {
    const CALL: &str = "WHvGetPartitionProperty";
    let (raw, len) = property_code(code);
    let mut buffer = 0u64;
    let mut written = 0u32;
    if len as usize > core::mem::size_of_val(&buffer) {
        // A property wider than a word has no word to be read into; the caller
        // wants a shaped verb, as `SyntheticProcessorFeaturesBanks` does.
        return Err(WhpError::contract(CALL));
    }
    // SAFETY: the buffer is a live `u64` and the guard above establishes that
    // `len` is at most its 8 bytes; `written` is a live `u32`. A short write is
    // caught below rather than trusted.
    check(
        unsafe {
            WHvGetPartitionProperty(
                partition.get(),
                raw,
                core::ptr::from_mut(&mut buffer).cast(),
                len,
                &mut written,
            )
        },
        CALL,
    )?;
    if written < len {
        return Err(WhpError::contract(CALL));
    }
    // The platform writes `len` little-endian bytes, so a 4-byte answer leaves
    // the high half of the word zero and needs no masking.
    Ok(buffer)
}

/// Set a partition property whose payload is a structure rather than a word.
///
/// The buffer must be exactly the width the property's own table declares: the
/// platform reads a fixed structure, and a shorter buffer would have it read
/// past what the caller owns while a longer one hides a caller that built the
/// wrong shape.
pub fn set_property_bytes(
    partition: RawPartition,
    code: PropertyCode,
    payload: &[u8],
) -> WhpResult<()> {
    const CALL: &str = "WHvSetPartitionProperty(bytes)";
    let (raw, len) = property_code(code);
    if payload.len() != len as usize {
        return Err(WhpError::contract(CALL));
    }
    // SAFETY: the platform reads exactly `len` bytes, and the guard above
    // establishes that `len` is exactly what the slice owns; the slice outlives
    // the call because it is borrowed for it.
    check(
        unsafe {
            WHvSetPartitionProperty(partition.get(), raw, payload.as_ptr().cast(), len)
        },
        CALL,
    )
}

/// Stop the partition's reference clock, and with it every timer the guest
/// reads through the platform.
pub fn suspend_time(partition: RawPartition) -> WhpResult<()> {
    // SAFETY: no pointer crosses; the partition handle is live.
    check(
        unsafe { WHvSuspendPartitionTime(partition.get()) },
        "WHvSuspendPartitionTime",
    )
}

/// Start it again, from where [`suspend_time`] stopped it.
pub fn resume_time(partition: RawPartition) -> WhpResult<()> {
    // SAFETY: no pointer crosses; the partition handle is live.
    check(
        unsafe { WHvResumePartitionTime(partition.get()) },
        "WHvResumePartitionTime",
    )
}

/// Read one per-processor state blob, answering how many bytes the platform
/// wrote.
///
/// The buffer must be exactly the size the state type declares — the platform
/// refuses a shorter one, and a longer one would hide a caller that sized it
/// from the wrong structure.
pub fn get_vp_state(
    partition: RawPartition,
    index: u32,
    state: VpStateType,
    out: &mut [u8],
) -> WhpResult<usize> {
    const CALL: &str = "WHvGetVirtualProcessorState";
    let (raw, bytes) = vp_state_code(state);
    if out.len() != bytes {
        return Err(WhpError::contract(CALL));
    }
    let Ok(len) = u32::try_from(bytes) else {
        return Err(WhpError::contract(CALL));
    };
    let mut written = 0u32;
    // SAFETY: the platform writes at most `len` bytes, and the guard above
    // establishes that `len` is exactly what the slice owns; `written` is a
    // live, correctly typed out-parameter. The buffer outlives the call because
    // it is borrowed for it.
    check(
        unsafe {
            WHvGetVirtualProcessorState(
                partition.get(),
                index,
                raw,
                out.as_mut_ptr().cast(),
                len,
                &mut written,
            )
        },
        CALL,
    )?;
    usize::try_from(written).map_err(|_| WhpError::contract(CALL))
}

/// Write one per-processor state blob — the counterpart of [`get_vp_state`],
/// taking the same buffer back at the same size.
pub fn set_vp_state(
    partition: RawPartition,
    index: u32,
    state: VpStateType,
    blob: &[u8],
) -> WhpResult<()> {
    const CALL: &str = "WHvSetVirtualProcessorState";
    let (raw, bytes) = vp_state_code(state);
    if blob.len() != bytes {
        return Err(WhpError::contract(CALL));
    }
    let Ok(len) = u32::try_from(bytes) else {
        return Err(WhpError::contract(CALL));
    };
    // SAFETY: the platform reads exactly `len` bytes, and the guard above
    // establishes that `len` is exactly what the slice owns; the slice outlives
    // the call because it is borrowed for it.
    check(
        unsafe {
            WHvSetVirtualProcessorState(
                partition.get(),
                index,
                raw,
                blob.as_ptr().cast(),
                len,
            )
        },
        CALL,
    )
}

pub fn create_partition() -> WhpResult<RawPartition> {
    const CALL: &str = "WHvCreatePartition";
    let mut handle: WHV_PARTITION_HANDLE = 0;
    // SAFETY: `handle` is a live, correctly typed out-parameter.
    check(unsafe { WHvCreatePartition(&mut handle) }, CALL)?;
    RawPartition::new(handle, CALL)
}

/// Release a partition and every host resource behind it.
///
/// # Safety
/// The caller owns the uniqueness this call cannot check. `partition` must be
/// a handle [`create_partition`] returned, must not have been deleted already,
/// and this must be its last use — no verb below may name it afterwards.
/// [`RawPartition`] is `Copy`, so the type system tracks none of that; a
/// second delete, or any later call on the same word, hands the platform a
/// stale handle.
pub unsafe fn delete_partition(partition: RawPartition) {
    // The result is deliberately not propagated: a deletion is the last thing
    // that happens to a handle, so there is nobody left to report to, and the
    // only documented failure is a handle that was already invalid. It is
    // logged rather than discarded so a leaked partition is still visible.
    let hresult = WHvDeletePartition(partition.get());
    if hresult < 0 {
        tracing::warn!(
            hresult = format_args!("{:#010x}", hresult as u32),
            "WHvDeletePartition refused; the partition's host resources are leaked"
        );
    }
}

/// Set a partition property that fits in a word.
///
/// The counterpart of [`get_property_word`], and the symmetric guard: a
/// property whose table entry is wider than a word has no word to be written
/// from, and the caller wants a shaped verb — [`set_property_bytes`], as
/// `SyntheticProcessorFeaturesBanks` does.
pub fn set_property(
    partition: RawPartition,
    code: PropertyCode,
    value: u64,
) -> WhpResult<()> {
    const CALL: &str = "WHvSetPartitionProperty";
    let (raw, len) = property_code(code);
    if len as usize > core::mem::size_of_val(&value) {
        return Err(WhpError::contract(CALL));
    }
    // SAFETY: the platform reads `len` bytes, and the guard above establishes
    // that `len` is at most the 8 bytes this local holds; the local outlives the
    // call because it is borrowed for it.
    check(
        unsafe {
            WHvSetPartitionProperty(
                partition.get(),
                raw,
                core::ptr::from_ref(&value).cast(),
                len,
            )
        },
        CALL,
    )
}

pub fn set_cpuid_exit_list(partition: RawPartition, leaves: &[u32]) -> WhpResult<()> {
    const CALL: &str = "WHvSetPartitionProperty(CpuidExitList)";
    let bytes = core::mem::size_of_val(leaves);
    let Ok(len) = u32::try_from(bytes) else {
        return Err(WhpError::contract(CALL));
    };
    // SAFETY: the platform reads exactly the `len` bytes the slice owns.
    check(
        unsafe {
            WHvSetPartitionProperty(
                partition.get(),
                WHvPartitionPropertyCodeCpuidExitList,
                leaves.as_ptr().cast(),
                len,
            )
        },
        CALL,
    )
}

pub fn setup(partition: RawPartition) -> WhpResult<()> {
    // SAFETY: a handle from `WHvCreatePartition` that has not been deleted.
    check(unsafe { WHvSetupPartition(partition.get()) }, "WHvSetupPartition")
}

/// Give a guest-physical range the host bytes behind it.
///
/// `host` must be page-aligned and a whole number of pages, and `gpa`
/// page-aligned; the platform refuses the call otherwise, which is an error
/// rather than a hazard.
///
/// # Safety
/// This is the platform's one *retaining* verb: the hypervisor keeps `host`'s
/// address past the call and reads and writes through it while the guest runs,
/// so the borrow in this signature ends long before the access does. The
/// caller owns that lifetime — `host` must stay allocated, at the same
/// address, and unmoved, until the range is unmapped or the partition is
/// deleted, whichever comes first. Freeing or reallocating mapped memory
/// leaves the guest running against pages the host no longer owns.
pub unsafe fn map_gpa(
    partition: RawPartition,
    host: &mut [u8],
    gpa: u64,
    perms: GpaPerms,
) -> WhpResult<()> {
    const CALL: &str = "WHvMapGpaRange";
    check(
        WHvMapGpaRange(
            partition.get(),
            host.as_mut_ptr().cast(),
            gpa,
            host.len() as u64,
            map_flags(perms),
        ),
        CALL,
    )
}

pub fn unmap_gpa(partition: RawPartition, gpa: u64, len: u64) -> WhpResult<()> {
    // SAFETY: no pointer crosses; the partition handle is live.
    check(unsafe { WHvUnmapGpaRange(partition.get(), gpa, len) }, "WHvUnmapGpaRange")
}

pub fn dirty_bitmap(
    partition: RawPartition,
    gpa: u64,
    len: u64,
    out: &mut [u64],
) -> WhpResult<()> {
    const CALL: &str = "WHvQueryGpaRangeDirtyBitmap";
    let bytes = core::mem::size_of_val(out);
    let Ok(bitmap_len) = u32::try_from(bytes) else {
        return Err(WhpError::contract(CALL));
    };
    // SAFETY: the platform writes at most `bitmap_len` bytes, which is exactly
    // what the slice owns.
    check(
        unsafe {
            WHvQueryGpaRangeDirtyBitmap(partition.get(), gpa, len, out.as_mut_ptr(), bitmap_len)
        },
        CALL,
    )
}

pub fn create_vp(partition: RawPartition, index: u32) -> WhpResult<()> {
    // SAFETY: the handle is live and the flags word is the documented zero.
    check(
        unsafe { WHvCreateVirtualProcessor(partition.get(), index, 0) },
        "WHvCreateVirtualProcessor",
    )
}

/// Release one virtual processor.
///
/// # Safety
/// As [`delete_partition`], and owned by the caller for the same reason:
/// `partition` must still be live, `index` must name a processor
/// [`create_vp`] created and that has not been deleted, and this must be its
/// last use.
pub unsafe fn delete_vp(partition: RawPartition, index: u32) {
    // Logged rather than propagated, for the reason `delete_partition` gives.
    let hresult = WHvDeleteVirtualProcessor(partition.get(), index);
    if hresult < 0 {
        tracing::warn!(
            index,
            hresult = format_args!("{:#010x}", hresult as u32),
            "WHvDeleteVirtualProcessor refused"
        );
    }
}

pub fn run_vp(partition: RawPartition, index: u32) -> WhpResult<Exit> {
    const CALL: &str = "WHvRunVirtualProcessor";
    // SAFETY: every field of the exit context is plain data for which the
    // all-zero pattern is valid, and the platform overwrites what it uses.
    let mut context: WHV_RUN_VP_EXIT_CONTEXT = unsafe { core::mem::zeroed() };
    let size = core::mem::size_of::<WHV_RUN_VP_EXIT_CONTEXT>() as u32;
    // SAFETY: `context` is live for the call and `size` is its exact width.
    check(
        unsafe {
            WHvRunVirtualProcessor(
                partition.get(),
                index,
                core::ptr::from_mut(&mut context).cast(),
                size,
            )
        },
        CALL,
    )?;
    Ok(decode_exit(&context))
}

pub fn cancel_vp(partition: RawPartition, index: u32) -> WhpResult<()> {
    // SAFETY: the handle is live and the flags word is the documented zero.
    // This is the one verb the platform documents as callable from a thread
    // other than the one inside `WHvRunVirtualProcessor`.
    check(
        unsafe { WHvCancelRunVirtualProcessor(partition.get(), index, 0) },
        "WHvCancelRunVirtualProcessor",
    )
}

pub fn request_interrupt(
    partition: RawPartition,
    request: InterruptRequest,
) -> WhpResult<()> {
    let control = WHV_INTERRUPT_CONTROL {
        _bitfield: request.control_word(),
        Destination: request.destination,
        Vector: request.vector,
    };
    let size = core::mem::size_of::<WHV_INTERRUPT_CONTROL>() as u32;
    // SAFETY: `control` is live for the call and `size` is its exact width.
    check(
        unsafe { WHvRequestInterrupt(partition.get(), &control, size) },
        "WHvRequestInterrupt",
    )
}

/// The platform value for each counter set this port reads. Exhaustive, so a
/// set added to the enum must be named here (R5).
const fn counter_set_code(set: CounterSet) -> WHV_PROCESSOR_COUNTER_SET {
    match set {
        CounterSet::Intercepts => WHvProcessorCounterSetIntercepts,
        CounterSet::Runtime => WHvProcessorCounterSetRuntime,
    }
}

/// Read one counter set into `out`, answering how many bytes the platform
/// wrote.
///
/// The buffer is `u64`-shaped because every counter structure the platform
/// defines is a run of `u64`s; a byte buffer would carry no guarantee of the
/// alignment those words are read back at.
pub fn get_counters(
    partition: RawPartition,
    index: u32,
    set: CounterSet,
    out: &mut [u64],
) -> WhpResult<usize> {
    const CALL: &str = "WHvGetVirtualProcessorCounters";
    let Ok(len) = u32::try_from(core::mem::size_of_val(out)) else {
        return Err(WhpError::contract(CALL));
    };
    let mut written = 0u32;
    // SAFETY: the platform writes at most `len` bytes, and `len` is exactly
    // what the slice owns; `written` is a live, correctly typed out-parameter.
    // The buffer outlives the call because it is borrowed for it.
    check(
        unsafe {
            WHvGetVirtualProcessorCounters(
                partition.get(),
                index,
                counter_set_code(set),
                out.as_mut_ptr().cast(),
                len,
                &mut written,
            )
        },
        CALL,
    )?;
    usize::try_from(written).map_err(|_| WhpError::contract(CALL))
}

/// Read the processor's whole extended-state area, answering how many bytes
/// the platform wrote.
///
/// The buffer is the guest's XSAVE area exactly as the architecture lays it
/// out — legacy `FXSAVE` region, header, then components — because that is the
/// only shape the platform offers the x87 and vector file in:
/// `WHV_REGISTER_NAME` stops at the XMM names and carries no YMM or ZMM
/// register at all.
pub fn get_xsave(
    partition: RawPartition,
    index: u32,
    out: &mut [u8],
) -> WhpResult<usize> {
    const CALL: &str = "WHvGetVirtualProcessorXsaveState";
    let Ok(len) = u32::try_from(out.len()) else {
        return Err(WhpError::contract(CALL));
    };
    let mut written = 0u32;
    // SAFETY: the platform writes at most `len` bytes, and `len` is exactly
    // what the slice owns; `written` is a live, correctly typed out-parameter.
    // The buffer outlives the call because it is borrowed for it.
    check(
        unsafe {
            WHvGetVirtualProcessorXsaveState(
                partition.get(),
                index,
                out.as_mut_ptr().cast(),
                len,
                &mut written,
            )
        },
        CALL,
    )?;
    // The architecture's least XSAVE area: the 512-byte legacy region plus
    // its 64-byte header. An answer shorter than its own header is outside
    // the contract, and refusing it here is what lets a caller index the
    // header without doubting it.
    match usize::try_from(written) {
        Ok(bytes) if bytes >= 576 => Ok(bytes),
        _ => Err(WhpError::contract(CALL)),
    }
}

/// Write the processor's whole extended-state area — the counterpart of
/// [`get_xsave`], taking the same layout back.
pub fn set_xsave(partition: RawPartition, index: u32, area: &[u8]) -> WhpResult<()> {
    const CALL: &str = "WHvSetVirtualProcessorXsaveState";
    let Ok(len) = u32::try_from(area.len()) else {
        return Err(WhpError::contract(CALL));
    };
    // SAFETY: the platform reads at most `len` bytes, and `len` is exactly
    // what the slice owns; the buffer outlives the call because it is borrowed
    // for it.
    check(
        unsafe {
            WHvSetVirtualProcessorXsaveState(partition.get(), index, area.as_ptr().cast(), len)
        },
        CALL,
    )?;
    Ok(())
}

/// The register-read choke point (R5), mirroring `set_values`: every typed
/// getter converges here, so the length agreement and the aligned buffer are
/// established once and each getter differs only in which member of the value
/// union it reads back.
fn get_values(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    wanted: usize,
) -> WhpResult<[RegVal; SET_MAX]> {
    const CALL: &str = "WHvGetVirtualProcessorRegisters";
    if regs.len() != wanted || regs.len() > SET_MAX {
        return Err(WhpError::contract(CALL));
    }
    let mut names = [0 as WHV_REGISTER_NAME; SET_MAX];
    let mut values = [RegVal::zeroed(); SET_MAX];
    for (slot, reg) in names.iter_mut().zip(regs) {
        *slot = register_name(*reg);
    }
    // SAFETY: both arrays are live and hold at least `regs.len()` entries;
    // `values` is `RegVal`, so its stride and alignment are the 16 the SDK
    // asks for.
    check(
        unsafe {
            WHvGetVirtualProcessorRegisters(
                partition.get(),
                index,
                names.as_ptr(),
                regs.len() as u32,
                values.as_mut_ptr().cast(),
            )
        },
        CALL,
    )?;
    Ok(values)
}

pub fn get_words(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    out: &mut [u64],
) -> WhpResult<()> {
    let values = get_values(partition, index, regs, out.len())?;
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value.as_word();
    }
    Ok(())
}

/// Read registers of mixed shape in one call.
///
/// The whole architectural state of a processor is one transfer rather than
/// three, which matters because a machine running a guest makes one of these
/// per slice and the cost is the call rather than what it carries. Each value
/// is taken from the union member its register names, decided by
/// [`sys::shape_of`] so a caller cannot ask for the wrong one.
pub fn get_registers(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    out: &mut [RegisterValue],
) -> WhpResult<()> {
    let values = get_values(partition, index, regs, out.len())?;
    for ((slot, value), reg) in out.iter_mut().zip(values).zip(regs) {
        *slot = match shape_of(*reg) {
            RegisterValue::Word(_) => RegisterValue::Word(value.as_word()),
            RegisterValue::Segment(_) => RegisterValue::Segment(value.as_segment()),
            RegisterValue::Table(_) => RegisterValue::Table(value.as_table()),
            RegisterValue::Words128(_) => RegisterValue::Words128(value.as_words128()),
        };
    }
    Ok(())
}

/// Write registers of mixed shape in one call. The counterpart of
/// [`get_registers`].
pub fn set_registers(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    values: &[RegisterValue],
) -> WhpResult<()> {
    let mut raw = [RegVal::zeroed(); SET_MAX];
    if values.len() > SET_MAX {
        return Err(WhpError::contract("WHvSetVirtualProcessorRegisters"));
    }
    for (slot, value) in raw.iter_mut().zip(values) {
        *slot = match *value {
            RegisterValue::Word(word) => RegVal::word(word),
            RegisterValue::Segment(seg) => RegVal::segment(seg),
            RegisterValue::Table(table) => RegVal::table(table),
            RegisterValue::Words128(words) => RegVal::words128(words),
        };
    }
    set_values(partition, index, regs, values.len(), &raw)
}

pub fn get_segments(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    out: &mut [SegmentRegister],
) -> WhpResult<()> {
    let values = get_values(partition, index, regs, out.len())?;
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value.as_segment();
    }
    Ok(())
}

pub fn get_tables(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    out: &mut [TableRegister],
) -> WhpResult<()> {
    let values = get_values(partition, index, regs, out.len())?;
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value.as_table();
    }
    Ok(())
}

pub fn set_words(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    words: &[u64],
) -> WhpResult<()> {
    let mut values = [RegVal::zeroed(); SET_MAX];
    for (slot, word) in values.iter_mut().zip(words) {
        *slot = RegVal::word(*word);
    }
    set_values(partition, index, regs, words.len(), &values)
}

pub fn set_segments(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    segments: &[SegmentRegister],
) -> WhpResult<()> {
    let mut values = [RegVal::zeroed(); SET_MAX];
    for (slot, seg) in values.iter_mut().zip(segments) {
        *slot = RegVal::segment(*seg);
    }
    set_values(partition, index, regs, segments.len(), &values)
}

pub fn set_tables(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    tables: &[TableRegister],
) -> WhpResult<()> {
    let mut values = [RegVal::zeroed(); SET_MAX];
    for (slot, table) in values.iter_mut().zip(tables) {
        *slot = RegVal::table(*table);
    }
    set_values(partition, index, regs, tables.len(), &values)
}

/// How many registers one platform call may name. The platform takes an array
/// of names and an array of values, so a fixed buffer bounds a single call;
/// callers batch larger transfers themselves.
///
/// Large enough to hold every word-shaped register an engine exchanges at once
/// — the general registers, the control and debug registers and the model
/// specific registers together — because the cost of a transfer is the call and
/// not the registers in it. A caller forced to split that in two pays twice for
/// the same information, once per slice, for as long as a guest runs. At
/// sixteen bytes a value this buffer is a kilobyte of stack.
const SET_MAX: usize = 64;

/// The register-write choke point (R5): both typed setters converge here, so
/// the length agreement and the aligned buffer are established once.
fn set_values(
    partition: RawPartition,
    index: u32,
    regs: &[Reg],
    supplied: usize,
    values: &[RegVal; SET_MAX],
) -> WhpResult<()> {
    const CALL: &str = "WHvSetVirtualProcessorRegisters";
    if regs.len() != supplied || regs.len() > SET_MAX {
        return Err(WhpError::contract(CALL));
    }
    let mut names = [0 as WHV_REGISTER_NAME; SET_MAX];
    for (slot, reg) in names.iter_mut().zip(regs) {
        *slot = register_name(*reg);
    }
    // SAFETY: both arrays are live and hold at least `regs.len()` entries, and
    // `values` is `RegVal`, carrying the SDK's required 16-byte alignment.
    check(
        unsafe {
            WHvSetVirtualProcessorRegisters(
                partition.get(),
                index,
                names.as_ptr(),
                regs.len() as u32,
                values.as_ptr().cast(),
            )
        },
        CALL,
    )
}

pub fn translate_gva(
    partition: RawPartition,
    index: u32,
    gva: u64,
) -> WhpResult<GvaTranslation> {
    const CALL: &str = "WHvTranslateGva";
    let mut result = WHV_TRANSLATE_GVA_RESULT { ResultCode: 0, Reserved: 0 };
    let mut gpa = 0u64;
    // SAFETY: both out-parameters are live and correctly typed.
    check(
        unsafe {
            WHvTranslateGva(
                partition.get(),
                index,
                gva,
                WHvTranslateGvaFlagValidateRead,
                &mut result,
                &mut gpa,
            )
        },
        CALL,
    )?;
    Ok(GvaTranslation { result_code: result.ResultCode, gpa })
}

/// Which APIC register a `WHV_X64_APIC_WRITE_TYPE` names, or `None` for a value
/// the SDK does not define.
///
/// The five constants are the xAPIC MMIO offsets themselves, which
/// [`ApicWriteType::mmio_offset`] restates on the other side; this pairing is
/// what keeps the two from drifting, and the test below checks it.
#[allow(
    non_upper_case_globals,
    reason = "\
        the match arms are the SDK's own `WHvX64ApicWriteType*` constants used \
        as patterns; spelling them any other way would cost the grep that \
        connects each arm to WinHvPlatformDefs.h"
)]
const fn apic_write_type(raw: WHV_X64_APIC_WRITE_TYPE) -> Option<ApicWriteType> {
    match raw {
        WHvX64ApicWriteTypeLdr => Some(ApicWriteType::Ldr),
        WHvX64ApicWriteTypeDfr => Some(ApicWriteType::Dfr),
        WHvX64ApicWriteTypeSvr => Some(ApicWriteType::Svr),
        WHvX64ApicWriteTypeLint0 => Some(ApicWriteType::Lint0),
        WHvX64ApicWriteTypeLint1 => Some(ApicWriteType::Lint1),
        _ => None,
    }
}

/// Read the platform's exit union into this crate's exhaustive enum. The one
/// place a `WHV_RUN_VP_EXIT_CONTEXT` is ever inspected.
#[allow(
    non_upper_case_globals,
    reason = "\
        the match arms are the SDK's own `WHvRunVpExitReason*` constants used \
        as patterns; spelling them any other way would cost the grep that \
        connects each arm to WinHvPlatformDefs.h"
)]
fn decode_exit(context: &WHV_RUN_VP_EXIT_CONTEXT) -> Exit {
    // SAFETY: `ExecutionState` is a union of a bitfield struct and the `u16`
    // read here, both 2 bytes of plain data.
    let execution_state = unsafe { context.VpContext.ExecutionState.AsUINT16 };
    // SAFETY: as above — the segment attribute union's two members are the
    // bitfield and this `u16`.
    let cs_attributes = unsafe { context.VpContext.Cs.Anonymous.Attributes };
    let vp = VpContext {
        rip: context.VpContext.Rip,
        rflags: context.VpContext.Rflags,
        cs: SegmentRegister {
            base: context.VpContext.Cs.Base,
            limit: context.VpContext.Cs.Limit,
            selector: context.VpContext.Cs.Selector,
            attributes: cs_attributes,
        },
        // `InstructionLength` occupies the low nibble of the packed byte and
        // `Cr8` the high one.
        instruction_length: context.VpContext._bitfield & 0x0F,
        cr8: context.VpContext._bitfield >> 4,
        execution_state,
    };

    // SAFETY of every arm below: the exit reason is what the platform
    // contract says selects the union member, so each arm reads the member
    // that reason names and no other.
    let reason = match context.ExitReason {
        WHvRunVpExitReasonNone => ExitReason::None,
        WHvRunVpExitReasonMemoryAccess => {
            let access = unsafe { context.Anonymous.MemoryAccess };
            let info = unsafe { access.AccessInfo.AsUINT32 };
            ExitReason::MemoryAccess(MemoryAccess {
                gpa: access.Gpa,
                gva: access.Gva,
                gva_valid: info >> 3 & 1 != 0,
                access: match info & 0b11 {
                    0 => AccessType::Read,
                    1 => AccessType::Write,
                    2 => AccessType::Execute,
                    other => AccessType::Unknown(other),
                },
                gpa_unmapped: info >> 2 & 1 != 0,
                instruction_byte_count: access.InstructionByteCount,
                instruction_bytes: access.InstructionBytes,
            })
        }
        WHvRunVpExitReasonX64IoPortAccess => {
            let access = unsafe { context.Anonymous.IoPortAccess };
            let info = unsafe { access.AccessInfo.AsUINT32 };
            ExitReason::IoPortAccess(IoPortAccess {
                port: access.PortNumber,
                is_write: info & 1 != 0,
                access_size: (info >> 1 & 0b111) as u8,
                string_op: info >> 4 & 1 != 0,
                rep_prefix: info >> 5 & 1 != 0,
                rax: access.Rax,
                rcx: access.Rcx,
                rsi: access.Rsi,
                rdi: access.Rdi,
            })
        }
        WHvRunVpExitReasonUnrecoverableException => ExitReason::UnrecoverableException,
        WHvRunVpExitReasonInvalidVpRegisterValue => ExitReason::InvalidVpRegisterValue,
        WHvRunVpExitReasonUnsupportedFeature => {
            let feature = unsafe { context.Anonymous.UnsupportedFeature };
            ExitReason::UnsupportedFeature {
                code: feature.FeatureCode,
                parameter: feature.FeatureParameter,
            }
        }
        WHvRunVpExitReasonX64InterruptWindow => ExitReason::InterruptWindow,
        WHvRunVpExitReasonX64Halt => ExitReason::Halt,
        WHvRunVpExitReasonX64ApicEoi => {
            let eoi = unsafe { context.Anonymous.ApicEoi };
            ExitReason::ApicEoi { vector: eoi.InterruptVector }
        }
        WHvRunVpExitReasonSynicSintDeliverable => ExitReason::SynicSintDeliverable,
        WHvRunVpExitReasonX64MsrAccess => {
            let access = unsafe { context.Anonymous.MsrAccess };
            let info = unsafe { access.AccessInfo.AsUINT32 };
            ExitReason::MsrAccess(MsrAccess {
                msr: access.MsrNumber,
                is_write: info & 1 != 0,
                rax: access.Rax,
                rdx: access.Rdx,
            })
        }
        WHvRunVpExitReasonX64Cpuid => {
            let cpuid = unsafe { context.Anonymous.CpuidAccess };
            ExitReason::Cpuid(CpuidAccess {
                rax: cpuid.Rax,
                rcx: cpuid.Rcx,
                rdx: cpuid.Rdx,
                rbx: cpuid.Rbx,
                default_rax: cpuid.DefaultResultRax,
                default_rcx: cpuid.DefaultResultRcx,
                default_rdx: cpuid.DefaultResultRdx,
                default_rbx: cpuid.DefaultResultRbx,
            })
        }
        WHvRunVpExitReasonException => ExitReason::Exception,
        WHvRunVpExitReasonX64Rdtsc => ExitReason::Rdtsc,
        WHvRunVpExitReasonX64ApicSmiTrap => ExitReason::ApicSmiTrap,
        WHvRunVpExitReasonHypercall => ExitReason::Hypercall,
        WHvRunVpExitReasonX64ApicInitSipiTrap => ExitReason::ApicInitSipiTrap,
        WHvRunVpExitReasonX64ApicWriteTrap => {
            let write = unsafe { context.Anonymous.ApicWrite };
            match apic_write_type(write.Type) {
                Some(register) => {
                    ExitReason::ApicWriteTrap { register, value: write.WriteValue }
                }
                // A `WHV_X64_APIC_WRITE_TYPE` outside the five the SDK names.
                // Reported as the raw exit rather than guessed at, because
                // mirroring a write into the wrong APIC register is worse than
                // refusing the exit.
                None => ExitReason::Unrecognized(context.ExitReason),
            }
        }
        WHvRunVpExitReasonCanceled => {
            let cancel = unsafe { context.Anonymous.CancelReason };
            ExitReason::Canceled { reason: cancel.CancelReason }
        }
        other => ExitReason::Unrecognized(other),
    };
    Exit { vp, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcpu::{ALL_REGS, UNEXCHANGED_REGS};

    /// No two registers may share a platform name.
    ///
    /// `register_name` is one exhaustive match over fifty-odd arms, so a
    /// register added to the enum cannot be forgotten — the compiler refuses.
    /// What the compiler cannot see is a copy-paste slip WITHIN it: writing
    /// `Reg::R11 => WHvX64RegisterR10` type-checks perfectly and silently
    /// transfers the wrong register, which is the failure this catches. Both
    /// lists are swept, so a register outside the per-slice exchange is held to
    /// the same rule.
    #[test]
    fn every_register_names_a_different_platform_register() {
        let every: Vec<Reg> = ALL_REGS.iter().chain(UNEXCHANGED_REGS).copied().collect();
        for (position, reg) in every.iter().enumerate() {
            let name = register_name(*reg);
            for other in &every[position + 1..] {
                assert_ne!(
                    name,
                    register_name(*other),
                    "{reg:?} and {other:?} both map to platform register {name}"
                );
            }
        }
    }

    /// The SDK's `WHV_X64_APIC_WRITE_TYPE` values ARE the xAPIC MMIO offsets,
    /// which is the identity [`ApicWriteType::mmio_offset`] rests on. Pinned
    /// here because the two are transcribed in different files: a decode arm
    /// that named the wrong register would still produce a plausible exit, and
    /// the host would mirror a LINT0 write into its SVR.
    #[test]
    fn each_apic_write_type_is_its_own_mmio_offset() {
        let each = [
            (WHvX64ApicWriteTypeLdr, ApicWriteType::Ldr),
            (WHvX64ApicWriteTypeDfr, ApicWriteType::Dfr),
            (WHvX64ApicWriteTypeSvr, ApicWriteType::Svr),
            (WHvX64ApicWriteTypeLint0, ApicWriteType::Lint0),
            (WHvX64ApicWriteTypeLint1, ApicWriteType::Lint1),
        ];
        for (raw, register) in each {
            assert_eq!(apic_write_type(raw), Some(register));
            assert_eq!(
                i32::from(register.mmio_offset()),
                raw,
                "{register:?}'s platform value is its MMIO offset"
            );
        }
        assert_eq!(apic_write_type(-1), None, "a value the SDK does not name");
    }

    /// The 128-bit member is the whole value, so a pair of words survives the
    /// union and comes back in the order it went in — a swapped pair would put
    /// a pending event's reserved half where its vector belongs.
    #[test]
    fn a_hundred_and_twenty_eight_bit_value_round_trips_low_half_first() {
        let words = [0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210];
        assert_eq!(RegVal::words128(words).as_words128(), words);
        // The low half is the same eight bytes a word-shaped read would return,
        // which is what makes reading a 128-bit register as a word a silent
        // truncation rather than a visible error.
        assert_eq!(RegVal::words128(words).as_word(), words[0]);
    }

    /// The general-purpose registers are consecutive in the platform's own
    /// numbering, in the architectural order. Bochs and the SDK agree on that
    /// order, so a transposition among the eight new ones shows up as a gap.
    #[test]
    fn the_general_purpose_registers_run_in_architectural_order() {
        let names: Vec<_> = [
            Reg::Rax, Reg::Rcx, Reg::Rdx, Reg::Rbx, Reg::Rsp, Reg::Rbp, Reg::Rsi, Reg::Rdi,
            Reg::R8, Reg::R9, Reg::R10, Reg::R11, Reg::R12, Reg::R13, Reg::R14, Reg::R15,
        ]
        .iter()
        .map(|reg| register_name(*reg))
        .collect();
        for pair in names.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "the platform numbers its general-purpose registers consecutively"
            );
        }
    }
}
