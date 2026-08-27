//! What this host's hypervisor says it can do.
//!
//! Every bit position below is transcribed from the Windows SDK's
//! `WinHvPlatformDefs.h`, from the union named in each doc comment. The SDK
//! declares them as C bitfields, and the `windows` crate surfaces those as an
//! opaque `_bitfield` word, so the positions have to be restated here — which
//! makes them exactly the kind of spec-defined table that is worth reading off
//! the header rather than recalling.

use crate::error::WhpResult;
use crate::sys;

/// Bit positions within `WHV_CAPABILITY_FEATURES` (AMD64 layout).
mod feature_bit {
    pub(super) const PARTIAL_UNMAP: u32 = 0;
    pub(super) const LOCAL_APIC_EMULATION: u32 = 1;
    pub(super) const XSAVE: u32 = 2;
    pub(super) const DIRTY_PAGE_TRACKING: u32 = 3;
    pub(super) const SPECULATION_CONTROL: u32 = 4;
    pub(super) const APIC_REMOTE_READ: u32 = 5;
    pub(super) const IDLE_SUSPEND: u32 = 6;
}

/// Bit positions within `WHV_EXTENDED_VM_EXITS` (AMD64 layout). Note that the
/// four `X64ApicWrite*ExitTrap` bits sit between `HypercallExit` and
/// `GpaAccessFaultExit`, which is why the last one lands at 14 and not lower.
mod exit_bit {
    pub(super) const CPUID: u32 = 0;
    pub(super) const MSR: u32 = 1;
    pub(super) const EXCEPTION: u32 = 2;
    pub(super) const RDTSC: u32 = 3;
    pub(super) const APIC_SMI_TRAP: u32 = 4;
    pub(super) const HYPERCALL: u32 = 5;
    pub(super) const GPA_ACCESS_FAULT: u32 = 14;
}

/// The subset of `WHV_CAPABILITY_FEATURES` this port has a use for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// `WHvUnmapGpaRange` may name a sub-range of an existing mapping.
    pub partial_unmap: bool,
    /// `WHvPartitionPropertyCodeLocalApicEmulationMode` is settable.
    pub local_apic_emulation: bool,
    /// The partition can carry guest XSAVE state.
    pub xsave: bool,
    /// `WHvMapGpaRangeFlagTrackDirtyPages` and
    /// `WHvQueryGpaRangeDirtyBitmap` are available.
    pub dirty_page_tracking: bool,
    /// Speculation-control MSRs are exposed to the guest.
    pub speculation_control: bool,
    /// A remote read of another VP's APIC is supported.
    pub apic_remote_read: bool,
    /// The hypervisor can suspend an idle VP.
    pub idle_suspend: bool,
    /// The whole word, so a bit this port has no name for is still reportable.
    pub raw: u64,
}

/// The subset of `WHV_EXTENDED_VM_EXITS` this port has a use for. The same
/// union is both the capability answer and the partition property, so one type
/// serves for asking and for setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtendedVmExits {
    /// `CPUID` leaves named by `CpuidExitList` exit instead of executing.
    pub cpuid: bool,
    /// `RDMSR`/`WRMSR` exit.
    pub msr: bool,
    /// Exceptions named by `ExceptionExitBitmap` exit.
    pub exception: bool,
    /// `RDTSC`/`RDTSCP` exit.
    pub rdtsc: bool,
    /// An APIC-written SMI traps out instead of being delivered.
    pub apic_smi_trap: bool,
    /// `VMCALL`/`VMMCALL` exits.
    pub hypercall: bool,
    /// A second-level page fault against a *mapped* range exits, rather than
    /// being turned into a guest fault. This is the bit that decides whether a
    /// write to a read-only shadowed-ROM window can be serviced by the host.
    pub gpa_access_fault: bool,
}

impl ExtendedVmExits {
    /// The word to hand `WHvSetPartitionProperty`.
    #[must_use]
    pub const fn as_word(self) -> u64 {
        (self.cpuid as u64) << exit_bit::CPUID
            | (self.msr as u64) << exit_bit::MSR
            | (self.exception as u64) << exit_bit::EXCEPTION
            | (self.rdtsc as u64) << exit_bit::RDTSC
            | (self.apic_smi_trap as u64) << exit_bit::APIC_SMI_TRAP
            | (self.hypercall as u64) << exit_bit::HYPERCALL
            | (self.gpa_access_fault as u64) << exit_bit::GPA_ACCESS_FAULT
    }

