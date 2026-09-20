//! What this host's hypervisor says it can do.
//!
//! Every bit position below is transcribed from the Windows SDK's
//! `WinHvPlatformDefs.h`, from the union named in each doc comment. The SDK
//! declares them as C bitfields, and the `windows` crate surfaces those as an
//! opaque `_bitfield` word, so the positions have to be restated here — which
//! makes them exactly the kind of spec-defined table that is worth reading off
//! the header rather than recalling.

use crate::sys::{self, WhpResult};

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
/// Which model-specific register accesses leave the partition.
///
/// Setting [`ExtendedVmExits::msr`] is not by itself enough, and finding that
/// out cost a wrong test: the platform answers a handful of MSRs from inside
/// the hypervisor and exits only for what this names. Everything else — every
/// MSR the platform has no opinion about — is covered by
/// [`MsrExits::unhandled`].
///
/// The six are the whole of `WHV_X64_MSR_EXIT_BITMAP`; there is no way to name
/// an individual MSR.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MsrExits {
    /// Every MSR the platform does not handle itself.
    pub unhandled: bool,
    /// `WRMSR` to `IA32_TIME_STAMP_COUNTER`.
    pub tsc_write: bool,
    /// `RDMSR` of `IA32_TIME_STAMP_COUNTER`. Note this is the MSR and not the
    /// `RDTSC` instruction, which has its own bit in [`ExtendedVmExits`] — a
    /// caller that traps one and not the other leaves a guest two clocks that
    /// disagree.
    pub tsc_read: bool,
    /// `WRMSR` to `IA32_APIC_BASE`.
    pub apic_base_write: bool,
    /// `RDMSR` of `IA32_MISC_ENABLE`.
    pub misc_enable_read: bool,
    /// `RDMSR` of `IA32_BIOS_SIGN_ID`, the microcode revision.
    pub microcode_revision_read: bool,
}

impl MsrExits {
    /// Every MSR access the platform will hand over.
    pub const ALL: Self = Self {
        unhandled: true,
        tsc_write: true,
        tsc_read: true,
        apic_base_write: true,
        misc_enable_read: true,
        microcode_revision_read: true,
    };

    /// The word to hand `WHvSetPartitionProperty`.
    #[must_use]
    pub const fn as_word(self) -> u64 {
        (self.unhandled as u64)
            | (self.tsc_write as u64) << 1
            | (self.tsc_read as u64) << 2
            | (self.apic_base_write as u64) << 3
            | (self.misc_enable_read as u64) << 4
            | (self.microcode_revision_read as u64) << 5
    }
}

mod exit_bit {
    pub(super) const CPUID: u32 = 0;
    pub(super) const MSR: u32 = 1;
    pub(super) const EXCEPTION: u32 = 2;
    pub(super) const RDTSC: u32 = 3;
    pub(super) const APIC_SMI_TRAP: u32 = 4;
    pub(super) const HYPERCALL: u32 = 5;
    pub(super) const APIC_INIT_SIPI_TRAP: u32 = 6;
    pub(super) const APIC_WRITE_LINT0_TRAP: u32 = 7;
    pub(super) const APIC_WRITE_LINT1_TRAP: u32 = 8;
    pub(super) const APIC_WRITE_SVR_TRAP: u32 = 9;
    // 10 and 11 are `UnknownSynicConnection` and `RetargetUnknownVpciDevice`,
    // neither of which this port has a use for.
    pub(super) const APIC_WRITE_LDR_TRAP: u32 = 12;
    pub(super) const APIC_WRITE_DFR_TRAP: u32 = 13;
    pub(super) const GPA_ACCESS_FAULT: u32 = 14;
}

/// Bit positions within `WHV_X64_PROCESSOR_FEATURES1`, the second bank of
/// `WHvCapabilityCodeProcessorFeaturesBanks`.
mod processor_feature1_bit {
    /// `TscDeadlineTmrSupport`, the eighteenth field the union declares.
    pub(super) const TSC_DEADLINE_TIMER: u32 = 17;
}

