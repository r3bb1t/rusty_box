//! The machine: a processor on the `virt` board, and the run loop that
//! advances them together.
//!
//! The run loop is where determinism is kept:
//!
//! - interrupts are taken only at instruction boundaries, from line levels
//!   that change only at instructions (an MMIO access, a timer register
//!   write) or at a precomputed tick (`next_event`);
//! - waiting for an interrupt jumps time straight to the next event, and the
//!   processor stays in the waiting state across calls, so a run that is cut
//!   into budgets behaves exactly like one that is not;
//! - nothing reads the host clock.

use rusty_box_core::{ClockHz, VmInstant};

use crate::board::{VirtBoard, FLASH_BANK_SIZE};
use crate::cpu::{Activity, Cpu, CpuConfig, DebugStops, Step, StopCause, UndefinedInsn};
use crate::devices::cfi_flash::{CfiFlash, FlashStorage, SECTOR_LEN};
use crate::devices::pl011::TX_CAPACITY;

/// How the machine is built.
#[derive(Debug, Clone, Copy)]
pub struct VirtConfig {
    /// Emulated instructions per second.
    pub ips: ClockHz,
    /// The generic timer's system counter frequency.
    pub counter_hz: ClockHz,
    /// What the RTC reads at emulated time zero, in seconds since the Unix
    /// epoch.
    pub rtc_epoch_seconds: u64,
    /// Faults that stop the machine instead of reaching the guest.
    pub debug_stops: DebugStops,
}

impl VirtConfig {
    /// 2026-01-01T00:00:00Z.
    pub const DEFAULT_RTC_EPOCH: u64 = 1_767_225_600;

    pub const DEFAULT: Self = Self {
        ips: ClockHz::BOCHS_DEFAULT,
        counter_hz: CpuConfig::QEMU_VIRT_COUNTER_HZ,
        rtc_epoch_seconds: Self::DEFAULT_RTC_EPOCH,
        debug_stops: DebugStops {
            on_undefined: false,
            on_bus_fault: false,
        },
    };
}

impl Default for VirtConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The memory a caller lends the machine. Nothing is copied except the
/// device tree, which is placed at the base of RAM where QEMU's `virt` puts
/// it for firmware to find.
pub struct VirtMemory<'m> {
    /// Guest RAM at 0x4000_0000. Its size must match the device tree's
    /// `memory` node.
    pub ram: &'m mut [u8],
    /// Flash bank 0 (firmware code), mapped read-only.
    pub flash0: &'m [u8],
    /// Flash bank 1 (the UEFI variable store), writable.
    pub flash1: &'m mut [u8],
    /// The flattened device tree describing this board.
    pub dtb: &'m [u8],
}

/// Why a machine could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtBuildError {
    /// The device tree does not fit in RAM.
    DtbLargerThanRam,
    /// A flash image is larger than its 64 MiB bank.
    FlashTooLarge,
    /// A flash image is not a whole number of 256 KiB sectors.
    FlashNotWholeSectors,
}

/// What bounds one call of [`VirtMachine::run`]: a number of ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBudget {
    ticks: u64,
}

impl RunBudget {
    pub const fn ticks(ticks: u64) -> Self {
        Self { ticks }
    }
}

/// Why [`VirtMachine::run`] returned.
///
/// Closed (R5): each variant names one way a run ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The budget was spent; running again continues.
    BudgetExhausted,
    /// The UART's transmit buffer is nearly full; drain it and run again.
    UartOutputPending,
    /// The processor waits for an interrupt and nothing is scheduled to
    /// raise one.
    Halted,
    /// PSCI `SYSTEM_OFF`.
    PoweredOff,
    /// PSCI `SYSTEM_RESET`.
    ResetRequested,
    /// PSCI `CPU_OFF` on the only processor.
    CpuOff,
    /// A defined instruction this interpreter does not implement.
    Unimplemented(UndefinedInsn),
    /// An UNDEFINED instruction, with `DebugStops::on_undefined` set.
    Undefined(UndefinedInsn),
    /// An access to nothing, with `DebugStops::on_bus_fault` set.
    BusFault { pc: u64, addr: u64 },
}

impl From<StopCause> for StopReason {
    fn from(c: StopCause) -> Self {
        match c {
            StopCause::Unimplemented(i) => Self::Unimplemented(i),
            StopCause::Undefined(i) => Self::Undefined(i),
            StopCause::BusFault { pc, addr } => Self::BusFault { pc, addr },
            StopCause::PowerOff => Self::PoweredOff,
            StopCause::Reset => Self::ResetRequested,
            StopCause::CpuOff => Self::CpuOff,
        }
    }
}

