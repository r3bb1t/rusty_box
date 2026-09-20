//! The PCI configuration-space role, as a device sees it.
//!
//! The bridge, the routing table and the config-address latch are the
//! machine's; what lives here is only what a device model must implement to be
//! reachable through them.

/// Encode a PCI device/function number: `(device << 3) | function`.
/// Bochs: `BX_PCI_DEVICE(device, function)` macro (pci.h).
pub const fn pci_device(device: u8, function: u8) -> u8 {
    (device << 3) | (function & 7)
}

/// A device that answers configuration space at one fixed bus-0 devfunc.
///
/// The devfunc travels with the device rather than with the dispatch arm, so
/// the routing table cannot send device 1 function 3's config cycles to
/// somebody else's registers.
///
/// Bochs registers each model's handler pair with `DEV_register_pci_handlers`
/// (pci.cc), which is where the devfunc and the handlers are married upstream.
pub trait PciDevice {
    /// `(device << 3) | function` on bus 0 — see [`pci_device`].
    ///
    /// A constant is correct because this port ships exactly one chipset.
    /// Upstream it is runtime data twice over: pci2isa.cc, pci_ide.cc and
    /// acpi.cc each pick device 7 under `BX_PCI_CHIPSET_I440BX` and device 1
    /// otherwise, and vga.cc passes 0 so `register_pci_handlers` auto-assigns
    /// the first free slot. Both reduce to fixed values here — i440FX gives
    /// the device-1 branch, and VGA, the only slot-taking device, takes slot 1,
    /// which `slot_to_dev[0]` maps to device 2. Porting i440BX or a second
    /// slot-taking device turns this back into runtime state.
    const DEVFUNC: u8;

    /// What a config write asks the machine to do that the device cannot do
    /// for itself: re-register a relocated BAR, re-derive a chipset mapping.
    /// `()` would mean a device whose registers affect nothing outside it —
    /// no PC device is currently that self-contained.
    type WriteEffects;

    fn pci_read(&self, address: u8, io_len: u8) -> u32;

    /// Dropping the returned effects strands a BAR at its old address. The
    /// `#[must_use]` that enforces that sits on `pci_config_write` in
    /// `devices.rs` — the one path every guest config write takes — rather
    /// than here, where it would only decorate register-semantics tests that
    /// assert on the device's own state instead.
    fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> Self::WriteEffects;
}

