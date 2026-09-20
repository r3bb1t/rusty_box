//! Why a machine part is being reset.

/// Reason for a reset — Bochs `BX_RESET_SOFTWARE` / `BX_RESET_HARDWARE`.
///
/// The discriminants are Bochs's own, because they reach the guest: the CMOS
/// shutdown-status byte and the ACPI reset path both record which kind
/// happened, and a soft reset preserves state a hard reset clears.
///
/// Arch-neutral by nature — every part of a machine can be reset, and the
/// distinction between "the guest asked" and "the power did it" is not an x86
/// idea.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum ResetReason {
    Software = 10,
    Hardware = 11,
}
