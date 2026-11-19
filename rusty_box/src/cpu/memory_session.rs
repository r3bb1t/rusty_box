//! CPU-only x86 execution over a caller's physical memory loan.

mod control;
#[cfg(feature = "alloc")]
mod owned;
#[cfg(feature = "alloc")]
pub use owned::{CpuEmulator, CpuEmulatorBuilder};
pub(super) mod scalar_io;
pub use control::{EntryMode, SessionRun, SessionStop};
pub use scalar_io::ScalarIoRequest;

use super::decoder::Opcode;
use super::exec_ctx::ExecCtx;
pub use super::registers::RegisterRefusal;
use super::{BxCpuC, CpuError, Instruction, Instrumentation, Result};
use crate::config::BxPhyAddress;
use crate::memory::execution::ExecutionMemory;
use crate::memory::mapped::MemoryLoan;

/// One physical fragment of a scalar operand. Values are little-endian,
/// with the addressed byte in the low eight bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRequest {
    pub address: BxPhyAddress,
    pub operation: PhysicalOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalOperation {
    Read { len: u8 },
    Write { len: u8, value: u64 },
}

/// The host handles the entire request as one operation. A device that
/// cannot commit the requested width must refuse it without side effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalResponse {
    /// The read or write committed exactly once. The value is used for reads.
    Complete(u64),
    /// No effect committed. The host may prepare the device and retry the step.
    Pending,
    /// No effect committed. The code is supplied by the host.
    Failed(u32),
}

pub(super) struct ExternalScalar {
    page: BxPhyAddress,
    completion: Option<(PhysicalRequest, u64)>,
}

impl ExternalScalar {
    pub(super) fn owns(&self, address: BxPhyAddress) -> bool {
        self.page == address.page_base()
    }

    pub(super) fn access(&mut self, request: PhysicalRequest) -> Result<u64> {
        match self.completion.take() {
            None => Err(CpuError::PhysicalAccessPending(request)),
            Some((expected, value)) if expected == request => Ok(value),
            Some(_) => Err(CpuError::UnsupportedCpuOperation {
                operation: "external scalar replay diverged from its preflight",
            }),
        }
    }
}

pub(super) fn supports_scalar_step(instruction: &Instruction) -> bool {
    match instruction.get_ia_opcode() {
        Opcode::MovAlod
        | Opcode::MovAxod
        | Opcode::MovEaxod
        | Opcode::MovOdAl
        | Opcode::MovOdAx
        | Opcode::MovOdEax => true,
        Opcode::MovGbEb
        | Opcode::MovEbGb
        | Opcode::MovEbIb
        | Opcode::MovGwEw
        | Opcode::MovEwGw
        | Opcode::MovEwIw
        | Opcode::MovOp32GdEd
        | Opcode::MovOp32EdGd
        | Opcode::MovEdId => !instruction.mod_c0(),
        _ => false,
    }
}

/// Short host-side view of the guest's backed physical memory.
pub struct PhysicalMemory<'a> {
    memory: ExecutionMemory<'a>,
    pending: &'a mut Option<scalar_io::PendingScalarIo>,
}

impl PhysicalMemory<'_> {
    /// Read bytes from one backed RAM or ROM window.
    pub fn read_phys(&mut self, address: BxPhyAddress, out: &mut [u8]) -> crate::Result<()> {
        self.memory.read_ram(address, out).map(|_| ())
    }

    /// Write bytes to one writable RAM window and mark any cached code dirty.
    /// Page-table edits still require the usual guest TLB invalidation.
    pub fn write_phys(&mut self, address: BxPhyAddress, data: &[u8]) -> crate::Result<()> {
        self.memory.write_ram(address, data)?;
        *self.pending = None;
        Ok(())
    }
}

/// An exclusive loan of a processor and its physical memory map.
///
/// The caller supplies an initialized CPU and its current register state.
/// Reset and processor placement use the existing CPU APIs. Keep the session
/// across consecutive runs to reuse translations and decoded traces. Ending
/// it releases both borrows. A new session invalidates host mappings and
/// decoded code even if the arena was reused at the same address.
pub struct MemorySession<'a, T: Instrumentation = ()> {
    ctx: ExecCtx<'a, T>,
}

impl<'a, T: Instrumentation> MemorySession<'a, T> {
    pub fn new(cpu: &'a mut BxCpuC<T>, memory: MemoryLoan<'a>) -> Self {
        Self {
            ctx: ExecCtx::borrowed(cpu, memory),
        }
    }

