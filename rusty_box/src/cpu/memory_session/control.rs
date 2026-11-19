use super::{MemorySession, RegisterRefusal};
use crate::config::BxPhyAddress;
use crate::cpu::api_bridge::FlatSegments;
use crate::cpu::registers::{self, Registers};
use crate::cpu::{
    BreakpointRefusal, CodeBreakpoint, CodeBreakpointHit, CpuError, Instrumentation,
    ProcessorActivity, ResetReason, X86Reg,
};
use crate::params::OnTripleFault;

/// Firmware-free entry layouts for the CPU-only session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryMode {
    /// 16-bit real mode with all segment bases zero.
    Real16,
    /// Unpaged protected mode with flat 32-bit code and data segments.
    /// Requires writable RAM for the 24-byte GDT at physical 0x800.
    Protected32,
    /// The same layout with the GDT in caller-selected writable physical RAM.
    /// The full 24-byte table must fit in the unpaged 32-bit address space.
    Protected32WithGdt(BxPhyAddress),
}

/// Why a controlled CPU-only run returned.
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionStop {
    Budget,
    Observer,
    Breakpoint(CodeBreakpointHit),
    Inactive(ProcessorActivity),
    /// An interpreter error or host boundary. Use the contained error's
    /// retirement and restart semantics before deciding whether to resume.
    Exit(CpuError),
    /// The engine returned early without a more specific stop.
    Yielded,
}

#[derive(Debug)]
pub struct SessionRun {
    /// Engine count delta, including progress before an exit. REP and guest
    /// exception restarts follow the engine's existing accounting.
    pub instructions: u64,
    pub stop: SessionStop,
}

