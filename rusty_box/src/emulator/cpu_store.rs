//! Where a machine keeps its CPUs.
//!
//! One collection, entry 0 the boot processor — Bochs `bx_cpu_array`, whose
//! `BX_CPU(0)` is likewise the CPU that boots. Holding the boot CPU in a field
//! of its own and the rest in a second one forced every accessor to branch on
//! `index == 0`, and forced the whole struct to change shape between builds.
//!
//! The representation is a type, not a feature-gated enum variant (doctrine
//! R0): a build knows its storage at compile time, so an enum would model a
//! runtime choice that never happens.
//!
//! R6 ("Send derives, never promised") is held by the `const` assertions at the
//! foot of this file plus `assert_send::<Emulator<()>>()` in the parent module,
//! NOT by a `Send` supertrait on the trait. A supertrait would read well but
//! would force `T: Send` onto every `impl Emulator<T>` block in the crate,
//! narrowing the machine's API to `Send` tracers for a guarantee the sealed
//! trait plus those assertions already give — nothing outside this module can
//! implement `CpuStore`, and a store that stopped being `Send` fails to
//! compile here.

use super::BxCpuC;
use crate::cpu::instrumentation::Instrumentation;

mod sealed {
    pub trait CpuStoreSealed {}
}

/// The machine's CPUs. Length is fixed at construction: no path changes the CPU
/// count, and a snapshot whose count disagrees is rejected rather than resized.
pub trait CpuStore<T: Instrumentation>: sealed::CpuStoreSealed {
    /// How many CPUs this machine has. Always at least one.
    fn count(&self) -> usize;
    fn get(&self, index: usize) -> &BxCpuC<T>;
    fn get_mut(&mut self, index: usize) -> &mut BxCpuC<T>;
}

/// CPUs this machine allocated and owns.
///
/// A boxed slice of boxed CPUs, never `Box<[BxCpuC<T>]>`: a `BxCpuC` is tens of
/// megabytes (its icache alone is ~19 MiB of fixed arrays), so a flat slice
/// would demand one contiguous allocation of `count × that` and would have to
/// move each CPU by value to build it. Boxing each CPU separately moves only
/// pointers, and keeps every CPU at the stable address its own cached host
/// mappings are measured from.
#[cfg(feature = "alloc")]
pub struct OwnedCpus<T: Instrumentation>(alloc::boxed::Box<[alloc::boxed::Box<BxCpuC<T>>]>);

#[cfg(feature = "alloc")]
impl<T: Instrumentation> OwnedCpus<T> {
    /// Take ownership of already-constructed CPUs, boot processor first.
    pub(crate) fn new(cpus: alloc::vec::Vec<alloc::boxed::Box<BxCpuC<T>>>) -> Self {
        debug_assert!(!cpus.is_empty(), "a machine has at least a boot CPU");
        Self(cpus.into_boxed_slice())
    }
}

#[cfg(feature = "alloc")]
impl<T: Instrumentation> sealed::CpuStoreSealed for OwnedCpus<T> {}

