//! 8259A PIC (Programmable Interrupt Controller) Emulation
//!
//! Ported from Bochs `iodev/pic.cc` — behavior matches Bochs exactly.
//!
//! # Overview
//!
//! The 8259 PIC handles hardware interrupt routing in the IBM PC architecture.
//! A PC has two 8259 PIC chips in a cascaded configuration:
//! - **Master PIC**: ports 0x20-0x21, handles IRQ 0-7
//! - **Slave PIC**: ports 0xA0-0xA1, handles IRQ 8-15
//!
//! The slave's output (INT pin) is connected to the master's IRQ2 input,
//! giving 15 usable hardware IRQ lines (IRQ2 is consumed by the cascade).
//!
//! # Standard PC IRQ Assignments
//!
//! ```text
//! IRQ 0:  PIT Timer (System Timer, ~18.2 Hz or programmed rate)
//! IRQ 1:  Keyboard Controller (8042)
//! IRQ 2:  CASCADE from Slave PIC (not available to devices)
//! IRQ 3:  COM2 / COM4 (Serial Port)
//! IRQ 4:  COM1 / COM3 (Serial Port)
//! IRQ 5:  LPT2 or Sound Card
//! IRQ 6:  Floppy Disk Controller
//! IRQ 7:  LPT1 (Parallel Port) / Spurious
//! IRQ 8:  RTC (Real-Time Clock)
//! IRQ 9:  ACPI / Available
//! IRQ 10: Available
//! IRQ 11: Available
//! IRQ 12: PS/2 Mouse (8042 AUX port)
//! IRQ 13: FPU / Coprocessor
//! IRQ 14: Primary ATA (IDE)
//! IRQ 15: Secondary ATA (IDE)
//! ```
//!
//! # Initialization Sequence (ICW1-ICW4)
//!
//! The PIC is programmed through a strict 4-byte initialization sequence.
//! Writing a byte with bit 4 set to the command port triggers ICW1:
//!
//! ```text
//! ICW1 (command port, bit 4 = 1):
//!   Bit 0: IC4 — 1=ICW4 will be sent, 0=no ICW4
//!   Bit 1: SNGL — 1=single mode, 0=cascade mode (always 0 in PC)
//!   Bit 3: LTIM — 1=level triggered, 0=edge triggered (always 0 in PC)
//!   Effect: Resets ISR, IRR, IMR; clears auto-EOI; sets lowest_priority=7
//!
//! ICW2 (data port, after ICW1):
//!   Bits 7-3: Base interrupt vector (top 5 bits)
//!   Example: 0x08 for master (IRQ0 = INT 8), 0x70 for slave (IRQ8 = INT 0x70)
//!
//! ICW3 (data port, after ICW2):
//!   Master: bitmask of slave IRQ lines (0x04 = slave on IRQ2)
//!   Slave: slave ID number (0x02 = connected to master IRQ2)
//!
//! ICW4 (data port, after ICW3, only if IC4=1 in ICW1):
//!   Bit 0: uPM — must be 1 for 8086 mode
//!   Bit 1: AEOI — 1=auto EOI, 0=manual EOI (normal)
//!   Bit 2: BUF — buffered mode (not used)
//!   Bit 3: M/S — master/slave in buffered mode
//!   Bit 4: SFNM — specially fully nested mode
//! ```
//!
//! # Operation Command Words (OCW1-OCW3)
//!
//! After initialization, the PIC accepts operation commands:
//!
//! ```text
//! OCW1 (data port): Write IMR (Interrupt Mask Register)
//!   Each bit masks the corresponding IRQ line (1=masked, 0=enabled)
//!
//! OCW2 (command port, bits 4:3 = 00):
//!   0x20: Non-specific EOI — clears highest priority in-service bit
//!   0x60-0x67: Specific EOI for IRQ 0-7
//!   0xA0: Rotate on non-specific EOI
//!   0xE0-0xE7: Specific EOI + set priority rotation
//!   0xC0-0xC7: Set lowest priority (priority rotation)
//!   0x00/0x80: Clear/Set rotate-on-auto-EOI mode
//!
//! OCW3 (command port, bits 4:3 = 01):
//!   Bit 0-1: Read register select (0x02=IRR, 0x03=ISR)
//!   Bit 2: Poll command
//!   Bit 5-6: Special mask mode (0x40=clear, 0x60=set)
//! ```
//!
//! # Interrupt Priority and Service
//!
//! Priority is determined by `lowest_priority` (default 7), making IRQ0
//! the highest priority and IRQ7 the lowest. The `pic_service()` function
//! scans from highest priority through all IRQ lines, looking for unmasked
//! requests that are not blocked by an in-service interrupt of higher priority.
//!
//! When a request is found:
//! - Master: asserts INT to CPU (INTR pin)
//! - Slave: asserts cascade to master (triggers master IRQ2)
//!
//! # Interrupt Acknowledge (INTA) Cycle
//!
//! When the CPU acknowledges an interrupt (`iac()`):
//! 1. INT is deasserted
//! 2. Spurious check: if no unmasked requests, returns spurious vector (offset+7)
//! 3. For edge-triggered IRQs: clears IRR bit. For level-triggered: keeps it
//! 4. In manual EOI mode: sets ISR bit (host must send EOI to clear it)
//! 5. In auto-EOI mode: does not set ISR bit
//! 6. For cascade (IRQ2): delegates to slave PIC for the actual vector
//! 7. Returns interrupt vector number (interrupt_offset + irq)
//!
//! # Edge/Level Triggered Mode (ELCR)
//!
//! The Edge/Level Control Registers at ports 0x4D0-0x4D1 set per-IRQ
//! trigger mode. Edge-triggered IRQs must be deasserted before re-asserting
//! to create a new edge. Level-triggered IRQs remain asserted in IRR until
//! the device deasserts the line. IRQ0-2 and IRQ8,13 are always edge-triggered.

