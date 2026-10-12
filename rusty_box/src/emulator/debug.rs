//! A processor read the way Bochs's debugger reads it: through
//! `BX_CPU_C::dbg_xlate_linear2phy` (cpu/paging.cc) and
//! `BX_MEM_C::dbg_fetch_mem` (memory/misc_mem.cc). A page walk here touches
//! neither the TLB nor an accessed or dirty bit; a read of a device window is
//! that device's read, as it is in Bochs.

use crate::cpu::instrumentation::{CpuDebugView, Fetched, Instrumentation};
use crate::memory::{misc_mem::DbgFetch, mmio_map::MmioHit, CpuMemoryPolicy};
use crate::Result;

use super::{Emulator, SliceEngine, SoftwareEngine};

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
    /// this processor's paging structures, or `None` where it does not.
    pub fn dbg_xlate_linear2phy(&mut self, laddr: u64) -> Option<u64> {
        self.machine
            .exec_ctx(self.index)
            .translate_linear_system_read(laddr)
            .ok()
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
