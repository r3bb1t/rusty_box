//! Completion of a scalar port instruction already checked by the CPU.

use super::MemorySession;
use crate::cpu::instrumentation::{IoHookEvent, MemAccessRW};
use crate::cpu::{CpuError, Instruction, Instrumentation, Result};
use crate::iodev::IoPort;

/// A preflighted scalar IN or OUT. Width is 1, 2, or 4 bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarIoRequest {
    pub port: IoPort,
    pub len: u8,
    /// OUT data, masked to the width. IN carries `None`.
    pub output: Option<u32>,
}

pub(in crate::cpu) struct PendingScalarIo {
    pub(in crate::cpu) next_rip: u64,
    pub(in crate::cpu) request: ScalarIoRequest,
    pub(in crate::cpu) instruction: Instruction,
}

pub(in crate::cpu) fn mask_io_value(value: u32, len: u8) -> u32 {
    match len {
        1 => value & 0xFF,
        2 => value & 0xFFFF,
        _ => value,
    }
}

impl<T: Instrumentation> MemorySession<'_, T> {
    /// Service the outstanding scalar IN or OUT after an `IoUnavailable` exit.
    ///
    /// Return `Some(input)` for IN or `Some(0)` to acknowledge OUT. Return
    /// `None` only if the device has had no effect, to leave the request
    /// pending. Returns true when the instruction completed. An intervening
    /// execution call or successful state edit discards the saved request.
    /// A panic in the host callback also discards it.
    ///
    /// Completion uses the recorded next IP and retires once. Execution
    /// observers receive the normal after-execution callback, and I/O
    /// observers see the completed transfer. String I/O has no continuation.
    pub fn service_scalar_io(
        &mut self,
        service: impl FnOnce(ScalarIoRequest) -> Option<u32>,
    ) -> Result<bool> {
        let Some(pending) = self.ctx.pending_scalar_io.take() else {
            return Err(CpuError::UnsupportedCpuOperation {
                operation: "no outstanding scalar port instruction",
            });
        };
        let Some(response) = service(pending.request) else {
            self.ctx.pending_scalar_io = Some(pending);
            return Ok(false);
        };

        let start = self.ctx.rip();
        self.ctx.set_rip(pending.next_rip);
        let req = pending.request;
        if let Some(value) = req.output {
            if self.ctx.instrumentation.active.has_io() {
                self.ctx.instrumentation.fire_outp(&IoHookEvent {
                    port: req.port,
                    size: req.len,
                    value,
                    access: MemAccessRW::Write,
                });
            }
        } else {
            let value = mask_io_value(response, req.len);
            if self.ctx.instrumentation.active.has_io() {
                self.ctx.instrumentation.fire_inp(req.port, req.len);
                self.ctx.instrumentation.fire_inp2(&IoHookEvent {
                    port: req.port,
                    size: req.len,
                    value,
                    access: MemAccessRW::Read,
                });
            }
            match req.len {
                1 => self.ctx.set_al(value as u8),
                2 => self.ctx.set_ax(value as u16),
                _ => self.ctx.set_rax(u64::from(value)),
            }
        }
        if self.ctx.instrumentation.active.has_exec() {
            self.ctx
                .instrumentation
                .fire_after_execution(start, &pending.instruction);
        }
        self.ctx.prev_rip = pending.next_rip;
        self.ctx.icount += 1;
        #[cfg(feature = "profiling")]
        {
            self.ctx.perf_instructions += 1;
        }
        Ok(true)
    }
}
