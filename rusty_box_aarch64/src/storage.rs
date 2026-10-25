//! Where a machine's memory lives.
//!
//! A machine runs in bytes someone has to hold: guest RAM, the two flash
//! arrays, and — for a kernel the host booted directly — the images a reset
//! loads again. [`GuestStorage`] is that holding, behind one static-dispatch
//! trait (R8), so the same machine runs in memory it owns (`OwnedStorage`,
//! with `alloc`) or in memory a caller lends it ([`LentStorage`], for a
//! `no_std` host that has no allocator but does have buffers).
//!
//! Both are built by [`MachineBuilder`](crate::machine::MachineBuilder), which
//! is the only place their invariants — RAM of the configured size, flash
//! arrays no larger than a bank — are established; neither can be assembled
//! by hand.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

/// The images a directly booted kernel is loaded from, kept so that a reset
/// loads them again the way QEMU's boot loader reset hook does. Both are
/// empty for a machine that boots firmware or nothing.
#[derive(Debug, Clone, Copy)]
pub struct BootImages<'a> {
    /// The arm64 `Image`.
    pub kernel: &'a [u8],
    /// The initial RAM filesystem, or empty.
    pub initrd: &'a [u8],
}

/// Every array of a storage at once, RAM writable beside the images it is
/// loaded from.
pub struct StorageParts<'a> {
    pub ram: &'a mut [u8],
    pub code: &'a [u8],
    pub vars: &'a mut [u8],
    pub kernel: &'a [u8],
    pub initrd: &'a [u8],
}

/// The bytes a machine is built from and runs in.
///
/// Sealed: the builder establishes what a machine assumes of these slices,
/// so only the two kinds it builds exist.
pub trait GuestStorage: sealed::Sealed {
    /// Every array at once.
    fn parts(&mut self) -> StorageParts<'_>;
    /// Guest RAM, mapped at [`RAM_BASE`](crate::board::RAM_BASE).
    fn ram(&self) -> &[u8];
    fn ram_mut(&mut self) -> &mut [u8];
    /// Flash bank 0's array: firmware code, read-only to the guest. Shorter
    /// than the bank is allowed; the rest reads as erased flash.
    fn code(&self) -> &[u8];
    /// Flash bank 1's array: the UEFI variable store, writable by the guest.
    fn vars(&self) -> &[u8];
    fn vars_mut(&mut self) -> &mut [u8];
    /// What a reset of a directly booted kernel loads again.
    fn boot_images(&self) -> BootImages<'_>;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::LentStorage<'_> {}
    #[cfg(feature = "alloc")]
    impl Sealed for super::OwnedStorage {}
}

/// Memory a caller lends a machine: RAM and the variable store are the
/// caller's buffers, and the firmware and kernel images are borrowed where
/// they already are. Nothing is copied but what a boot places in RAM and an
/// initial variable store the builder was given.
pub struct LentStorage<'m> {
    pub(crate) ram: &'m mut [u8],
    pub(crate) code: &'m [u8],
    pub(crate) vars: &'m mut [u8],
    pub(crate) kernel: &'m [u8],
    pub(crate) initrd: &'m [u8],
}

impl GuestStorage for LentStorage<'_> {
    fn parts(&mut self) -> StorageParts<'_> {
        StorageParts {
            ram: self.ram,
            code: self.code,
            vars: self.vars,
            kernel: self.kernel,
            initrd: self.initrd,
        }
    }
    #[inline(always)]
    fn ram(&self) -> &[u8] {
        self.ram
    }
    #[inline(always)]
    fn ram_mut(&mut self) -> &mut [u8] {
        self.ram
    }
    #[inline]
    fn code(&self) -> &[u8] {
        self.code
    }
    #[inline]
    fn vars(&self) -> &[u8] {
        self.vars
    }
    #[inline]
    fn vars_mut(&mut self) -> &mut [u8] {
        self.vars
    }
    fn boot_images(&self) -> BootImages<'_> {
        BootImages {
            kernel: self.kernel,
            initrd: self.initrd,
        }
    }
}

/// Memory the machine owns: every array is its own allocation, so a machine
/// built this way borrows nothing and can be moved to another thread.
#[cfg(feature = "alloc")]
pub struct OwnedStorage {
    pub(crate) ram: Box<[u8]>,
    pub(crate) code: Box<[u8]>,
    pub(crate) vars: Box<[u8]>,
    pub(crate) kernel: Box<[u8]>,
    pub(crate) initrd: Box<[u8]>,
}

#[cfg(feature = "alloc")]
impl GuestStorage for OwnedStorage {
    fn parts(&mut self) -> StorageParts<'_> {
        StorageParts {
            ram: &mut self.ram,
            code: &self.code,
            vars: &mut self.vars,
            kernel: &self.kernel,
            initrd: &self.initrd,
        }
    }
    #[inline(always)]
    fn ram(&self) -> &[u8] {
        &self.ram
    }
    #[inline(always)]
    fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }
    #[inline]
    fn code(&self) -> &[u8] {
        &self.code
    }
    #[inline]
    fn vars(&self) -> &[u8] {
        &self.vars
    }
    #[inline]
    fn vars_mut(&mut self) -> &mut [u8] {
        &mut self.vars
    }
    fn boot_images(&self) -> BootImages<'_> {
        BootImages {
            kernel: &self.kernel,
            initrd: &self.initrd,
        }
    }
}