use core::ffi::c_void;

/// PIC I/O port addresses
pub const PIC_MASTER_CMD: u16 = 0x0020;
pub const PIC_MASTER_DATA: u16 = 0x0021;
pub const PIC_SLAVE_CMD: u16 = 0x00A0;
pub const PIC_SLAVE_DATA: u16 = 0x00A1;

/// Edge/Level Control Register ports (ELCR)
pub const PIC_ELCR1: u16 = 0x04D0;
pub const PIC_ELCR2: u16 = 0x04D1;

/// Action returned by `Pic8259State::service()`.
///
/// Separates the PIC's internal state changes from external side effects,
/// avoiding the need to clone state or pass raw pointers (as Bochs does).
/// The Rust borrow checker prevents passing `&mut Pic8259State` to a method
/// that also needs `&mut self` — this enum bridges the gap cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PicServiceAction {
    /// No external action needed
    None,
    /// Master: raise INTR to CPU (Bochs `BX_RAISE_INTR`)
    RaiseIntr,
    /// Master: clear INTR from CPU (Bochs `BX_CLEAR_INTR`)
    ClearIntr,
    /// Slave: assert cascade on master IRQ2
    RaiseCascade,
    /// Slave: deassert cascade on master IRQ2
    LowerCascade,
}

/// State for a single 8259 PIC chip (Bochs `bx_pic_t`)
#[derive(Debug, Clone)]
pub struct Pic8259State {
    /// Is this the master PIC?
    pub(crate) master: bool,
    /// Programmable interrupt vector offset (ICW2, top 5 bits)
    pub(crate) interrupt_offset: u8,
    /// Specially fully nested mode (ICW4 bit 4)
    pub(crate) sfnm: bool,
    /// Buffered mode (ICW4 bit 2)
    pub(crate) buffered_mode: bool,
    /// Master/slave in buffered mode (ICW4 bit 3)
    pub(crate) master_slave: bool,
    /// Auto EOI mode — true=automatic, false=manual (ICW4 bit 1)
    pub(crate) auto_eoi: bool,
    /// Interrupt Mask Register (1=masked)
    pub(crate) imr: u8,
    /// In-Service Register (bit set when IRQ is being serviced)
    pub(crate) isr: u8,
    /// Interrupt Request Register (bit set on raise_irq)
    pub(crate) irr: u8,
    /// Read register select: false=IRR, true=ISR (OCW3 read_op)
    pub(crate) read_reg_select: bool,
    /// Current winning IRQ number (0-7), set by service()
    pub(crate) irq: u8,
    /// Current lowest priority IRQ (for rotating priority, default=7)
    pub(crate) lowest_priority: u8,
    /// INT output pin — true = asserting interrupt to CPU/cascade
    pub(crate) int_pin: bool,
    /// Per-line IRQ assertion flag.
    /// Bochs uses a bitmask of `irq_type` flags (ISA, PCI) to support
    /// shared IRQ lines. We simplify to 0/1 since we don't have PCI IRQ sharing.
    pub(crate) irq_in: [u8; 8],
    /// Initialization sequence state
    pub(crate) init: PicInitState,
    /// Special mask mode (OCW3)
    pub(crate) special_mask: bool,
    /// Poll command issued (OCW3 bit 2)
    pub(crate) polled: bool,
    /// Rotate on auto-EOI (OCW2 0x80/0x00)
    pub(crate) rotate_on_autoeoi: bool,
    /// Edge/level trigger mode bitmap (0=edge, 1=level per IRQ line)
    pub(crate) edge_level: u8,
}

/// PIC initialization sequence state (Bochs `bx_pic_t::init`)
#[derive(Debug, Clone, Default)]
pub struct PicInitState {
    /// Currently in initialization sequence
    pub(crate) in_init: bool,
    /// ICW4 required (ICW1 bit 0)
    pub(crate) requires_4: bool,
    /// Which ICW byte is expected next (2, 3, or 4)
    pub(crate) byte_expected: u8,
}

impl Default for Pic8259State {
    fn default() -> Self {
        Self {
            master: false,
            interrupt_offset: 0,
            sfnm: false,
            buffered_mode: false,
            master_slave: false,
            auto_eoi: false,
            imr: 0xFF, // All IRQs masked initially
            isr: 0,
            irr: 0,
            read_reg_select: false,
            irq: 0,
            lowest_priority: 7,
            int_pin: false,
            irq_in: [0; 8],
            init: PicInitState::default(),
            special_mask: false,
            polled: false,
            rotate_on_autoeoi: false,
            edge_level: 0,
        }
    }
}