    /// Read the word back into named bits.
    #[must_use]
    pub const fn from_word(word: u64) -> Self {
        const fn bit(word: u64, at: u32) -> bool {
            word >> at & 1 != 0
        }
        Self {
            cpuid: bit(word, exit_bit::CPUID),
            msr: bit(word, exit_bit::MSR),
            exception: bit(word, exit_bit::EXCEPTION),
            rdtsc: bit(word, exit_bit::RDTSC),
            apic_smi_trap: bit(word, exit_bit::APIC_SMI_TRAP),
            hypercall: bit(word, exit_bit::HYPERCALL),
            gpa_access_fault: bit(word, exit_bit::GPA_ACCESS_FAULT),
        }
    }
}

/// Everything worth asking the platform before building a partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// What the host is willing to do at all.
    pub features: Features,
    /// Which extended exits the host is willing to be asked for. A partition
    /// may only request a subset of these.
    pub supported_exits: ExtendedVmExits,
    /// Guest-physical address width in bits, as the host reports it.
    pub physical_address_width: u32,
}

/// Whether a hypervisor is present and usable from this process.
///
/// This is the one call that answers `false` rather than failing when the
/// platform is absent — every other verb in the crate presumes it said yes.
pub fn hypervisor_present() -> WhpResult<bool> {
    sys::hypervisor_present()
}

/// Ask the host what it can do.
///
/// # Errors
/// [`crate::WhpErrorKind::Unsupported`] when no hypervisor is present, or
/// [`crate::WhpErrorKind::Platform`] when a capability query is refused.
pub fn capabilities() -> WhpResult<Capabilities> {
    const fn bit(word: u64, at: u32) -> bool {
        word >> at & 1 != 0
    }
    let features_word = sys::capability(sys::CapabilityCode::Features)?;
    let exits_word = sys::capability(sys::CapabilityCode::ExtendedVmExits)?;
    // `WHvCapabilityCodePhysicalAddressWidth` is newer than the rest; a host
    // that does not know it refuses rather than answering zero, and 0 is the
    // honest report for "the host would not say".
    let width = sys::capability(sys::CapabilityCode::PhysicalAddressWidth).unwrap_or(0);
    Ok(Capabilities {
        features: Features {
            partial_unmap: bit(features_word, feature_bit::PARTIAL_UNMAP),
            local_apic_emulation: bit(features_word, feature_bit::LOCAL_APIC_EMULATION),
            xsave: bit(features_word, feature_bit::XSAVE),
            dirty_page_tracking: bit(features_word, feature_bit::DIRTY_PAGE_TRACKING),
            speculation_control: bit(features_word, feature_bit::SPECULATION_CONTROL),
            apic_remote_read: bit(features_word, feature_bit::APIC_REMOTE_READ),
            idle_suspend: bit(features_word, feature_bit::IDLE_SUSPEND),
            raw: features_word,
        },
        supported_exits: ExtendedVmExits::from_word(exits_word),
        physical_address_width: width as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bit this port cares about most is also the one furthest from where
    /// a reader would guess it sits, so pin its position against the SDK's
    /// declared field order rather than trusting the constant alone.
    #[test]
    fn the_second_level_fault_exit_is_the_fifteenth_bit_of_the_word() {
        let only = ExtendedVmExits { gpa_access_fault: true, ..ExtendedVmExits::default() };
        assert_eq!(only.as_word(), 1 << 14);
        assert_eq!(ExtendedVmExits::from_word(1 << 14), only);
    }

    #[test]
    fn a_word_survives_a_round_trip_through_the_named_bits() {
        let asked = ExtendedVmExits {
            cpuid: true,
            msr: false,
            exception: false,
            rdtsc: true,
            apic_smi_trap: false,
            hypercall: false,
            gpa_access_fault: true,
        };
        assert_eq!(ExtendedVmExits::from_word(asked.as_word()), asked);
    }

    /// A partition may only ask for exits the host advertises, so the two uses
    /// of the union have to agree bit for bit.
    #[test]
    fn asking_for_an_unsupported_exit_is_visible_as_a_missing_bit() {
        let host = ExtendedVmExits::from_word(1 << 0);
        let wanted = ExtendedVmExits { cpuid: true, gpa_access_fault: true, ..Default::default() };
        assert_eq!(wanted.as_word() & !host.as_word(), 1 << 14);
    }
}
