//! A processor read the way Bochs's debugger reads it: through
//! `BX_CPU_C::dbg_xlate_linear2phy` (cpu/paging.cc) and
//! `BX_MEM_C::dbg_fetch_mem` (memory/misc_mem.cc). A page walk here touches
//! neither the TLB nor an accessed or dirty bit; a read of a device window is
//! that device's read, as it is in Bochs.

use crate::cpu::instrumentation::{
    CpuDebugView, Fetched, Instrumentation, LinearRange, MappingWalk, PageMapping, VmxGuest,
    WalkStep,
};
use crate::cpu::paging::{DbgWalk, VmxGuestWalk};
#[cfg(doc)]
use crate::cpu::{MAX_LINEAR_BREAKPOINTS, MAX_PHYSICAL_BREAKPOINTS};
use crate::cpu::{BreakpointRefusal, BreakpointSupport, CodeBreakpoint, CodeBreakpointHit};
use crate::memory::{misc_mem::DbgFetch, mmio_map::MmioHit, CpuMemoryPolicy};
use crate::Result;

use super::{Emulator, SliceEngine, SoftwareEngine, StopCause};

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

    /// Replaces the debugger's code breakpoints on every processor: Bochs's
    /// `lbreak` and `pbreak`, up to [`MAX_LINEAR_BREAKPOINTS`] and
    /// [`MAX_PHYSICAL_BREAKPOINTS`]. A processor that reaches one stops before
    /// its instruction executes; the run reports
    /// [`StopReason::Breakpoint`](super::StopReason::Breakpoint) and raises the
    /// stop flag, and the run after the flag is lowered executes that
    /// instruction. Nothing is written to guest memory and no debug register
    /// changes, so the guest cannot tell. An empty slice clears them.
    ///
    /// # Errors
    /// [`BreakpointRefusal`] when a kind has more breakpoints than slots, or
    /// when breakpoints are asked of an engine that does not run the guest
    /// through the interpreter ([`SliceEngine::CODE_BREAKPOINTS`]). A refused
    /// set leaves the one before it in place.
    pub fn set_code_breakpoints(
        &mut self,
        breakpoints: &[CodeBreakpoint],
    ) -> core::result::Result<(), BreakpointRefusal> {
        match E::CODE_BREAKPOINTS {
            BreakpointSupport::Honoured => {}
            BreakpointSupport::Refused if breakpoints.is_empty() => {}
            BreakpointSupport::Refused => return Err(BreakpointRefusal::EngineCannotStop),
        }
        for index in 0..self.cpu_count() {
            self.cpu_mut_at(index).set_code_breakpoints(breakpoints)?;
        }
        Ok(())
    }

    /// The code breakpoint the machine stopped at, while the stop flag it
    /// raised still stands, for a driver that reads no stop reason
    /// (`run_interactive` returns a count). A hit is told once — here or as a
    /// run's [`StopReason::Breakpoint`](super::StopReason::Breakpoint),
    /// whichever asks first — and the flag then stands as a plain stop.
    pub fn stopped_at_breakpoint(&mut self) -> Option<CodeBreakpointHit> {
        let cause = self.stop_in_force()?;
        self.breakpoint_reported(cause);
        match cause {
            StopCause::Breakpoint(hit) => Some(hit),
            StopCause::HostRequested | StopCause::GuestPowerOff | StopCause::EngineFault => None,
        }
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
            .dbg_xlate_linear2phy(laddr, DbgWalk::Guest)
    }

    /// [`Self::dbg_xlate_linear2phy`] through another address space: the
    /// paging structures rooted at the CR3 value `cr3`, read under this
    /// processor's CR4 and EFER. That is how a debugger follows a guest OS
    /// into a process other than the one this processor is running.
    pub fn dbg_xlate_linear2phy_in(&mut self, cr3: u64, laddr: u64) -> Option<u64> {
        self.machine
            .exec_ctx(self.index)
            .dbg_xlate_linear2phy(laddr, DbgWalk::Root(cr3))
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

    /// The guest the current VMCS holds, while this processor is in VMX root
    /// operation — a VMM paused between two runs of its guest — and that VMCS
    /// has been launched: the guest's state as the last VM exit from it wrote
    /// it into the VMCS. `None` when the processor is not in VMX root
    /// operation or has no launched VMCS; inside a guest, [`Self::view`] is
    /// already the guest's. Bochs's debugger has no such view: it reads the
    /// processor's own registers only.
    pub fn vmx_guest(&mut self) -> Option<VmxGuest> {
        self.machine.exec_ctx(self.index).vmx_guest()
    }

    /// [`Self::dbg_xlate_linear2phy_in`] inside the VMX guest `guest`: the
    /// paging structures rooted at the CR3 value `cr3`, read under the
    /// guest's CR0, CR4 and EFER, every structure address and the result
    /// through the guest's EPT ([`VmxGuest::memory`]), so the address
    /// returned is the machine's — what [`Self::dbg_fetch_mem`] reads.
    pub fn dbg_xlate_vmx_guest_linear2phy(&mut self, guest: &VmxGuest, cr3: u64, laddr: u64) -> Option<u64> {
        let walk = DbgWalk::VmxGuest(VmxGuestWalk::of(guest, cr3));
        self.machine.exec_ctx(self.index).dbg_xlate_linear2phy(laddr, walk)
    }

    /// [`Self::dbg_page_mappings_in`] inside the VMX guest `guest`, read as
    /// [`Self::dbg_xlate_vmx_guest_linear2phy`] reads it: each mapping's
    /// physical address is the machine's.
    pub fn dbg_vmx_guest_page_mappings<F>(
        &mut self,
        guest: &VmxGuest,
        cr3: u64,
        range: LinearRange,
        mut visit: F,
    ) -> MappingWalk
    where
        F: FnMut(PageMapping) -> WalkStep,
    {
        self.machine
            .exec_ctx(self.index)
            .dbg_vmx_guest_page_mappings(VmxGuestWalk::of(guest, cr3), range, &mut visit)
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