impl Pic8259State {
    /// Clear the highest priority in-service interrupt (Bochs `clear_highest_interrupt`).
    ///
    /// Scans ISR from highest priority (lowest_priority + 1) wrapping around,
    /// clears the first set bit found.
    fn clear_highest_interrupt(&mut self) {
        let mut highest_priority = self.lowest_priority + 1;
        if highest_priority > 7 {
            highest_priority = 0;
        }

        let mut irq = highest_priority;
        loop {
            if (self.isr & (1 << irq)) != 0 {
                self.isr &= !(1 << irq);
                break;
            }
            irq += 1;
            if irq > 7 {
                irq = 0;
            }
            if irq == highest_priority {
                break;
            }
        }
    }

    /// Service the PIC — find highest priority pending interrupt (Bochs `pic_service`).
    ///
    /// Returns an action indicating what external side effect is needed.
    /// The PIC's internal state (int_pin, irq) is updated before returning.
    ///
    /// Algorithm from Bochs `pic.cc:550-612`:
    /// 1. Compute `max_irq` — the boundary beyond which ISR blocks preemption
    /// 2. Scan from highest_priority to max_irq for unmasked, un-in-service requests
    /// 3. If found and INT not already asserted, assert INT and signal
    /// 4. If no requests and INT was asserted, deassert INT
    fn service(&mut self) -> PicServiceAction {
        let mut highest_priority = self.lowest_priority + 1;
        if highest_priority > 7 {
            highest_priority = 0;
        }

        let isr = self.isr;
        let mut max_irq = highest_priority;

        if self.special_mask {
            // Special mask mode: all priorities may be enabled.
            // Check all IRR bits except ones with corresponding ISR bits set.
            // max_irq stays at highest_priority (full scan).
        } else {
            // Normal mode: find the highest priority IRQ blocked by an in-service IRQ.
            if isr != 0 {
                while (isr & (1 << max_irq)) == 0 {
                    max_irq += 1;
                    if max_irq > 7 {
                        max_irq = 0;
                    }
                }
                // Highest priority interrupt is already in-service — no preemption possible.
                if max_irq == highest_priority {
                    return PicServiceAction::None;
                }
            }
        }

        let unmasked_requests = self.irr & !self.imr;
        if unmasked_requests != 0 {
            let mut irq = highest_priority;
            loop {
                // In special mask mode, skip IRQs already in-service
                if !(self.special_mask && ((isr >> irq) & 0x01) != 0) {
                    // Only signal if INT not already asserted (prevents double-assertion)
                    if !self.int_pin && (unmasked_requests & (1 << irq)) != 0 {
                        self.int_pin = true;
                        self.irq = irq;
                        return if self.master {
                            PicServiceAction::RaiseIntr
                        } else {
                            PicServiceAction::RaiseCascade
                        };
                    }
                }
                irq += 1;
                if irq > 7 {
                    irq = 0;
                }
                if irq == max_irq {
                    break;
                }
            }
        } else if self.int_pin {
            // No unmasked requests — deassert INT
            self.int_pin = false;
            return if self.master {
                PicServiceAction::ClearIntr
            } else {
                PicServiceAction::LowerCascade
            };
        }

        PicServiceAction::None
    }
}

/// Dual 8259 PIC Controller — Master + Slave (Bochs `bx_pic_c`)
#[derive(Debug)]
pub struct BxPicC {
    /// Master PIC state (ports 0x20-0x21)
    pub(crate) master: Pic8259State,
    /// Slave PIC state (ports 0xA0-0xA1)
    pub(crate) slave: Pic8259State,
    /// Edge/Level Control Registers (ELCR)
    pub(crate) elcr: [u8; 2],
    /// Raw pointer to CPU's `async_event` for BX_RAISE_INTR / BX_CLEAR_INTR signaling.
    /// When master int_pin asserts, we write 1 here so the CPU breaks out of the
    /// inner trace loop at the next instruction boundary (matching Bochs BX_RAISE_INTR).
    cpu_async_event_ptr: *mut u32,
    /// Raw pointer to CPU's `pending_event` for event-bit management.
    cpu_pending_event_ptr: *mut u32,
}

impl Default for BxPicC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPicC {
    /// Create a new PIC controller (Bochs `bx_pic_c::init`)
    pub fn new() -> Self {
        let mut master = Pic8259State::default();
        master.master = true;
        master.interrupt_offset = 0x08; // IRQ0 = INT 0x08
        master.master_slave = true;

        let mut slave = Pic8259State::default();
        slave.master = false;
        slave.interrupt_offset = 0x70; // IRQ8 = INT 0x70
        slave.master_slave = false;

        Self {
            master,
            slave,
            elcr: [0, 0],
            cpu_async_event_ptr: core::ptr::null_mut(),
            cpu_pending_event_ptr: core::ptr::null_mut(),
        }
    }

    /// Wire CPU signal pointers for BX_RAISE_INTR / BX_CLEAR_INTR.
    ///
    /// Must be called before CPU execution begins. The pointers must remain
    /// valid for the lifetime of the emulator.
    ///
    /// # Safety
    /// The caller must ensure the pointers remain valid and that the PIC
    /// is only accessed from one thread at a time.
    pub unsafe fn set_cpu_signal_ptrs(&mut self, async_event: *mut u32, pending_event: *mut u32) {
        self.cpu_async_event_ptr = async_event;
        self.cpu_pending_event_ptr = pending_event;
    }