    /// Run for up to `dispatch_budget` instruction dispatches. The return
    /// value is the engine's instruction count delta. REP elements and guest
    /// exception restarts follow the existing Bochs accounting conventions.
    /// HLT, an observer stop or a machine boundary may end a run early.
    /// Unbacked fetch returns `CpuError::VetoedDirectRead` before the
    /// instruction completes. To replace the map, end the session first.
    /// See [`super::CpuError`] for CPU-only exits and retirement semantics.
    pub fn run(&mut self, dispatch_budget: u64) -> Result<u64> {
        self.ctx.pending_scalar_io = None;
        if self.ctx.mmio.has_composed() {
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "a composed PC device requires the software PC",
            });
        }
        self.ctx.cpu_loop_n_slice(dispatch_budget, true, 1)
    }

    /// Borrow the physical RAM and ROM currently attached to this session.
    pub fn memory(&mut self) -> PhysicalMemory<'_> {
        PhysicalMemory {
            memory: self.ctx.memory.reborrow(),
            pending: &mut self.ctx.pending_scalar_io,
        }
    }

    pub fn cpu(&self) -> &BxCpuC<T> {
        &self.ctx
    }

    /// Execute one unpaged integer MOV step with a single external data page.
    ///
    /// `page` must be page-aligned and unbacked in this session. The page is
    /// external only for this call. Instructions and implicit system accesses
    /// are outside this interface. In particular, this step reports a guest
    /// fault before delivering it, and rejects pending CPU events.
    ///
    /// The service sees at most one request, after all operand checks and
    /// before any operand bytes or destination registers have been changed.
    /// On completion the step consumes the response locally and retires once.
    /// On failure or pending service, RIP still names the instruction. A
    /// pending request must be retried with no device effect already committed.
    pub fn step_with_external_page(
        &mut self,
        page: BxPhyAddress,
        service: impl FnOnce(PhysicalRequest) -> PhysicalResponse,
    ) -> Result<u64> {
        // A dense partial page starts with backing too. Checking the page
        // base rules it out, as well as any whole sparse RAM/ROM page.
        if page.page_offset() != 0
            || !matches!(&self.ctx.memory, ExecutionMemory::Borrowed(mem)
                if !mem.has_backing_at(page))
        {
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "external data page must be aligned and unbacked",
            });
        }

        self.ctx.pending_scalar_io = None;
        self.ctx.external_scalar = Some(ExternalScalar {
            page,
            completion: None,
        });
        let result = self.ctx.guarded_scalar_step();
        self.ctx.external_scalar = None;
        let Err(CpuError::PhysicalAccessPending(request)) = result else {
            return result;
        };

        // Release all transient engine state before calling host code. A
        // panicking provider cannot leave the session in guarded mode.
        let value = match service(request) {
            PhysicalResponse::Complete(value) => value,
            PhysicalResponse::Pending => return Err(CpuError::PhysicalAccessPending(request)),
            PhysicalResponse::Failed(code) => {
                return Err(CpuError::PhysicalBusFailure { request, code });
            }
        };
        self.ctx.external_scalar = Some(ExternalScalar {
            page,
            completion: Some((request, value)),
        });
        let result = self.ctx.guarded_scalar_step();
        let state = self.ctx.external_scalar.take();
        if matches!(result, Ok(1)) && state.is_some_and(|state| state.completion.is_none()) {
            return result;
        }
        // This indicates a broken replay invariant. The device might already
        // have committed, so never return a retryable result from this arm.
        Err(CpuError::UnsupportedCpuOperation {
            operation: "external scalar completion did not retire the preflighted instruction",
        })
    }

    /// Update writable physical RAM between runs using the engine's code
    /// stamp tracking. The whole write must fit in one RAM window. Updates
    /// to guest page tables still require guest TLB invalidation.
    pub fn write_ram(&mut self, address: BxPhyAddress, data: &[u8]) -> crate::Result<()> {
        self.memory().write_phys(address, data)
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use crate::cpu::builder::BxCpuBuilder;
    use crate::cpu::cpu::Exception;
    use crate::cpu::{MemPerms, ResetReason};
    use crate::memory::mapped::{MappedMemory, PhysicalRegion, RegionKind};
    use crate::memory::permissions::PagePermissions;

    #[test]
    fn split_store_checks_second_fragment_before_committing_first_device_fragment() {
        let mut cpu = BxCpuBuilder::new().build().unwrap();
        cpu.reset(ResetReason::Hardware);
        let mut permissions = PagePermissions::new(0x5000);
        permissions.set(BxPhyAddress::new(0x4000), 4096, MemPerms::READ);
        cpu.page_permissions = Some(permissions);
        let mut arena = [0; 3 * 4096];
        // Setup AX and execute a word store from the device page into denied RAM.
        arena[..6].copy_from_slice(&[0xB8, 0xCD, 0xAB, 0xA3, 0xFF, 0x3F]);
        arena[4096] = 0x77;
        arena[0x2FF0..0x2FF5].copy_from_slice(&[0xEA, 0, 0x10, 0, 0]);
        let map = [
            PhysicalRegion {
                guest_start: BxPhyAddress::new(0x1000),
                arena_offset: 0,
                len: 4096,
                kind: RegionKind::Ram,
            },
            PhysicalRegion {
                guest_start: BxPhyAddress::new(0x4000),
                arena_offset: 4096,
                len: 4096,
                kind: RegionKind::Ram,
            },
            PhysicalRegion {
                guest_start: BxPhyAddress::new(0xFFFF_F000),
                arena_offset: 8192,
                len: 4096,
                kind: RegionKind::Rom,
            },
        ];
        let mut stamps = [0; 3];
        {
            let mut memory = MappedMemory::new(&mut arena, &mut stamps, &map).unwrap();
            let mut session = MemorySession::new(&mut cpu, memory.loan());
            assert_eq!(session.run(2).unwrap(), 2);
            let ticks = session.cpu().cpu_ticks();
            let result = session.step_with_external_page(BxPhyAddress::new(0x3000), |_| {
                panic!("the second half failed its permission check")
            });
            assert!(matches!(
                result,
                Err(CpuError::GuestFaultBeforeDelivery {
                    vector: Exception::Gp,
                    error_code: 0
                })
            ));
            assert_eq!(session.cpu().rip(), 0x1003);
            assert_eq!(session.cpu().cpu_ticks(), ticks);
        }
        assert_eq!(arena[4096], 0x77);
    }
}