impl<T: Instrumentation> MemorySession<'_, T> {
    /// Reset into a firmware-free layout with the supplied IP and stack
    /// pointer. The low 16 bits are used in real mode. Protected mode
    /// uses the low 32 bits and installs its GDT before resetting the CPU.
    /// A missing GDT backing leaves the CPU and memory untouched.
    pub fn reset_at(&mut self, mode: EntryMode, entry: u32, stack: u32) -> crate::Result<()> {
        let segments = FlatSegments::FLAT_PROTECTED32;
        let gdt = match mode {
            EntryMode::Real16 => None,
            EntryMode::Protected32 => Some(FlatSegments::GDT_BASE),
            EntryMode::Protected32WithGdt(address) => Some(address),
        };
        if let Some(address) = gdt {
            let mut bytes = [0; 24];
            if address.get() > u64::from(u32::MAX) - (bytes.len() as u64 - 1) {
                return Err(CpuError::UnsupportedCpuOperation {
                    operation: "protected-mode GDT must fit in the 32-bit address space",
                }
                .into());
            }
            for (bytes, descriptor) in bytes.chunks_exact_mut(8).zip(segments.gdt()) {
                bytes.copy_from_slice(&descriptor.to_le_bytes());
            }
            self.write_ram(address, &bytes)?;
        }

        self.ctx.instrumentation.stop_request = false;
        self.ctx.instrumentation.stop_honored = false;
        self.ctx.set_on_triple_fault(OnTripleFault::ShutDown);
        self.ctx.reset(ResetReason::Hardware);
        let (entry, stack) = match gdt {
            None => {
                self.ctx.configure_real_segments();
                (entry & 0xFFFF, stack & 0xFFFF)
            }
            Some(address) => {
                self.ctx.configure_flat_segments(segments, address);
                // Last byte of the three descriptors installed by this session.
                self.ctx.set_gdtr_limit_for_api(23);
                self.ctx.enter_protected_mode_for_api();
                (entry, stack)
            }
        };
        self.ctx.set_rsp(u64::from(stack));
        self.set_instruction_pointer(u64::from(entry));
        Ok(())
    }

    /// Run with a dispatch budget and report the stopping boundary.
    pub fn run_controlled(&mut self, dispatch_budget: u64) -> SessionRun {
        if dispatch_budget == 0 {
            return SessionRun {
                instructions: 0,
                stop: SessionStop::Budget,
            };
        }
        let start = self.ctx.icount_for_api();
        self.ctx.instrumentation.stop_honored = false;
        let _ = self.ctx.code_breakpoints.take_hit();

        let result = self.run(dispatch_budget);
        let instructions = self.ctx.icount_for_api().wrapping_sub(start);
        let observer = core::mem::take(&mut self.ctx.instrumentation.stop_honored)
            | core::mem::take(&mut self.ctx.instrumentation.stop_request);
        let breakpoint = self.ctx.code_breakpoints.take_hit();
        let activity = self.ctx.processor_activity();
        let stop = if let Err(error) = result {
            SessionStop::Exit(error)
        } else if let Some(hit) = breakpoint {
            SessionStop::Breakpoint(CodeBreakpointHit {
                cpu: 0,
                breakpoint: hit.breakpoint,
                linear: hit.linear,
            })
        } else if observer {
            SessionStop::Observer
        } else if activity != ProcessorActivity::Active {
            SessionStop::Inactive(activity)
        } else if instructions >= dispatch_budget {
            SessionStop::Budget
        } else {
            SessionStop::Yielded
        };
        SessionRun { instructions, stop }
    }

    /// Run with a one-dispatch budget, including guest exception delivery.
    ///
    /// A breakpoint, halt or host exit can stop before an instruction
    /// retires. REP may execute several elements in one dispatch. A guest
    /// fault can consume the budget and leave IP at its handler. The result
    /// reports engine counts rather than a precise retirement count.
    pub fn step(&mut self) -> SessionRun {
        self.run_controlled(1)
    }

    /// Replace the engine's fixed-capacity code breakpoint set. Breakpoints
    /// remain armed across runs, with one resume allowed at a reported hit.
    pub fn set_code_breakpoints(
        &mut self,
        breakpoints: &[CodeBreakpoint],
    ) -> Result<(), BreakpointRefusal> {
        self.ctx.set_code_breakpoints(breakpoints)?;
        self.ctx.pending_scalar_io = None;
        Ok(())
    }

    /// Set the instruction offset in the current CS. The next fetch checks
    /// it according to the guest mode. This preserves the activity state.
    pub fn set_instruction_pointer(&mut self, address: u64) {
        self.ctx.pending_scalar_io = None;
        registers::set_ip(&mut self.ctx, address);
    }

    /// Read backed guest physical bytes from one RAM or ROM window.
    pub fn read_physical(&mut self, address: BxPhyAddress, out: &mut [u8]) -> crate::Result<()> {
        self.memory().read_phys(address, out)
    }

    /// Discard cached guest translation state after editing page tables.
    pub fn invalidate_translations(&mut self) {
        self.ctx.pending_scalar_io = None;
        self.ctx.tlb_flush();
    }

    pub fn regs(&mut self) -> Registers<'_, T> {
        self.ctx.registers()
    }

    pub fn read_register(&self, register: X86Reg) -> Result<u64, RegisterRefusal> {
        registers::read_dynamic(&self.ctx, register)
    }

    /// Compatibility scalar register write, including a raw CR3 edit.
    /// The caller supplies a valid raw CR3 value for the current mode.
    pub fn write_register(&mut self, register: X86Reg, value: u64) -> Result<(), RegisterRefusal> {
        if register == X86Reg::Cr3 {
            self.ctx.set_cr3_raw_for_api(value);
            self.ctx.pending_scalar_io = None;
            Ok(())
        } else {
            self.regs().write_dynamic(register, value)
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::config::BxLinAddress;
    use crate::cpu::builder::BxCpuBuilder;
    use crate::memory::flat::FlatMemory;

    #[test]
    fn gpr_edit_preserves_warm_data_and_fetch_mappings() {
        let mut cpu = BxCpuBuilder::new().build().unwrap();
        let mut bytes = [0; 5 * 4096];
        let mut stamps = [0; 5];
        let mut memory = FlatMemory::new(&mut bytes, &mut stamps).unwrap();
        let mut session = MemorySession::new(&mut cpu, memory.loan());
        session
            .reset_at(EntryMode::Protected32, 0x1000, 0x4FFC)
            .unwrap();

        // Identity-map the code and data pages through ordinary RAM.
        for (address, entry) in [(0x2000, 0x3003u32), (0x3004, 0x1003), (0x3010, 0x4003)] {
            session
                .write_ram(BxPhyAddress::new(address), &entry.to_le_bytes())
                .unwrap();
        }
        // mov eax,[0x4000]
        session
            .write_ram(BxPhyAddress::new(0x1000), &[0xA1, 0, 0x40, 0, 0])
            .unwrap();
        session.write_register(X86Reg::Cr3, 0x2000).unwrap();
        let cr0 = session.ctx.get_cr0_val();
        session.ctx.set_cr0_raw_for_api(cr0 | (1 << 31));
        assert!(matches!(session.step().stop, SessionStop::Budget));

        let linear = BxLinAddress::new(0x4000);
        let direct_page = session.ctx.dtlb.entry_of(linear, 0).host_page;
        assert!(direct_page.is_some());
        assert!(session.ctx.eip_fetch_window.is_some());
        session.regs().write(crate::cpu::registers::EAX, 7);
        assert_eq!(session.ctx.dtlb.entry_of(linear, 0).host_page, direct_page);
        assert!(session.ctx.eip_fetch_window.is_some());

        session.regs().write(crate::cpu::registers::EIP, 0x1000);
        assert!(session.ctx.eip_fetch_window.is_none());
        assert_eq!(session.ctx.dtlb.entry_of(linear, 0).host_page, direct_page);
    }
}