    /// BX_RAISE_INTR — signal CPU that an external interrupt is pending.
    ///
    /// Matches Bochs `BX_RAISE_INTR()` macro which calls
    /// `BX_CPU(0)->signal_event(BX_EVENT_PENDING_INTR)`.
    #[inline]
    fn raise_intr(&self) {
        if !self.cpu_async_event_ptr.is_null() {
            unsafe {
                // pending_event |= (1 << BX_EVENT_PENDING_INTR)
                // BX_EVENT_PENDING_INTR = 0, so bit 0
                *self.cpu_pending_event_ptr |= 1;
                *self.cpu_async_event_ptr = 1;
            }
        }
    }

    /// BX_CLEAR_INTR — clear the external interrupt pending signal.
    ///
    /// Matches Bochs `BX_CLEAR_INTR()` macro which calls
    /// `BX_CPU(0)->clear_event(BX_EVENT_PENDING_INTR)`.
    #[inline]
    fn clear_intr(&self) {
        if !self.cpu_pending_event_ptr.is_null() {
            unsafe {
                *self.cpu_pending_event_ptr &= !1;
            }
        }
    }

    /// Initialize the PIC (called during device init)
    pub fn init(&mut self) {
        tracing::info!("PIC: Initializing 8259 Programmable Interrupt Controller");
        self.reset();
    }

    /// Reset the PIC to initial state
    pub fn reset(&mut self) {
        self.master.imr = 0xFF;
        self.master.isr = 0;
        self.master.irr = 0;
        self.master.int_pin = false;
        self.master.init = PicInitState::default();
        self.master.irq_in = [0; 8];

        self.slave.imr = 0xFF;
        self.slave.isr = 0;
        self.slave.irr = 0;
        self.slave.int_pin = false;
        self.slave.init = PicInitState::default();
        self.slave.irq_in = [0; 8];

        self.elcr = [0, 0];
    }

    /// Dispatch `Pic8259State::service()` result, handling cascade side effects.
    ///
    /// For master: signal CPU via BX_RAISE_INTR / BX_CLEAR_INTR (Bochs pic.cc).
    /// For slave: cascade via `raise_irq(2)` / `lower_irq(2)` on the master.
    fn service_pic_dispatch(&mut self, is_master: bool) {
        if is_master {
            let action = self.master.service();
            match action {
                PicServiceAction::RaiseIntr => self.raise_intr(),
                PicServiceAction::ClearIntr => self.clear_intr(),
                _ => {}
            }
        } else {
            let action = self.slave.service();
            match action {
                PicServiceAction::RaiseCascade => {
                    self.raise_irq(2);
                }
                PicServiceAction::LowerCascade => {
                    self.lower_irq(2);
                }
                _ => {}
            }
        }
    }

    // ---- Read handlers ----

    /// Read from PIC I/O port (Bochs `bx_pic_c::read`)
    ///
    /// Takes `&mut self` because poll mode read triggers an interrupt acknowledge
    /// which modifies PIC state (Bochs pic.cc:205-219).
    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        // Poll mode: read triggers interrupt acknowledge (Bochs pic.cc:205-219)
        if (port == PIC_MASTER_CMD || port == PIC_MASTER_DATA) && self.master.polled {
            self.master.clear_highest_interrupt();
            self.master.polled = false;
            self.service_pic_dispatch(true);
            return self.master.irq as u32;
        }
        if (port == PIC_SLAVE_CMD || port == PIC_SLAVE_DATA) && self.slave.polled {
            self.slave.clear_highest_interrupt();
            self.slave.polled = false;
            self.service_pic_dispatch(false);
            return self.slave.irq as u32;
        }

