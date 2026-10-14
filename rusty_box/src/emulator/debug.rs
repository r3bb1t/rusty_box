//! A processor read the way Bochs's debugger reads it: through
//! `BX_CPU_C::dbg_xlate_linear2phy` (cpu/paging.cc) and
//! `BX_MEM_C::dbg_fetch_mem` (memory/misc_mem.cc). A page walk here touches
//! neither the TLB nor an accessed or dirty bit; a read of a device window is
//! that device's read, as it is in Bochs.

use crate::cpu::instrumentation::{
    CpuDebugView, Fetched, Instrumentation, LinearRange, MappingWalk, PageMapping, WalkStep,
};
use crate::memory::{misc_mem::DbgFetch, mmio_map::MmioHit, CpuMemoryPolicy};
use crate::Result;

use super::{Emulator, SliceEngine, SoftwareEngine};

/// A byte's place in guest RAM's storage order, which is not a physical
/// address (R4): below the PCI hole the two agree, and the RAM above the
/// hole is stored straight after the RAM below it
/// (memory_rusty_box.rs `bx_translate_gpa_to_linear`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RamOffset(pub u64);

impl RamOffset {
    /// The guest-physical address this RAM byte sits at.
    pub fn physical(self) -> u64 {
        crate::memory::memory_rusty_box::bx_translate_linear_to_gpa(self.0)
    }

    /// The RAM byte at guest-physical `physical`, or, for an address in the
    /// PCI hole, the first RAM byte above it — so a walk through RAM by
    /// physical address can start anywhere and never reads a device window.
    pub fn at_or_after_physical(physical: u64) -> Self {
        use crate::memory::memory_rusty_box::{
            bx_is_pci_hole_addr, bx_translate_gpa_to_linear, BX_PCI_HOLE_END,
        };
        if bx_is_pci_hole_addr(physical) {
            Self(bx_translate_gpa_to_linear(BX_PCI_HOLE_END))
        } else {
            Self(bx_translate_gpa_to_linear(physical))
        }
    }

    /// The RAM byte at guest-physical `physical`, or `None` in the PCI hole.
    pub fn of_physical(physical: u64) -> Option<Self> {
        use crate::memory::memory_rusty_box::{bx_is_pci_hole_addr, bx_translate_gpa_to_linear};
        (!bx_is_pci_hole_addr(physical)).then(|| Self(bx_translate_gpa_to_linear(physical)))
    }
}

/// One processor of a machine, chosen by index, for a debugger to read.
pub struct DebugCpu<'m, T: Instrumentation, E = SoftwareEngine> {
    machine: &'m mut Emulator<T, E>,
    index: usize,
}

impl<T: Instrumentation, E: SliceEngine<T>> Emulator<T, E> {
    /// Processor `index` (0 is the boot processor), or `None` past the last.
    pub fn debug_cpu(&mut self, index: usize) -> Option<DebugCpu<'_, T, E>> {
        (index < self.cpu_count()).then_some(DebugCpu {
            machine: self,
            index,
        })
    }
}

impl<T: Instrumentation, E: SliceEngine<T>> DebugCpu<'_, T, E> {
    /// The processor's registers and what they mean for its code and stack.
    pub fn view(&self) -> CpuDebugView {
        self.machine.cpu_ref(self.index).debug_view_for_api()
    }

    /// Bochs paging.cc `dbg_xlate_linear2phy`: where `laddr` maps through
    /// this processor's paging structures (and EPT or the SVM nested tables
    /// when it runs as a guest under them), or `None` where it does not.
    pub fn dbg_xlate_linear2phy(&mut self, laddr: u64) -> Option<u64> {
        self.machine
            .exec_ctx(self.index)
            .dbg_xlate_linear2phy(laddr, crate::cpu::paging::DbgWalk::Guest)
    }

    /// [`Self::dbg_xlate_linear2phy`] through another address space: the
    /// paging structures rooted at the CR3 value `cr3`, read under this
    /// processor's CR4 and EFER. That is how a debugger follows a guest OS
    /// into a process other than the one this processor is running.
    pub fn dbg_xlate_linear2phy_in(&mut self, cr3: u64, laddr: u64) -> Option<u64> {
        self.machine
            .exec_ctx(self.index)
            .dbg_xlate_linear2phy(laddr, crate::cpu::paging::DbgWalk::Root(cr3))
    }

    /// Every page the paging structures rooted at the CR3 value `cr3` map
    /// within `range`, in address order, handed to `visit` until it asks to
    /// stop: a page's linear and physical address, its size, and the rights
    /// all of its levels together grant. The tables are read as
    /// [`Self::dbg_xlate_linear2phy_in`] reads them, so the two agree.
    pub fn dbg_page_mappings_in<F>(&mut self, cr3: u64, range: LinearRange, mut visit: F) -> MappingWalk
    where
        F: FnMut(PageMapping) -> WalkStep,
    {
        self.machine
            .exec_ctx(self.index)
            .dbg_page_mappings(cr3, range, &mut visit)
    }

    /// Bochs misc_mem.cc `dbg_fetch_mem`: `buf.len()` guest-physical bytes
    /// from `addr`, as this processor sees them (its SMM state decides
    /// whether SMRAM is visible).
    ///
    /// # Errors
    /// A RAM block that could not be brought back into residency.
    pub fn dbg_fetch_mem(&mut self, addr: u64, buf: &mut [u8]) -> Result<Fetched> {
        let policy = CpuMemoryPolicy::new(self.machine.cpu_ref(self.index).smm_mode(), false);
        match self.machine.memory.dbg_fetch_mem(policy, addr, buf)? {
            DbgFetch::Read(fetched) => Ok(fetched),
            DbgFetch::Device(hit) => {
                let now_ticks = self.machine.pc_system.time_ticks();
                let Emulator {
                    devices,
                    pc_system,
                    device_manager,
                    ..
                } = &mut *self.machine;
                // A window with no device behind it has no handler to call
                // (Bochs finds no memory handler there): it reads as nothing.
                if !devices.mmio_claimed(hit, device_manager) {
                    buf.fill(0xff);
                    return Ok(Fetched::OutsideMemory);
                }
                let mut fetched = Fetched::Valid;
                for (offset, byte) in (0u64..).zip(buf.iter_mut()) {
                    // Bochs hands the handler one byte at a time, at the
                    // address it has reached.
                    let at = MmioHit {
                        token: hit.token,
                        offset: hit.offset.wrapping_add(offset),
                    };
                    let mut one = [0xff_u8];
                    if !devices.mmio_read(at, 1, &mut one, now_ticks, pc_system, device_manager) {
                        fetched = Fetched::OutsideMemory;
                    }
                    *byte = one[0];
                }
                Ok(fetched)
            }
        }
    }
}