/// Leave this much room in the UART buffer: the run stops early rather than
/// let the oldest output be overwritten.
const TX_HIGH_WATER: usize = TX_CAPACITY - 4096;

/// A Cortex-A57-shaped processor on QEMU's `virt` board.
pub struct VirtMachine<'m> {
    cpu: Cpu,
    board: VirtBoard<'m>,
    /// The next tick at which a timer or device output changes by itself.
    next_event: u64,
}

impl<'m> VirtMachine<'m> {
    /// A machine at its reset vector, with the device tree placed in RAM.
    pub fn new(config: VirtConfig, memory: VirtMemory<'m>) -> Result<Self, VirtBuildError> {
        let VirtMemory { ram, flash0, flash1, dtb } = memory;
        for image_len in [flash0.len() as u64, flash1.len() as u64] {
            if image_len > FLASH_BANK_SIZE {
                return Err(VirtBuildError::FlashTooLarge);
            }
            if image_len % SECTOR_LEN != 0 {
                return Err(VirtBuildError::FlashNotWholeSectors);
            }
        }
        match ram.get_mut(..dtb.len()) {
            Some(dst) => dst.copy_from_slice(dtb),
            None => return Err(VirtBuildError::DtbLargerThanRam),
        }
        let mut cpu = Cpu::new(CpuConfig {
            ips: config.ips,
            counter_hz: config.counter_hz,
            reset_pc: crate::board::FLASH0_BASE,
        });
        cpu.set_debug_stops(config.debug_stops);
        let board = VirtBoard::new(
            ram,
            CfiFlash::new(FlashStorage::ReadOnly(flash0)),
            CfiFlash::new(FlashStorage::Writable(flash1)),
            config.rtc_epoch_seconds,
        );
        let mut machine = Self {
            cpu,
            board,
            next_event: 0,
        };
        machine.sync_events();
        Ok(machine)
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn board(&self) -> &VirtBoard<'m> {
        &self.board
    }

    /// Emulated time since reset.
    pub fn now(&self) -> VmInstant {
        self.cpu.now()
    }

    /// Bytes the guest wrote to the UART, oldest first.
    pub fn drain_uart(&mut self) -> rusty_box_core::ring_buffer::Drain<'_, u8, TX_CAPACITY> {
        self.board.uart.drain_tx()
    }

    /// Send a byte to the guest's UART. Returns false if its receive FIFO is
    /// full.
    pub fn send_uart(&mut self, byte: u8) -> bool {
        let accepted = self.board.uart.push_rx(byte);
        self.board.sync_device_lines();
        self.cpu.set_interrupt_lines(self.board.gic.irq_out(), self.board.gic.fiq_out());
        accepted
    }

    /// Re-evaluate everything that changes with time or with a timer
    /// register: the timer PPIs, the time-driven devices, the processor's
    /// interrupt inputs, and the next tick at which any of them changes.
    fn sync_events(&mut self) {
        self.cpu.take_timer_dirty();
        let lines = self.cpu.timer_lines();
        self.board.set_timer_lines(lines.physical, lines.virtual_);
        let device_deadline = self.board.advance_devices(self.cpu.clock()).map(|t| t.ticks());
        let timer_deadline = self.cpu.next_timer_deadline();
        self.next_event = match (timer_deadline, device_deadline) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => u64::MAX,
        };
        self.cpu.set_interrupt_lines(self.board.gic.irq_out(), self.board.gic.fiq_out());
    }

    /// Run until `budget` ticks have passed or something stops the machine.
    pub fn run(&mut self, budget: RunBudget) -> StopReason {
        let end = self.cpu.icount.saturating_add(budget.ticks);
        loop {
            if self.cpu.icount >= self.next_event {
                self.sync_events();
            }
            if self.cpu.activity == Activity::WaitingForInterrupt && !self.cpu.wake_if_pending() {
                if self.next_event == u64::MAX {
                    return StopReason::Halted;
                }
                let until = self.next_event.min(end);
                self.cpu.wait_until(VmInstant::from_ticks(until));
                if until == end {
                    return StopReason::BudgetExhausted;
                }
                continue;
            }
            if self.cpu.icount >= end {
                return StopReason::BudgetExhausted;
            }
            if self.cpu.interrupt_deliverable() {
                self.cpu.deliver_interrupt();
            }
            match self.cpu.step(&mut self.board) {
                Step::Retired | Step::Waiting => {}
                Step::Stop(cause) => return cause.into(),
            }
            if self.cpu.timer_dirty {
                self.sync_events();
            } else {
                self.cpu.set_interrupt_lines(self.board.gic.irq_out(), self.board.gic.fiq_out());
            }
            if self.board.uart.tx_len() >= TX_HIGH_WATER {
                return StopReason::UartOutputPending;
            }
        }
    }
}