        match port {
            PIC_MASTER_CMD => {
                if self.master.read_reg_select {
                    self.master.isr as u32
                } else {
                    self.master.irr as u32
                }
            }
            PIC_MASTER_DATA => self.master.imr as u32,
            PIC_SLAVE_CMD => {
                if self.slave.read_reg_select {
                    self.slave.isr as u32
                } else {
                    self.slave.irr as u32
                }
            }
            PIC_SLAVE_DATA => self.slave.imr as u32,
            PIC_ELCR1 => self.elcr[0] as u32,
            PIC_ELCR2 => self.elcr[1] as u32,
            _ => {
                tracing::warn!("PIC: Unknown read port {:#06x}", port);
                0xFF
            }
        }
    }

    // ---- Write handlers ----

    /// Write to PIC I/O port (Bochs `bx_pic_c::write`)
    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let value = value as u8;
        match port {
            PIC_MASTER_CMD => self.write_cmd(value, true),
            PIC_MASTER_DATA => self.write_data(value, true),
            PIC_SLAVE_CMD => self.write_cmd(value, false),
            PIC_SLAVE_DATA => self.write_data(value, false),
            PIC_ELCR1 => {
                self.elcr[0] = value & 0xF8; // IRQ0-2 are edge-triggered only
                                             // Sync ELCR to master PIC edge_level (Bochs pic.cc set_mode)
                self.master.edge_level = self.elcr[0];
                tracing::debug!("PIC: ELCR1 = {:#04x}", value);
            }
            PIC_ELCR2 => {
                self.elcr[1] = value & 0xDE; // IRQ8,13 are edge-triggered only
                                             // Sync ELCR to slave PIC edge_level (Bochs pic.cc set_mode)
                self.slave.edge_level = self.elcr[1];
                tracing::debug!("PIC: ELCR2 = {:#04x}", value);
            }
            _ => {
                tracing::warn!("PIC: Unknown write port {:#06x} value={:#04x}", port, value);
            }
        }
    }

    /// Handle command port write — ICW1, OCW2, or OCW3 (Bochs pic.cc:277-400).
    ///
    /// The command port multiplexes three different operations based on bit patterns:
    /// - **Bit 4 = 1**: ICW1 — starts a new initialization sequence
    /// - **Bits 4:3 = 00**: OCW2 — EOI commands and priority rotation
    /// - **Bits 4:3 = 01**: OCW3 — register read select, poll mode, special mask
    ///
    /// ## OCW2 Encoding (bits 7:5 select the operation)
    /// ```text
    /// 001: Non-specific EOI (clear highest priority ISR bit)
    /// 011: Specific EOI (clear ISR bit for IRQ in bits 2:0)
    /// 101: Rotate on non-specific EOI (EOI + rotate lowest_priority)
    /// 100: Rotate in auto-EOI mode SET
    /// 000: Rotate in auto-EOI mode CLEAR
    /// 111: Rotate on specific EOI (specific EOI + set lowest_priority)
    /// 110: Set priority command (set lowest_priority to bits 2:0)
    /// 010: (invalid/NOP)
    /// ```
    fn write_cmd(&mut self, value: u8, is_master: bool) {
        if (value & 0x10) != 0 {
            // ICW1 — Initialization Command Word 1 (Bochs pic.cc:278-306)
            eprintln!("[PIC-CMD] ICW1={:#04x} {}", value, if is_master { "master" } else { "slave" });
            tracing::debug!(
                "PIC: ICW1 = {:#04x} ({})",
                value,
                if is_master { "master" } else { "slave" }
            );
            {
                let pic = if is_master {
                    &mut self.master
                } else {
                    &mut self.slave
                };
                pic.init.in_init = true;
                pic.init.requires_4 = (value & 0x01) != 0;
                pic.init.byte_expected = 2;
                pic.imr = 0x00; // Clear IRQ mask
                pic.isr = 0x00; // No IRQs in service
                pic.irr = 0x00; // No IRQs requested
                pic.lowest_priority = 7;
                pic.auto_eoi = false;
                pic.rotate_on_autoeoi = false;
                pic.int_pin = false; // Reprogramming clears previous INTR
            }
            // Deassert: slave clears cascade line on master (Bochs pic.cc:304)
            if !is_master {
                self.master.irq_in[2] = 0;
            }
        } else if (value & 0x18) == 0x08 {
            // OCW3 — Operation Command Word 3 (Bochs pic.cc:309-329)
            let special_mask = (value & 0x60) >> 5;
            let poll = (value & 0x04) >> 2;
            let read_op = value & 0x03;

            // Poll command: set polled flag and return early (Bochs pic.cc:315-318)
            if poll != 0 {
                let pic = if is_master {
                    &mut self.master
                } else {
                    &mut self.slave
                };
                pic.polled = true;
                return;
            }

            let mut needs_service = false;
            {
                let pic = if is_master {
                    &mut self.master
                } else {
                    &mut self.slave
                };
                if read_op == 0x02 {
                    pic.read_reg_select = false; // Read IRR
                } else if read_op == 0x03 {
                    pic.read_reg_select = true; // Read ISR
                }
                if special_mask == 0x02 {
                    pic.special_mask = false; // Cancel special mask
                } else if special_mask == 0x03 {
                    pic.special_mask = true; // Set special mask
                    needs_service = true; // Bochs calls pic_service after enabling SMM
                }
            }
            if needs_service {
                self.service_pic_dispatch(is_master);
            }
        } else {
            // OCW2 — EOI and priority commands (Bochs pic.cc:333-400)
            self.write_ocw2(value, is_master);
        }
    }

    /// Handle OCW2 commands — EOI and priority rotation (Bochs pic.cc:333-400)
    ///
    /// Uses full-value matching like Bochs, not bit-field extraction.
    fn write_ocw2(&mut self, value: u8, is_master: bool) {
        match value {
            // Rotate in auto-EOI mode: clear (0x00) or set (0x80)
            0x00 | 0x80 => {
                let pic = if is_master {
                    &mut self.master
                } else {
                    &mut self.slave
                };
                pic.rotate_on_autoeoi = value != 0;
            }

            // Non-specific EOI (0x20) or Rotate on non-specific EOI (0xA0)
            // Bochs pic.cc:339-350
            0x20 | 0xA0 => {
                {
                    let pic = if is_master {
                        &mut self.master
                    } else {
                        &mut self.slave
                    };
                    pic.clear_highest_interrupt();
                    if value == 0xA0 {
                        // Rotate: increment lowest_priority (wraps at 7)
                        pic.lowest_priority += 1;
                        if pic.lowest_priority > 7 {
                            pic.lowest_priority = 0;
                        }
                    }
                }
                self.service_pic_dispatch(is_master);
            }

            // No-op: Intel spec (0x40) and 386BSD compatibility (0x02)
            0x40 | 0x02 => {}

            // Specific EOI for IRQ 0-7 (Bochs pic.cc:356-366)
            0x60..=0x67 => {
                {
                    let pic = if is_master {
                        &mut self.master
                    } else {
                        &mut self.slave
                    };
                    pic.isr &= !(1 << (value - 0x60));
                }
                self.service_pic_dispatch(is_master);
            }

            // Set lowest priority (IRQ priority rotation) (Bochs pic.cc:369-379)
            0xC0..=0xC7 => {
                let pic = if is_master {
                    &mut self.master
                } else {
                    &mut self.slave
                };
                pic.lowest_priority = value - 0xC0;
            }

            // Specific EOI + rotate priority (Bochs pic.cc:381-392)
            0xE0..=0xE7 => {
                {
                    let pic = if is_master {
                        &mut self.master
                    } else {
                        &mut self.slave
                    };
                    pic.isr &= !(1 << (value - 0xE0));
                    pic.lowest_priority = value - 0xE0;
                }
                self.service_pic_dispatch(is_master);
            }

            _ => {
                tracing::warn!("PIC: unexpected OCW2 value: {:#04x}", value);
            }
        }
    }

    /// Handle data port write — ICW sequence or IMR (Bochs pic.cc:401-458).
    ///
    /// The data port is context-sensitive:
    /// - **During initialization** (`init.in_init == true`): receives ICW2, ICW3,
    ///   and ICW4 in sequence. The `byte_expected` field tracks which ICW is next.
    /// - **After initialization**: writes set the IMR (Interrupt Mask Register).
    ///   Each bit masks the corresponding IRQ line: 1=masked (disabled), 0=enabled.
    ///   After writing the IMR, `pic_service()` is called to check if any previously
    ///   masked IRQ can now be serviced.
    fn write_data(&mut self, value: u8, is_master: bool) {
        let in_init = if is_master {
            self.master.init.in_init
        } else {
            self.slave.init.in_init
        };

        if in_init {
            self.write_icw(value, is_master);
        } else {
            // OCW1 — Set IMR, then re-service for any unmasked pending IRQs
            {
                let pic = if is_master {
                    &mut self.master
                } else {
                    &mut self.slave
                };
                static IMR_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
                let ic = IMR_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                if ic < 20 {
                    eprintln!("[PIC-IMR#{}] {} IMR={:#04x}", ic,
                        if is_master { "master" } else { "slave" }, value);
                }
                pic.imr = value;
            }
            self.service_pic_dispatch(is_master);
        }
    }

    /// Handle ICW2/3/4 during initialization sequence (Bochs pic.cc:403-451)
    fn write_icw(&mut self, value: u8, is_master: bool) {
        let pic = if is_master {
            &mut self.master
        } else {
            &mut self.slave
        };

        match pic.init.byte_expected {
            2 => {
                // ICW2 — Interrupt vector offset (top 5 bits)
                pic.interrupt_offset = value & 0xF8;
                eprintln!("[PIC-CMD] ICW2={:#04x} offset={:#04x} {}",
                    value, pic.interrupt_offset,
                    if is_master { "master" } else { "slave" });
                tracing::debug!(
                    "PIC: ICW2 = {:#04x} (offset = {:#04x})",
                    value,
                    pic.interrupt_offset
                );
                pic.init.byte_expected = 3;
            }
            3 => {
                // ICW3 — Cascade configuration
                tracing::debug!("PIC: ICW3 = {:#04x}", value);
                if pic.init.requires_4 {
                    pic.init.byte_expected = 4;
                } else {
                    pic.init.in_init = false;
                }
            }
            4 => {
                // ICW4 — Mode configuration
                tracing::debug!("PIC: ICW4 = {:#04x}", value);
                pic.auto_eoi = (value & 0x02) != 0;
                pic.buffered_mode = (value & 0x04) != 0;
                pic.master_slave = (value & 0x08) != 0;
                pic.sfnm = (value & 0x10) != 0;
                pic.init.in_init = false;
            }
            _ => {}
        }
    }

    // ---- IRQ management ----

    /// Raise an IRQ line (Bochs `bx_pic_c::raise_irq`)
    ///
    /// Sets the IRQ request register bit and calls `pic_service` if the IRQ
    /// was not already asserted. For edge-triggered IRQs, the device must
    /// call `lower_irq` first to create a new edge.
    ///
    /// Bochs uses `IRQ_in[n]` as a bitmask of `irq_type` flags (ISA, PCI)
    /// to support shared IRQ lines. We simplify to 0/1 since we don't have
    /// PCI IRQ sharing. The logic is equivalent for single-type IRQs.
    pub fn raise_irq(&mut self, irq_no: u8) {
        if irq_no < 8 {
            // Master PIC — Bochs pic.cc:491-496
            // Bochs guard: `(IRQ_in[n] & ~irq_type) == 0` allows re-assertion of same type.
            // For our single-type model, always set irq_in and check IRR.
            self.master.irq_in[irq_no as usize] = 1;
            if (self.master.irr & (1 << irq_no)) == 0 {
                self.master.irr |= 1 << irq_no;
                self.service_pic_dispatch(true);
            }
        } else if irq_no < 16 {
            // Slave PIC — same logic
            let slave_irq = irq_no - 8;
            self.slave.irq_in[slave_irq as usize] = 1;
            if (self.slave.irr & (1 << slave_irq)) == 0 {
                self.slave.irr |= 1 << slave_irq;
                self.service_pic_dispatch(false);
            }
        }
    }

    /// Lower an IRQ line (Bochs `bx_pic_c::lower_irq`)
    ///
    /// Clears the IRQ assertion flag and the IRR bit.
    /// Bochs clears IRR unconditionally when all types are deasserted.
    pub fn lower_irq(&mut self, irq_no: u8) {
        if irq_no < 8 {
            if self.master.irq_in[irq_no as usize] != 0 {
                self.master.irq_in[irq_no as usize] = 0;
                self.master.irr &= !(1 << irq_no);
            }
        } else if irq_no < 16 {
            let slave_irq = irq_no - 8;
            if self.slave.irq_in[slave_irq as usize] != 0 {
                self.slave.irq_in[slave_irq as usize] = 0;
                self.slave.irr &= !(1 << slave_irq);
            }
        }
    }

    /// Set IRQ level — raise or lower based on level (Bochs `bx_pic_c::set_irq_level`)
    ///
    /// Convenience wrapper used by devices that track IRQ state as a bool.
    pub fn set_irq_level(&mut self, irq_no: u8, level: bool) {
        if level {
            self.raise_irq(irq_no);
        } else {
            self.lower_irq(irq_no);
        }
    }

    /// Set edge/level trigger mode for a PIC (Bochs `bx_pic_c::set_mode`)
    ///
    /// Called by the PCI-to-ISA bridge when ELCR registers are written.
    /// `is_master`: true = master PIC, false = slave PIC
    /// `mode`: bitmap where each bit represents an IRQ line (0=edge, 1=level)
    pub fn set_mode(&mut self, is_master: bool, mode: u8) {
        if is_master {
            self.master.edge_level = mode;
        } else {
            self.slave.edge_level = mode;
        }
    }

    /// Check if an interrupt is pending (master INT pin asserted)
    pub fn has_interrupt(&self) -> bool {
        self.master.int_pin
    }

    /// Interrupt Acknowledge — CPU INTA cycle (Bochs `bx_pic_c::IAC`)
    ///
    /// Returns the interrupt vector number. Handles:
    /// - Spurious interrupt detection (returns offset+7 if no unmasked requests)
    /// - Edge vs level-triggered IRR clearing
    /// - Auto-EOI with optional priority rotation
    /// - Slave cascade via IRQ2
    /// - Re-service after acknowledge
    pub fn iac(&mut self) -> u8 {
        // Bochs pic.cc:620-621: BX_CLEAR_INTR(); master_pic.INT = 0;
        self.clear_intr(); // Signal CPU to clear pending interrupt event
        self.master.int_pin = false;

        // Spurious interrupt check: if no unmasked requests, return spurious vector
        // (Bochs pic.cc:623-625)
        if (self.master.irr & !self.master.imr) == 0 {
            return self.master.interrupt_offset + 7;
        }

        // Edge-triggered: clear IRR bit. Level-triggered: keep it.
        // (Bochs pic.cc:627-628) — Bochs does NOT clear irq_in here.
        if (self.master.edge_level & (1 << self.master.irq)) == 0 {
            self.master.irr &= !(1 << self.master.irq);
        }

        // Auto-EOI: don't set ISR. Manual EOI: set ISR bit.
        // (Bochs pic.cc:630-633)
        if !self.master.auto_eoi {
            self.master.isr |= 1 << self.master.irq;
        } else if self.master.rotate_on_autoeoi {
            self.master.lowest_priority = self.master.irq;
        }

        let vector;

        if self.master.irq != 2 {
            // Direct master IRQ (0, 1, 3-7)
            vector = self.master.irq + self.master.interrupt_offset;
        } else {
            // IRQ2 = slave cascade (IRQ8-15)
            // (Bochs pic.cc:638-657)
            self.slave.int_pin = false;
            // Bochs pic.cc:640: IRQ_in[2] &= ~BX_IRQ_TYPE_ISA
            // Clear cascade assertion (single-type model: set to 0)
            self.master.irq_in[2] = 0;

            // Slave spurious interrupt check (Bochs pic.cc:642-644)
            if (self.slave.irr & !self.slave.imr) == 0 {
                return self.slave.interrupt_offset + 7;
            }

            vector = self.slave.irq + self.slave.interrupt_offset;

            // Edge-triggered: clear slave IRR bit. Level: keep it.
            // (Bochs pic.cc:648-649) — Bochs does NOT clear irq_in here.
            if (self.slave.edge_level & (1 << self.slave.irq)) == 0 {
                self.slave.irr &= !(1 << self.slave.irq);
            }

            // Slave auto-EOI handling
            if !self.slave.auto_eoi {
                self.slave.isr |= 1 << self.slave.irq;
            } else if self.slave.rotate_on_autoeoi {
                self.slave.lowest_priority = self.slave.irq;
            }

            // Re-service slave after acknowledge (Bochs pic.cc:655)
            self.service_pic_dispatch(false);
        }

        // Re-service master after acknowledge (Bochs pic.cc:659)
        self.service_pic_dispatch(true);

        vector
    }
}