#[cfg(feature = "alloc")]
impl<T: Instrumentation> CpuStore<T> for OwnedCpus<T> {
    #[inline(always)]
    fn count(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    fn get(&self, index: usize) -> &BxCpuC<T> {
        &self.0[index]
    }

    #[inline(always)]
    fn get_mut(&mut self, index: usize) -> &mut BxCpuC<T> {
        &mut self.0[index]
    }
}

/// CPUs owned by the caller and lent to the machine for its whole life.
///
/// A slice of exclusive borrows rather than a slice of CPUs: the CPUs stay
/// wherever the caller put them, so a no-alloc host can page-allocate them
/// separately instead of finding one contiguous run of tens of megabytes per
/// CPU. `&mut` is an exclusive borrow, so this is the opposite of global
/// state — and it is `Send`, which a raw-pointer array could never be.
pub struct BorrowedCpus<'a, T: Instrumentation>(&'a mut [&'a mut BxCpuC<T>]);

impl<'a, T: Instrumentation> BorrowedCpus<'a, T> {
    /// Lend `cpus` to a machine, boot processor first.
    pub fn new(cpus: &'a mut [&'a mut BxCpuC<T>]) -> Self {
        debug_assert!(!cpus.is_empty(), "a machine has at least a boot CPU");
        Self(cpus)
    }
}

impl<T: Instrumentation> sealed::CpuStoreSealed for BorrowedCpus<'_, T> {}

impl<T: Instrumentation> CpuStore<T> for BorrowedCpus<'_, T> {
    #[inline(always)]
    fn count(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    fn get(&self, index: usize) -> &BxCpuC<T> {
        self.0[index]
    }

    #[inline(always)]
    fn get_mut(&mut self, index: usize) -> &mut BxCpuC<T> {
        self.0[index]
    }
}

/// The store this build's machines use. Naming it once here is what lets
/// `Emulator` declare a single un-`cfg`'d `cpus` field: the build varies the
/// type behind the alias, not the shape of the struct.
#[cfg(feature = "alloc")]
pub type MachineCpus<T> = OwnedCpus<T>;
/// See the `alloc` counterpart. A no-alloc machine borrows its CPUs for
/// `'static` because the caller's storage — firmware pages or a static — is
/// never freed while the machine runs.
#[cfg(not(feature = "alloc"))]
pub type MachineCpus<T> = BorrowedCpus<'static, T>;

// R6: neither store may become pointer-shaped. These are what enforce it — a
// raw-pointer field would stop the type deriving `Send` and fail here.
const _: () = {
    const fn assert_send<S: Send>() {}
    assert_send::<BorrowedCpus<'static, ()>>();
    #[cfg(feature = "alloc")]
    assert_send::<OwnedCpus<()>>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::builder::BxCpuBuilder;

    /// The borrowed store is the no-alloc machine's storage, and the no-alloc
    /// machine has no runnable tests — its build is only ever type-checked. So
    /// the store itself is exercised here, in a build that can run, over the
    /// property that matters: index 0 is the boot CPU and each index reaches
    /// its own CPU, with no aliasing between them.
    #[test]
    fn a_borrowed_store_indexes_the_cpus_it_was_lent() {
        let mut first = BxCpuBuilder::new().build().unwrap();
        let mut second = BxCpuBuilder::new().build().unwrap();
        // Distinct retired-instruction counts stand in for "these are
        // different CPUs" — the field is per-CPU state with no shared backing.
        first.icount = 11;
        second.icount = 22;

        let mut handles: [&mut BxCpuC<()>; 2] = [&mut first, &mut second];
        let mut store = BorrowedCpus::new(&mut handles);

        assert_eq!(store.count(), 2);
        assert_eq!(store.get(0).icount, 11, "index 0 is the boot processor");
        assert_eq!(store.get(1).icount, 22);

        // A write through one index must not be visible through the other.
        store.get_mut(1).icount = 99;
        assert_eq!(store.get(0).icount, 11, "the two indexes are not aliased");
        assert_eq!(store.get(1).icount, 99);
    }

    /// The owned store answers the same way, so the machine's accessors behave
    /// identically whichever storage a build selects.
    #[cfg(feature = "alloc")]
    #[test]
    fn an_owned_store_indexes_the_cpus_it_holds() {
        let mut first = BxCpuBuilder::new().build().unwrap();
        let mut second = BxCpuBuilder::new().build().unwrap();
        first.icount = 11;
        second.icount = 22;

        let mut store = OwnedCpus::new(alloc::vec![first, second]);

        assert_eq!(store.count(), 2);
        assert_eq!(store.get(0).icount, 11);
        assert_eq!(store.get(1).icount, 22);

        store.get_mut(1).icount = 99;
        assert_eq!(store.get(0).icount, 11);
        assert_eq!(store.get(1).icount, 99);
    }
}
