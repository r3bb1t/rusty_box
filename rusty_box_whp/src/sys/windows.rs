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
use crate::sys::{
    CapabilityCode, CounterSet, GpaPerms, GvaTranslation, PropertyCode, RawPartition,
};
use crate::vcpu::{
    AccessType, CpuidAccess, Exit, ExitReason, InterruptRequest, IoPortAccess, MemoryAccess,
    MsrAccess, Reg, SegmentRegister, TableRegister, VpContext,
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

pub(crate) fn hypervisor_present() -> WhpResult<bool> {
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

pub(crate) fn capability(code: CapabilityCode) -> WhpResult<u64> {
    const CALL: &str = "WHvGetCapability";
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

pub(crate) fn create_partition() -> WhpResult<RawPartition> {
    const CALL: &str = "WHvCreatePartition";
    let mut handle: WHV_PARTITION_HANDLE = 0;
    // SAFETY: `handle` is a live, correctly typed out-parameter.
    check(unsafe { WHvCreatePartition(&mut handle) }, CALL)?;
    RawPartition::new(handle, CALL)
}

pub(crate) fn delete_partition(partition: RawPartition) {
    // SAFETY: the handle came from `WHvCreatePartition` and this is its only
    // deletion — `Partition`'s `Drop` is the sole caller and consumes the
    // owner, so no second delete of the same handle is reachable.
    //
    // The result is deliberately not propagated: this runs from `Drop`, which
    // cannot report, and the only documented failure is an already-invalid
    // handle. It is logged rather than discarded so a leak is still visible.
    let hresult = unsafe { WHvDeletePartition(partition.get()) };
    if hresult < 0 {
        tracing::warn!(
            hresult = format_args!("{:#010x}", hresult as u32),
            "WHvDeletePartition refused; the partition's host resources are leaked"
        );
    }
}

pub(crate) fn set_property(
    partition: RawPartition,
    code: PropertyCode,
    value: u64,
) -> WhpResult<()> {
    let (raw, len) = property_code(code);
    // SAFETY: the platform reads `len` bytes, and `len` is at most the 8 this
    // local holds — `property_code` guarantees that pairing.
    check(
        unsafe {
            WHvSetPartitionProperty(
                partition.get(),
                raw,
                core::ptr::from_ref(&value).cast(),
                len,
            )
        },
        "WHvSetPartitionProperty",
    )
}

pub(crate) fn set_cpuid_exit_list(partition: RawPartition, leaves: &[u32]) -> WhpResult<()> {
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

pub(crate) fn setup(partition: RawPartition) -> WhpResult<()> {
    // SAFETY: a handle from `WHvCreatePartition` that has not been deleted.
    check(unsafe { WHvSetupPartition(partition.get()) }, "WHvSetupPartition")
}

pub(crate) fn map_gpa(
    partition: RawPartition,
    host: &mut [u8],
    gpa: u64,
    perms: GpaPerms,
) -> WhpResult<()> {
    const CALL: &str = "WHvMapGpaRange";
    // SAFETY: `host` is a live, page-aligned, page-multiple allocation whose
    // lifetime the caller ties to the mapping — `Partition::map` owns the
    // pages it maps, so the host memory cannot outlive or predecease the
    // partition that reads it.
    check(
        unsafe {
            WHvMapGpaRange(
                partition.get(),
                host.as_mut_ptr().cast(),
                gpa,
                host.len() as u64,
                map_flags(perms),
            )
        },
        CALL,
    )
}

pub(crate) fn unmap_gpa(partition: RawPartition, gpa: u64, len: u64) -> WhpResult<()> {
    // SAFETY: no pointer crosses; the partition handle is live.
    check(unsafe { WHvUnmapGpaRange(partition.get(), gpa, len) }, "WHvUnmapGpaRange")
}

pub(crate) fn dirty_bitmap(
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

pub(crate) fn create_vp(partition: RawPartition, index: u32) -> WhpResult<()> {
    // SAFETY: the handle is live and the flags word is the documented zero.
    check(
        unsafe { WHvCreateVirtualProcessor(partition.get(), index, 0) },
        "WHvCreateVirtualProcessor",
    )
}

pub(crate) fn delete_vp(partition: RawPartition, index: u32) {
    // SAFETY: as `delete_partition` — reached only from `Drop`, once per
    // created processor. Logged rather than propagated for the same reason.
    let hresult = unsafe { WHvDeleteVirtualProcessor(partition.get(), index) };
    if hresult < 0 {
        tracing::warn!(
            index,
            hresult = format_args!("{:#010x}", hresult as u32),
            "WHvDeleteVirtualProcessor refused"
        );
    }
}

pub(crate) fn run_vp(partition: RawPartition, index: u32) -> WhpResult<Exit> {
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

pub(crate) fn cancel_vp(partition: RawPartition, index: u32) -> WhpResult<()> {
    // SAFETY: the handle is live and the flags word is the documented zero.
    // This is the one verb the platform documents as callable from a thread
    // other than the one inside `WHvRunVirtualProcessor`.
    check(
        unsafe { WHvCancelRunVirtualProcessor(partition.get(), index, 0) },
        "WHvCancelRunVirtualProcessor",
    )
}

pub(crate) fn request_interrupt(
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
pub(crate) fn get_counters(
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

pub(crate) fn get_words(
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

pub(crate) fn get_segments(
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

pub(crate) fn get_tables(
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

pub(crate) fn set_words(
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

pub(crate) fn set_segments(
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

pub(crate) fn set_tables(
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
const SET_MAX: usize = 32;

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

pub(crate) fn translate_gva(
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
        WHvRunVpExitReasonX64ApicWriteTrap => ExitReason::ApicWriteTrap,
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
    use crate::vcpu::ALL_REGS;

    /// No two registers may share a platform name.
    ///
    /// `register_name` is one exhaustive match over fifty-odd arms, so a
    /// register added to the enum cannot be forgotten — the compiler refuses.
    /// What the compiler cannot see is a copy-paste slip WITHIN it: writing
    /// `Reg::R11 => WHvX64RegisterR10` type-checks perfectly and silently
    /// transfers the wrong register, which is the failure this catches.
    #[test]
    fn every_register_names_a_different_platform_register() {
        for (position, reg) in ALL_REGS.iter().enumerate() {
            let name = register_name(*reg);
            for other in &ALL_REGS[position + 1..] {
                assert_ne!(
                    name,
                    register_name(*other),
                    "{reg:?} and {other:?} both map to platform register {name}"
                );
            }
        }
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