/// PIC read handler for I/O port infrastructure
pub fn pic_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let pic = unsafe { &mut *(this_ptr as *mut BxPicC) };
    pic.read(port, io_len)
}

/// PIC write handler for I/O port infrastructure
pub fn pic_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let pic = unsafe { &mut *(this_ptr as *mut BxPicC) };
    pic.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pic_creation() {
        let pic = BxPicC::new();
        assert!(pic.master.master);
        assert!(!pic.slave.master);
        assert_eq!(pic.master.interrupt_offset, 0x08);
        assert_eq!(pic.slave.interrupt_offset, 0x70);
    }

    #[test]
    fn test_pic_imr() {
        let mut pic = BxPicC::new();
        pic.write(PIC_MASTER_DATA, 0x00, 1);
        assert_eq!(pic.master.imr, 0x00);
        pic.write(PIC_MASTER_DATA, 0xFF, 1);
        assert_eq!(pic.master.imr, 0xFF);
    }

    #[test]
    fn test_pic_irq() {
        let mut pic = BxPicC::new();
        pic.master.imr = 0x00; // Unmask all
        pic.raise_irq(0);
        assert!(pic.has_interrupt());
        let vector = pic.iac();
        assert_eq!(vector, 0x08); // IRQ0 -> INT 0x08
    }

    #[test]
    fn test_pic_spurious() {
        let mut pic = BxPicC::new();
        pic.master.int_pin = true; // Fake an interrupt assertion
        pic.master.imr = 0xFF; // But all masked — no unmasked requests
        let vector = pic.iac();
        assert_eq!(vector, 0x08 + 7); // Spurious vector = offset + 7
    }

    #[test]
    fn test_pic_specific_eoi() {
        let mut pic = BxPicC::new();
        pic.master.imr = 0x00;
        pic.raise_irq(3);
        assert!(pic.has_interrupt());
        let vector = pic.iac();
        assert_eq!(vector, 0x08 + 3);
        assert_eq!(pic.master.isr, 1 << 3); // IRQ3 in service
                                            // Specific EOI for IRQ3 (0x63)
        pic.write(PIC_MASTER_CMD, 0x63, 1);
        assert_eq!(pic.master.isr, 0); // Cleared
    }

    #[test]
    fn test_pic_nonspecific_eoi() {
        let mut pic = BxPicC::new();
        pic.master.imr = 0x00;
        pic.raise_irq(0);
        pic.iac();
        assert_eq!(pic.master.isr, 1 << 0); // IRQ0 in service
                                            // Non-specific EOI (0x20)
        pic.write(PIC_MASTER_CMD, 0x20, 1);
        assert_eq!(pic.master.isr, 0); // Cleared highest priority (IRQ0)
    }

    #[test]
    fn test_clear_highest_interrupt() {
        let mut state = Pic8259State::default();
        state.isr = 0b0000_1010; // IRQ1 and IRQ3 in service
        state.lowest_priority = 7; // Priority order: 0,1,2,3,4,5,6,7
        state.clear_highest_interrupt();
        // Should clear IRQ1 (highest priority in service)
        assert_eq!(state.isr, 0b0000_1000); // Only IRQ3 remains
    }

    #[test]
    fn test_pic_service_priority() {
        let mut state = Pic8259State::default();
        state.master = true;
        state.imr = 0x00;
        state.irr = 0b0000_0101; // IRQ0 and IRQ2 pending
        state.lowest_priority = 7;
        let action = state.service();
        assert_eq!(action, PicServiceAction::RaiseIntr);
        assert_eq!(state.irq, 0); // IRQ0 wins (highest priority)
        assert!(state.int_pin);
    }

    #[test]
    fn test_pic_service_no_double_assert() {
        let mut state = Pic8259State::default();
        state.master = true;
        state.imr = 0x00;
        state.irr = 0b0000_0101;
        state.lowest_priority = 7;
        // First service: asserts INT
        let action = state.service();
        assert_eq!(action, PicServiceAction::RaiseIntr);
        assert!(state.int_pin);
        // Second service: INT already asserted, no action
        let action = state.service();
        assert_eq!(action, PicServiceAction::None);
    }

    #[test]
    fn test_pic_icw1_resets() {
        let mut pic = BxPicC::new();
        pic.master.auto_eoi = true;
        pic.master.rotate_on_autoeoi = true;
        pic.master.lowest_priority = 3;
        // Send ICW1 (0x11 = init + ICW4 required)
        pic.write(PIC_MASTER_CMD, 0x11, 1);
        assert!(pic.master.init.in_init);
        assert!(!pic.master.auto_eoi);
        assert!(!pic.master.rotate_on_autoeoi);
        assert_eq!(pic.master.lowest_priority, 7);
        assert_eq!(pic.master.imr, 0x00);
    }

    #[test]
    fn test_pic_rotate_nonspecific_eoi() {
        let mut pic = BxPicC::new();
        pic.master.imr = 0x00;
        pic.master.lowest_priority = 7; // Priority: 0 highest
        pic.raise_irq(0);
        pic.iac();
        assert_eq!(pic.master.isr, 1 << 0);
        // Rotate on non-specific EOI (0xA0): clears highest ISR, increments lowest_priority
        pic.write(PIC_MASTER_CMD, 0xA0, 1);
        assert_eq!(pic.master.isr, 0);
        assert_eq!(pic.master.lowest_priority, 0); // 7+1 = 0 (wrapped)
    }
}