bitflags::bitflags! {
    /// `WHV_SYNTHETIC_PROCESSOR_FEATURES` bank 0. One flag per header bitfield, in the
    /// header's order; the composite at the end is OpenVMM's VTL0 set, the one the design
    /// exposes. `repr(transparent)` because the word crosses the FFI as the bank's `Bank0`
    /// field (bitflags does not add it on its own).
    ///
    /// The header names further fields above bit 30 — `RestoreTime`, `EnlightenedVmcs`,
    /// `NestedDebugCtl`, `SyntheticTimeUnhaltedTimer`, `IdleSpecCtrl`, `WakeVps` and
    /// `AccessVpRegs`. They are deliberately absent: this type is the set a VTL0 guest is
    /// offered, not a transcription of the union, and a host that sets one of them is still
    /// reported through [`SyntheticFeatures::from_bits_retain`].
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct SyntheticFeatures: u64 {
        /// CPUID leaves 0x40000000 and 0x40000001 are supported.
        const HYPERVISOR_PRESENT = 1 << 0;
        /// CPUID leaves 0x40000000–0x40000006 (the Hv#1 interface).
        const HV1 = 1 << 1;
        const ACCESS_VP_RUNTIME_REG = 1 << 2;
        const ACCESS_PARTITION_REFERENCE_COUNTER = 1 << 3;
        const ACCESS_SYNIC_REGS = 1 << 4;
        const ACCESS_SYNTHETIC_TIMER_REGS = 1 << 5;
        /// The VP assist page and, on x64, the APIC EOI/ICR/TPR MSRs.
        const ACCESS_INTR_CTRL_REGS = 1 << 6;
        const ACCESS_HYPERCALL_REGS = 1 << 7;
        const ACCESS_VP_INDEX = 1 << 8;
        const ACCESS_PARTITION_REFERENCE_TSC = 1 << 9;
        const ACCESS_GUEST_IDLE_REG = 1 << 10;
        const ACCESS_FREQUENCY_REGS = 1 << 11;
        const EXTENDED_GVA_RANGES_FOR_FLUSH = 1 << 15;
        const FAST_HYPERCALL_OUTPUT = 1 << 18;
        const DIRECT_SYNTHETIC_TIMERS = 1 << 22;
        const EXTENDED_PROCESSOR_MASKS = 1 << 24;
        const TB_FLUSH_HYPERCALLS = 1 << 25;
        const SYNTHETIC_CLUSTER_IPI = 1 << 26;
        const NOTIFY_LONG_SPIN_WAIT = 1 << 27;
        const QUERY_NUMA_DISTANCE = 1 << 28;
        const SIGNAL_EVENTS = 1 << 29;
        const RETARGET_DEVICE_INTERRUPT = 1 << 30;
        /// What OpenVMM grants a VTL0 guest with the offloaded APIC — every bit above.
        const OPENVMM_VTL0 = Self::HYPERVISOR_PRESENT.bits() | Self::HV1.bits()
            | Self::ACCESS_VP_RUNTIME_REG.bits() | Self::ACCESS_PARTITION_REFERENCE_COUNTER.bits()
            | Self::ACCESS_SYNIC_REGS.bits() | Self::ACCESS_SYNTHETIC_TIMER_REGS.bits()
            | Self::ACCESS_INTR_CTRL_REGS.bits() | Self::ACCESS_HYPERCALL_REGS.bits()
            | Self::ACCESS_VP_INDEX.bits() | Self::ACCESS_PARTITION_REFERENCE_TSC.bits()
            | Self::ACCESS_GUEST_IDLE_REG.bits() | Self::ACCESS_FREQUENCY_REGS.bits()
            | Self::EXTENDED_GVA_RANGES_FOR_FLUSH.bits() | Self::FAST_HYPERCALL_OUTPUT.bits()
            | Self::DIRECT_SYNTHETIC_TIMERS.bits() | Self::EXTENDED_PROCESSOR_MASKS.bits()
            | Self::TB_FLUSH_HYPERCALLS.bits() | Self::SYNTHETIC_CLUSTER_IPI.bits()
            | Self::NOTIFY_LONG_SPIN_WAIT.bits() | Self::QUERY_NUMA_DISTANCE.bits()
            | Self::SIGNAL_EVENTS.bits() | Self::RETARGET_DEVICE_INTERRUPT.bits();
    }
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
    /// An APIC-written INIT or SIPI traps out instead of being delivered — how
    /// a host owns processor startup while the hypervisor owns the APIC.
    pub apic_init_sipi_trap: bool,
    /// A guest write to the local APIC's LINT0 entry traps out. The bit that
    /// lets a host see a guest reprogramming the pin its 8259 drives.
    pub apic_write_lint0_trap: bool,
    /// The same for LINT1, the NMI pin.
    pub apic_write_lint1_trap: bool,
    /// A guest write to the spurious-interrupt vector register traps out,
    /// which is where the software enable of the whole APIC lives.
    pub apic_write_svr_trap: bool,
    /// A guest write to the logical-destination register traps out. With
    /// [`Self::apic_write_dfr_trap`] this is how a host watching an offloaded
    /// APIC learns that logical IPI routing has been reprogrammed.
    pub apic_write_ldr_trap: bool,
    /// The same for the destination-format register, which chooses between the
    /// flat and clustered logical models.
    pub apic_write_dfr_trap: bool,
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
            | (self.apic_init_sipi_trap as u64) << exit_bit::APIC_INIT_SIPI_TRAP
            | (self.apic_write_lint0_trap as u64) << exit_bit::APIC_WRITE_LINT0_TRAP
            | (self.apic_write_lint1_trap as u64) << exit_bit::APIC_WRITE_LINT1_TRAP
            | (self.apic_write_svr_trap as u64) << exit_bit::APIC_WRITE_SVR_TRAP
            | (self.apic_write_ldr_trap as u64) << exit_bit::APIC_WRITE_LDR_TRAP
            | (self.apic_write_dfr_trap as u64) << exit_bit::APIC_WRITE_DFR_TRAP
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
            apic_init_sipi_trap: bit(word, exit_bit::APIC_INIT_SIPI_TRAP),
            apic_write_lint0_trap: bit(word, exit_bit::APIC_WRITE_LINT0_TRAP),
            apic_write_lint1_trap: bit(word, exit_bit::APIC_WRITE_LINT1_TRAP),
            apic_write_svr_trap: bit(word, exit_bit::APIC_WRITE_SVR_TRAP),
            apic_write_ldr_trap: bit(word, exit_bit::APIC_WRITE_LDR_TRAP),
            apic_write_dfr_trap: bit(word, exit_bit::APIC_WRITE_DFR_TRAP),
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
    ///
    /// This carries only the bits [`ExtendedVmExits`] declares a field for, so
    /// it cannot on its own distinguish an exit the host does not offer from
    /// one this port has no name for. Read it beside
    /// [`Self::extended_exits_raw`] whenever the question is what the host
    /// offers rather than what this port can ask for.
    pub supported_exits: ExtendedVmExits,
    /// The whole `WHV_EXTENDED_VM_EXITS` word, exactly as the host answered it.
    ///
    /// [`ExtendedVmExits::from_word`] can only report a bit it has a field for,
    /// and this port deliberately names none for bits 10 and 11 while a host
    /// newer than this SDK header may set positions above 14. Keeping the word
    /// means such a bit is visible rather than silently dropped: the remainder
    /// this port cannot name is
    /// `extended_exits_raw & !supported_exits.as_word()`, the same shape as the
    /// unnamed remainder of [`Self::synthetic_features`].
    pub extended_exits_raw: u64,
    /// Guest-physical address width in bits, as the host reports it.
    pub physical_address_width: u32,
    /// The processor features the host banks, `WHV_PROCESSOR_FEATURES` as one
    /// raw word. What [`crate::PartitionConfig::processor_features`] may offer
    /// a guest: a partition left at the platform's default gets a narrower
    /// set, and a guest touching a feature the partition was not told about
    /// faults on hardware while working under an interpreter.
    pub processor_features: u64,
    /// Which Hyper-V enlightenments this host will let a partition offer its
    /// guest, `WHV_SYNTHETIC_PROCESSOR_FEATURES_BANKS` bank 0. Read with
    /// [`SyntheticFeatures::from_bits_retain`], so a bit newer than this
    /// port's SDK header survives into a report instead of being dropped; the
    /// unnamed remainder is `bits() & !SyntheticFeatures::all().bits()`.
    pub synthetic_features: SyntheticFeatures,
    /// How fast the platform's virtual processor clock runs, in hertz. Zero
    /// when the host declines the question.
    pub processor_clock_hz: u64,
    /// How fast the platform's interrupt clock runs, in hertz — what a guest's
    /// APIC timer counts against. Zero when the host declines the question.
    pub interrupt_clock_hz: u64,
    /// `WHV_X64_PROCESSOR_FEATURES1.TscDeadlineTmrSupport`: whether the host
    /// will let a guest arm its APIC timer by TSC deadline rather than by
    /// count. Read from bank 1 of the banked processor features, which is
    /// where the features that outgrew the original word live.
    pub tsc_deadline_timer: bool,
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
    let processor_features = sys::capability(sys::CapabilityCode::ProcessorFeatures)?;
    // `WHvCapabilityCodePhysicalAddressWidth` is newer than the rest; a host
    // that does not know it refuses rather than answering zero, and 0 is the
    // honest report for "the host would not say". The two clock frequencies
    // and the two banked reads are newer still and treated the same way: an
    // absent answer is an empty one, and only the capabilities every host has
    // had since this API shipped are allowed to fail the whole query.
    let width = sys::capability(sys::CapabilityCode::PhysicalAddressWidth).unwrap_or(0);
    let processor_clock_hz =
        sys::capability(sys::CapabilityCode::ProcessorClockFrequency).unwrap_or(0);
    let interrupt_clock_hz =
        sys::capability(sys::CapabilityCode::InterruptClockFrequency).unwrap_or(0);
    let synthetic = sys::capability_banks(sys::CapabilityCode::SyntheticProcessorFeaturesBanks)
        .unwrap_or_default();
    let banked = sys::capability_banks(sys::CapabilityCode::ProcessorFeaturesBanks)
        .unwrap_or_default();
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
        // Kept whole beside the decoded bits: a decode drops every position it
        // has no field for, and a report that shows only the decode cannot say
        // whether an absent exit was absent from the host or from this port.
        extended_exits_raw: exits_word,
        physical_address_width: width as u32,
        processor_features,
        // Retained rather than truncated: a bit this SDK's header does not name
        // is still something a probe report must be able to show.
        synthetic_features: SyntheticFeatures::from_bits_retain(synthetic.bank(0)),
        processor_clock_hz,
        interrupt_clock_hz,
        tsc_deadline_timer: bit(banked.bank(1), processor_feature1_bit::TSC_DEADLINE_TIMER),
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

    /// Each MSR exit sits where the SDK's `WHV_X64_MSR_EXIT_BITMAP` declares
    /// it, in field order.
    ///
    /// Checked one at a time against the header rather than as a round trip:
    /// there is no `from_word` here to make a round trip meaningful, and a
    /// pair of bits swapped between them would trap the wrong register while
    /// still looking like it worked — the guest would simply be told the
    /// host's answer for one MSR and this port's for another.
    #[test]
    fn each_msr_exit_sits_where_the_platform_declares_it() {
        let each = [
            (MsrExits { unhandled: true, ..MsrExits::default() }, 0),
            (MsrExits { tsc_write: true, ..MsrExits::default() }, 1),
            (MsrExits { tsc_read: true, ..MsrExits::default() }, 2),
            (MsrExits { apic_base_write: true, ..MsrExits::default() }, 3),
            (MsrExits { misc_enable_read: true, ..MsrExits::default() }, 4),
            (MsrExits { microcode_revision_read: true, ..MsrExits::default() }, 5),
        ];
        for (exits, bit) in each {
            assert_eq!(exits.as_word(), 1 << bit, "{exits:?} must be bit {bit}");
        }
        assert_eq!(MsrExits::ALL.as_word(), 0b11_1111);
        assert_eq!(MsrExits::default().as_word(), 0);
    }

    #[test]
    fn a_word_survives_a_round_trip_through_the_named_bits() {
        let asked = ExtendedVmExits {
            cpuid: true,
            rdtsc: true,
            apic_write_lint1_trap: true,
            gpa_access_fault: true,
            ..ExtendedVmExits::default()
        };
        assert_eq!(ExtendedVmExits::from_word(asked.as_word()), asked);
    }

    #[test]
    fn the_extended_exits_word_carries_the_apic_traps_where_the_header_puts_them() {
        let asked = ExtendedVmExits {
            apic_write_lint0_trap: true,
            hypercall: true,
            ..ExtendedVmExits::default()
        };
        assert_eq!(asked.as_word(), (1 << 7) | (1 << 5));
        assert_eq!(ExtendedVmExits::from_word(asked.as_word()), asked);
        // LDR and DFR sit above the two exits this port has no use for, so
        // their positions are the ones a reader is most likely to guess wrong.
        // Every `ApicWriteType` variant needs its trap here to be producible.
        let ldr = ExtendedVmExits { apic_write_ldr_trap: true, ..ExtendedVmExits::default() };
        assert_eq!(ldr.as_word(), 1 << 12);
        assert_eq!(ExtendedVmExits::from_word(ldr.as_word()), ldr);
        let dfr = ExtendedVmExits { apic_write_dfr_trap: true, ..ExtendedVmExits::default() };
        assert_eq!(dfr.as_word(), 1 << 13);
        assert_eq!(ExtendedVmExits::from_word(dfr.as_word()), dfr);
    }

    #[test]
    fn the_openvmm_vtl0_synthetic_set_is_every_named_flag_and_nothing_else() {
        // A ratchet, not a header check: the composite is defined as the union of the named
        // flags, so this can only fail when a flag is added without joining the composite.
        assert_eq!(SyntheticFeatures::OPENVMM_VTL0, SyntheticFeatures::all());
        // The check that catches a DROPPED constituent. bitflags 2's `IterNames` yields a
        // defined flag only while it still covers bits no earlier flag has yielded
        // (`src/iter.rs`: "When flags fully overlap, such as in convenience flags that are a
        // shorthand for others, we won't yield both flags"), so the composite defined last is
        // NOT yielded after its 22 constituents.
        assert_eq!(SyntheticFeatures::OPENVMM_VTL0.iter_names().count(), 22);
        // The header's bit positions, spot-checked where the numbering has gaps.
        assert_eq!(SyntheticFeatures::EXTENDED_GVA_RANGES_FOR_FLUSH.bits(), 1 << 15);
        assert_eq!(SyntheticFeatures::FAST_HYPERCALL_OUTPUT.bits(), 1 << 18);
        assert_eq!(SyntheticFeatures::DIRECT_SYNTHETIC_TIMERS.bits(), 1 << 22);
        assert_eq!(SyntheticFeatures::RETARGET_DEVICE_INTERRUPT.bits(), 1 << 30);
    }

    #[test]
    fn a_host_bank_with_a_bit_this_header_does_not_name_is_kept_not_dropped() {
        // A newer host may set a bit our SDK does not define (OpenVMM's ABI copy already has one
        // more). The capability read keeps it so the probe report can show it.
        let word = SyntheticFeatures::HV1.bits() | (1 << 40);
        let bank = SyntheticFeatures::from_bits_retain(word);
        assert!(bank.contains(SyntheticFeatures::HV1));
        assert_eq!(
            bank.bits() & !SyntheticFeatures::all().bits(),
            1 << 40,
            "the unknown bit survives"
        );
        assert_eq!(SyntheticFeatures::from_bits(word), None, "and strict parsing refuses it");
        assert_eq!(SyntheticFeatures::from_bits_truncate(word), SyntheticFeatures::HV1);
    }

    #[test]
    fn asking_for_more_than_the_host_allows_names_the_refused_flags() {
        let allowed = SyntheticFeatures::HYPERVISOR_PRESENT | SyntheticFeatures::HV1;
        let wanted = SyntheticFeatures::OPENVMM_VTL0;
        let refused = wanted.difference(allowed);
        assert!(refused.contains(SyntheticFeatures::ACCESS_SYNTHETIC_TIMER_REGS));
        assert!(!refused.contains(SyntheticFeatures::HV1));
    }

    /// An exit bit this port declares no field for is still recoverable from
    /// [`Capabilities::extended_exits_raw`].
    ///
    /// The decode is lossy by construction — it yields one `bool` per declared
    /// field and nothing for the rest — so the raw word is the only place an
    /// unnamed offer can be read back from. Asserted against both kinds of
    /// unnamed bit: one this port skips on purpose, and one above every
    /// position this SDK header declares.
    #[test]
    fn an_exit_bit_this_port_does_not_name_survives_in_the_raw_word() {
        let word = (1 << exit_bit::CPUID) | (1 << 10) | (1 << 40);
        let named = ExtendedVmExits::from_word(word);
        assert!(named.cpuid);
        assert_eq!(
            named.as_word(),
            1 << exit_bit::CPUID,
            "the decode carries only the positions it declares"
        );
        assert_eq!(
            word & !named.as_word(),
            (1 << 10) | (1 << 40),
            "and the raw word still carries every position the decode dropped"
        );
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
