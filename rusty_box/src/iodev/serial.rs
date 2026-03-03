//! 16550 UART Serial Port Controller
//!
//! Based on Bochs iodev/serial.cc (1815 lines) + serial.h (264 lines)
//! Implements a fully functional 16550A UART with 16-byte FIFOs.
//!
//! Port layout:
//!   COM1: 0x3F8-0x3FF, IRQ 4
//!   COM2: 0x2F8-0x2FF, IRQ 3
//!   COM3: 0x3E8-0x3EF, IRQ 4
//!   COM4: 0x2E8-0x2EF, IRQ 3

use alloc::collections::VecDeque;
use core::ffi::c_void;

/// UART crystal oscillator frequency (Hz) — Bochs BX_PC_CLOCK_XTL
const UART_CLOCK_XTL: f64 = 1_843_200.0;

/// COM port base addresses
const COM_BASES: [u16; 4] = [0x03F8, 0x02F8, 0x03E8, 0x02E8];
/// COM port IRQ assignments — COM1=IRQ4, COM2=IRQ3, COM3=IRQ4, COM4=IRQ3
const COM_IRQS: [u8; 4] = [4, 3, 4, 3];

/// FIFO size (16550A standard)
const FIFO_SIZE: usize = 16;

/// RX FIFO trigger levels indexed by 2-bit rxtrigger field
const RX_FIFO_TRIGGERS: [u8; 4] = [1, 4, 8, 14];

// Register offsets from base address
const REG_RBR_THR: u16 = 0; // RBR (read) / THR (write) when DLAB=0; DLL when DLAB=1
const REG_IER_DLM: u16 = 1; // IER when DLAB=0; DLM when DLAB=1
const REG_IIR_FCR: u16 = 2; // IIR (read) / FCR (write)
const REG_LCR: u16 = 3; // Line Control Register
const REG_MCR: u16 = 4; // Modem Control Register
const REG_LSR: u16 = 5; // Line Status Register
const REG_MSR: u16 = 6; // Modem Status Register
const REG_SCR: u16 = 7; // Scratch Register

/// Interrupt source types (matching Bochs BX_SER_INT_*)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum IntSource {
    Ier = 0,      // IER changed — re-evaluate pending interrupts
    RxData = 1,   // Received data available
    TxHold = 2,   // THR empty
    RxLstat = 3,  // Receiver line status error
    ModStat = 4,  // Modem status change
    Fifo = 5,     // FIFO character timeout
}

/// Interrupt Enable Register bits
#[derive(Debug, Default, Clone, Copy)]
struct IntEnable {
    rxdata_enable: bool,
    txhold_enable: bool,
    rxlstat_enable: bool,
    modstat_enable: bool,
}

/// Interrupt Identification Register state
#[derive(Debug, Clone, Copy)]
struct IntIdent {
    ipending: bool, // true = NO interrupt pending (inverted logic!)
    int_id: u8,     // Interrupt ID code (0-6)
}

impl Default for IntIdent {
    fn default() -> Self {
        Self {
            ipending: true, // No interrupt pending by default
            int_id: 0,
        }
    }
}

/// FIFO Control state
#[derive(Debug, Default, Clone, Copy)]
struct FifoControl {
    enable: bool,
    rxtrigger: u8, // 2-bit trigger level selector
}

/// Line Control Register state
#[derive(Debug, Clone, Copy)]
struct LineControl {
    wordlen_sel: u8,      // 0=5, 1=6, 2=7, 3=8 bits
    stopbits: bool,       // 0=1 stop, 1=1.5/2 stop
    parity_enable: bool,
    evenparity_sel: bool,
    stick_parity: bool,
    break_cntl: bool,
    dlab: bool,           // Divisor Latch Access Bit
}

impl Default for LineControl {
    fn default() -> Self {
        Self {
            wordlen_sel: 0,
            stopbits: false,
            parity_enable: false,
            evenparity_sel: false,
            stick_parity: false,
            break_cntl: false,
            dlab: false,
        }
    }
}

/// Modem Control Register state
#[derive(Debug, Default, Clone, Copy)]
struct ModemControl {
    dtr: bool,
    rts: bool,
    out1: bool,
    out2: bool,            // MUST be set for interrupts to reach PIC
    local_loopback: bool,
}

/// Line Status Register state
#[derive(Debug, Clone, Copy)]
struct LineStatus {
    rxdata_ready: bool,
    overrun_error: bool,
    parity_error: bool,
    framing_error: bool,
    break_int: bool,
    thr_empty: bool,
    tsr_empty: bool,
    fifo_error: bool,
}

impl Default for LineStatus {
    fn default() -> Self {
        Self {
            rxdata_ready: false,
            overrun_error: false,
            parity_error: false,
            framing_error: false,
            break_int: false,
            thr_empty: true,  // THR starts empty
            tsr_empty: true,  // TSR starts empty
            fifo_error: false,
        }
    }
}

/// Modem Status Register state
#[derive(Debug, Default, Clone, Copy)]
struct ModemStatus {
    delta_cts: bool,
    delta_dsr: bool,
    ri_trailedge: bool,
    delta_dcd: bool,
    cts: bool,
    dsr: bool,
    ri: bool,
    dcd: bool,
}

/// State for one serial port
#[derive(Debug)]
struct SerialPort {
    // Interrupt tracking
    ls_interrupt: bool,
    ms_interrupt: bool,
    rx_interrupt: bool,
    tx_interrupt: bool,
    fifo_interrupt: bool,
    ls_ipending: bool,
    ms_ipending: bool,
    rx_ipending: bool,
    fifo_ipending: bool,

    irq: u8,
    base: u16,

    // FIFOs
    rx_fifo: VecDeque<u8>,
    tx_fifo: VecDeque<u8>,

    // Registers
    rxbuffer: u8,
    thrbuffer: u8,
    tsrbuffer: u8,
    int_enable: IntEnable,
    int_ident: IntIdent,
    fifo_cntl: FifoControl,
    line_cntl: LineControl,
    modem_cntl: ModemControl,
    line_status: LineStatus,
    modem_status: ModemStatus,
    scratch: u8,

    // Divisor latch
    divisor_lsb: u8,
    divisor_msb: u8,

    // TX output buffer — bytes written by guest, drained by host
    tx_output: VecDeque<u8>,
}

impl SerialPort {
    fn new(port_index: usize) -> Self {
        let mut s = Self {
            ls_interrupt: false,
            ms_interrupt: false,
            rx_interrupt: false,
            tx_interrupt: false,
            fifo_interrupt: false,
            ls_ipending: false,
            ms_ipending: false,
            rx_ipending: false,
            fifo_ipending: false,

            irq: COM_IRQS[port_index],
            base: COM_BASES[port_index],

            rx_fifo: VecDeque::with_capacity(FIFO_SIZE),
            tx_fifo: VecDeque::with_capacity(FIFO_SIZE),

            rxbuffer: 0,
            thrbuffer: 0,
            tsrbuffer: 0,
            int_enable: IntEnable::default(),
            int_ident: IntIdent::default(),
            fifo_cntl: FifoControl::default(),
            line_cntl: LineControl::default(),
            modem_cntl: ModemControl::default(),
            line_status: LineStatus::default(),
            modem_status: ModemStatus::default(),
            scratch: 0,

            divisor_lsb: 1, // Default divisor=1 → 115200 baud
            divisor_msb: 0,

            tx_output: VecDeque::with_capacity(256),
        };
        // Simulate connected device
        s.modem_status.cts = true;
        s.modem_status.dsr = true;
        s
    }

    fn reset(&mut self) {
        self.ls_interrupt = false;
        self.ms_interrupt = false;
        self.rx_interrupt = false;
        self.tx_interrupt = false;
        self.fifo_interrupt = false;
        self.ls_ipending = false;
        self.ms_ipending = false;
        self.rx_ipending = false;
        self.fifo_ipending = false;

        self.rx_fifo.clear();
        self.tx_fifo.clear();
        self.tx_output.clear();

        self.rxbuffer = 0;
        self.thrbuffer = 0;
        self.tsrbuffer = 0;
        self.int_enable = IntEnable::default();
        self.int_ident = IntIdent::default();
        self.fifo_cntl = FifoControl::default();
        self.line_cntl = LineControl::default();
        self.modem_cntl = ModemControl::default();
        self.line_status = LineStatus::default();
        self.modem_status = ModemStatus::default();
        self.scratch = 0;
        self.divisor_lsb = 1;
        self.divisor_msb = 0;

        // Simulate connected device
        self.modem_status.cts = true;
        self.modem_status.dsr = true;
    }
}

/// 16550 UART Serial Controller — supports up to 4 COM ports
#[derive(Debug)]
pub struct BxSerialC {
    ports: [SerialPort; 4],
    num_ports: usize,
    /// Pending IRQ raise/lower actions — processed by the PIC after handler returns
    pending_irq_raise: [bool; 4],
    pending_irq_lower: [bool; 4],
}

impl Default for BxSerialC {
    fn default() -> Self {
        Self::new(1) // Default: COM1 only
    }
}

impl BxSerialC {
    pub fn new(num_ports: usize) -> Self {
        let num_ports = num_ports.min(4);
        Self {
            ports: [
                SerialPort::new(0),
                SerialPort::new(1),
                SerialPort::new(2),
                SerialPort::new(3),
            ],
            num_ports,
            pending_irq_raise: [false; 4],
            pending_irq_lower: [false; 4],
        }
    }

    pub fn reset(&mut self) {
        for port in &mut self.ports {
            port.reset();
        }
        self.pending_irq_raise = [false; 4];
        self.pending_irq_lower = [false; 4];
    }

    /// Drain transmitted bytes from a port (for host-side consumption)
    #[allow(dead_code)]
    pub fn drain_tx_output(&mut self, port_index: usize) -> impl Iterator<Item = u8> + '_ {
        self.ports[port_index].tx_output.drain(..)
    }

    /// Check if any IRQ actions are pending, and return them
    /// Returns (irq_number, raise) pairs to process
    pub fn take_pending_irqs(&mut self) -> impl Iterator<Item = (u8, bool)> + '_ {
        let mut results = alloc::vec::Vec::new();
        for i in 0..self.num_ports {
            if self.pending_irq_raise[i] {
                self.pending_irq_raise[i] = false;
                results.push((self.ports[i].irq, true));
            }
            if self.pending_irq_lower[i] {
                self.pending_irq_lower[i] = false;
                results.push((self.ports[i].irq, false));
            }
        }
        results.into_iter()
    }

    /// Identify which COM port a given I/O address belongs to
    fn port_for_addr(&self, addr: u16) -> Option<usize> {
        let base = addr & 0xFFF8; // Mask off low 3 bits
        for i in 0..self.num_ports {
            if base == COM_BASES[i] {
                return Some(i);
            }
        }
        None
    }

    // ========================================================================
    // Interrupt management (matching Bochs serial.cc raise_interrupt/lower_interrupt)
    // ========================================================================

    fn raise_interrupt(&mut self, port_idx: usize, source: IntSource) {
        let s = &mut self.ports[port_idx];
        let mut gen_int = false;

        match source {
            IntSource::RxData => {
                if s.int_enable.rxdata_enable {
                    s.rx_interrupt = true;
                    gen_int = true;
                } else {
                    s.rx_ipending = true;
                }
            }
            IntSource::TxHold => {
                if s.int_enable.txhold_enable {
                    s.tx_interrupt = true;
                    gen_int = true;
                }
                // No pending for TX — re-evaluated on IER change
            }
            IntSource::RxLstat => {
                if s.int_enable.rxlstat_enable {
                    s.ls_interrupt = true;
                    gen_int = true;
                } else {
                    s.ls_ipending = true;
                }
            }
            IntSource::ModStat => {
                if s.int_enable.modstat_enable {
                    s.ms_interrupt = true;
                    gen_int = true;
                } else {
                    s.ms_ipending = true;
                }
            }
            IntSource::Fifo => {
                if s.int_enable.rxdata_enable {
                    s.fifo_interrupt = true;
                    gen_int = true;
                } else {
                    s.fifo_ipending = true;
                }
            }
            IntSource::Ier => {
                gen_int = true;
            }
        }

        if gen_int && s.modem_cntl.out2 {
            self.pending_irq_raise[port_idx] = true;
        }
    }

    fn lower_interrupt(&mut self, port_idx: usize) {
        let s = &self.ports[port_idx];
        if !s.ls_interrupt && !s.ms_interrupt && !s.rx_interrupt
            && !s.tx_interrupt && !s.fifo_interrupt
        {
            self.pending_irq_lower[port_idx] = true;
        }
    }

    // ========================================================================
    // RX FIFO enqueue (matching Bochs serial.cc rx_fifo_enq)
    // ========================================================================

    fn rx_fifo_enq(&mut self, port_idx: usize, data: u8) {
        let s = &mut self.ports[port_idx];

        if s.fifo_cntl.enable {
            if s.rx_fifo.len() >= FIFO_SIZE {
                s.line_status.overrun_error = true;
                self.raise_interrupt(port_idx, IntSource::RxLstat);
                return;
            }
            s.rx_fifo.push_back(data);
            let trigger = RX_FIFO_TRIGGERS[s.fifo_cntl.rxtrigger as usize] as usize;
            if s.rx_fifo.len() >= trigger {
                s.line_status.rxdata_ready = true;
                self.raise_interrupt(port_idx, IntSource::RxData);
            }
            // If trigger not reached, a real 16550 would start the FIFO timeout timer.
            // We skip the timer for simplicity — data is immediately available.
        } else {
            if s.line_status.rxdata_ready {
                s.line_status.overrun_error = true;
                self.raise_interrupt(port_idx, IntSource::RxLstat);
                return;
            }
            s.rxbuffer = data;
            s.line_status.rxdata_ready = true;
            self.raise_interrupt(port_idx, IntSource::RxData);
        }
    }

    /// Feed data into a COM port's RX path (called from outside to inject serial input)
    #[allow(dead_code)]
    pub fn receive_byte(&mut self, port_index: usize, data: u8) {
        if port_index < self.num_ports {
            self.rx_fifo_enq(port_index, data);
        }
    }

    // ========================================================================
    // Register read handler (matching Bochs serial.cc read())
    // ========================================================================

    pub fn read(&mut self, port: u16, _io_len: u8) -> u32 {
        let port_idx = match self.port_for_addr(port) {
            Some(i) => i,
            None => return 0xFF,
        };
        let offset = port & 0x07;

        // Use direct indexing instead of a long-lived mutable borrow to allow
        // calling self.lower_interrupt() within branches.
        match offset {
            REG_RBR_THR => {
                if self.ports[port_idx].line_cntl.dlab {
                    self.ports[port_idx].divisor_lsb as u32
                } else {
                    let data = if self.ports[port_idx].fifo_cntl.enable {
                        let d = self.ports[port_idx].rx_fifo.pop_front().unwrap_or(0);
                        if self.ports[port_idx].rx_fifo.is_empty() {
                            self.ports[port_idx].line_status.rxdata_ready = false;
                            self.ports[port_idx].rx_interrupt = false;
                            self.ports[port_idx].rx_ipending = false;
                            self.ports[port_idx].fifo_interrupt = false;
                            self.ports[port_idx].fifo_ipending = false;
                        }
                        d
                    } else {
                        self.ports[port_idx].line_status.rxdata_ready = false;
                        self.ports[port_idx].rx_interrupt = false;
                        self.ports[port_idx].rx_ipending = false;
                        self.ports[port_idx].rxbuffer
                    };
                    self.lower_interrupt(port_idx);
                    data as u32
                }
            }

            REG_IER_DLM => {
                if self.ports[port_idx].line_cntl.dlab {
                    self.ports[port_idx].divisor_msb as u32
                } else {
                    let s = &self.ports[port_idx];
                    let mut val = 0u8;
                    if s.int_enable.rxdata_enable { val |= 0x01; }
                    if s.int_enable.txhold_enable { val |= 0x02; }
                    if s.int_enable.rxlstat_enable { val |= 0x04; }
                    if s.int_enable.modstat_enable { val |= 0x08; }
                    val as u32
                }
            }

            REG_IIR_FCR => {
                let s = &self.ports[port_idx];
                let (ipending, int_id) = if s.ls_interrupt {
                    (false, 0x03u8)
                } else if s.fifo_interrupt {
                    (false, 0x06)
                } else if s.rx_interrupt {
                    (false, 0x02)
                } else if s.tx_interrupt {
                    (false, 0x01)
                } else if s.ms_interrupt {
                    (false, 0x00)
                } else {
                    (true, 0x00)
                };
                let fifo_bits = if s.fifo_cntl.enable { 0xC0u8 } else { 0x00 };
                let iir_val = (if ipending { 1u8 } else { 0 })
                    | ((int_id & 0x07) << 1)
                    | fifo_bits;

                self.ports[port_idx].tx_interrupt = false;
                self.lower_interrupt(port_idx);
                iir_val as u32
            }

            REG_LCR => {
                let s = &self.ports[port_idx];
                let mut val = s.line_cntl.wordlen_sel;
                if s.line_cntl.stopbits { val |= 0x04; }
                if s.line_cntl.parity_enable { val |= 0x08; }
                if s.line_cntl.evenparity_sel { val |= 0x10; }
                if s.line_cntl.stick_parity { val |= 0x20; }
                if s.line_cntl.break_cntl { val |= 0x40; }
                if s.line_cntl.dlab { val |= 0x80; }
                val as u32
            }

            REG_MCR => {
                let s = &self.ports[port_idx];
                let mut val = 0u8;
                if s.modem_cntl.dtr { val |= 0x01; }
                if s.modem_cntl.rts { val |= 0x02; }
                if s.modem_cntl.out1 { val |= 0x04; }
                if s.modem_cntl.out2 { val |= 0x08; }
                if s.modem_cntl.local_loopback { val |= 0x10; }
                val as u32
            }

            REG_LSR => {
                let s = &self.ports[port_idx];
                let mut val = 0u8;
                if s.line_status.rxdata_ready { val |= 0x01; }
                if s.line_status.overrun_error { val |= 0x02; }
                if s.line_status.parity_error { val |= 0x04; }
                if s.line_status.framing_error { val |= 0x08; }
                if s.line_status.break_int { val |= 0x10; }
                if s.line_status.thr_empty { val |= 0x20; }
                if s.line_status.tsr_empty { val |= 0x40; }
                if s.line_status.fifo_error { val |= 0x80; }

                let s = &mut self.ports[port_idx];
                s.line_status.overrun_error = false;
                s.line_status.parity_error = false;
                s.line_status.framing_error = false;
                s.line_status.break_int = false;
                s.line_status.fifo_error = false;
                s.ls_interrupt = false;
                s.ls_ipending = false;

                self.lower_interrupt(port_idx);
                val as u32
            }

            REG_MSR => {
                let s = &self.ports[port_idx];
                let mut val = 0u8;
                if s.modem_status.delta_cts { val |= 0x01; }
                if s.modem_status.delta_dsr { val |= 0x02; }
                if s.modem_status.ri_trailedge { val |= 0x04; }
                if s.modem_status.delta_dcd { val |= 0x08; }
                if s.modem_status.cts { val |= 0x10; }
                if s.modem_status.dsr { val |= 0x20; }
                if s.modem_status.ri { val |= 0x40; }
                if s.modem_status.dcd { val |= 0x80; }

                let s = &mut self.ports[port_idx];
                s.modem_status.delta_cts = false;
                s.modem_status.delta_dsr = false;
                s.modem_status.ri_trailedge = false;
                s.modem_status.delta_dcd = false;
                s.ms_interrupt = false;
                s.ms_ipending = false;

                self.lower_interrupt(port_idx);
                val as u32
            }

            REG_SCR => self.ports[port_idx].scratch as u32,

            _ => 0xFF,
        }
    }

    // ========================================================================
    // Register write handler (matching Bochs serial.cc write())
    // ========================================================================

    pub fn write(&mut self, port: u16, value: u32, _io_len: u8) {
        let port_idx = match self.port_for_addr(port) {
            Some(i) => i,
            None => return,
        };
        let offset = port & 0x07;
        let val = value as u8;

        match offset {
            REG_RBR_THR => {
                let s = &mut self.ports[port_idx];
                if s.line_cntl.dlab {
                    // DLAB=1: write Divisor Latch LSB
                    s.divisor_lsb = val;
                } else {
                    // DLAB=0: write THR
                    let bitmask: u8 = 0xFF >> (3u8.saturating_sub(s.line_cntl.wordlen_sel));
                    let data = val & bitmask;

                    if s.line_status.thr_empty {
                        if s.fifo_cntl.enable && !s.modem_cntl.local_loopback {
                            s.tx_fifo.push_back(data);
                        } else {
                            s.thrbuffer = data;
                        }
                        s.line_status.thr_empty = false;

                        if s.line_status.tsr_empty {
                            // Move to shift register
                            if s.fifo_cntl.enable && !s.modem_cntl.local_loopback {
                                if let Some(byte) = s.tx_fifo.pop_front() {
                                    s.tsrbuffer = byte;
                                    s.line_status.thr_empty = s.tx_fifo.is_empty();
                                }
                            } else {
                                s.tsrbuffer = s.thrbuffer;
                                s.line_status.thr_empty = true;
                            }

                            if s.line_status.thr_empty {
                                self.raise_interrupt(port_idx, IntSource::TxHold);
                            }

                            let s = &mut self.ports[port_idx];
                            s.line_status.tsr_empty = false;

                            if s.modem_cntl.local_loopback {
                                // Loopback: immediately enqueue into RX
                                let byte = s.tsrbuffer;
                                s.line_status.tsr_empty = true;
                                self.rx_fifo_enq(port_idx, byte);
                            } else {
                                // "Transmit" immediately — we're an emulator
                                let s = &mut self.ports[port_idx];
                                s.tx_output.push_back(s.tsrbuffer);
                                s.line_status.tsr_empty = true;
                            }
                        } else {
                            // TSR busy — clear TX interrupt, data queued
                            let s = &mut self.ports[port_idx];
                            s.tx_interrupt = false;
                            self.lower_interrupt(port_idx);
                        }
                    } else if s.fifo_cntl.enable {
                        // THR already has data, FIFO mode
                        if s.tx_fifo.len() < FIFO_SIZE {
                            s.tx_fifo.push_back(data);
                        }
                        // else: overflow, silently drop
                    }
                }
            }

            REG_IER_DLM => {
                let s = &mut self.ports[port_idx];
                if s.line_cntl.dlab {
                    // DLAB=1: write Divisor Latch MSB
                    s.divisor_msb = val;
                } else {
                    // DLAB=0: write IER
                    let new_rxdata = (val & 0x01) != 0;
                    let new_txhold = (val & 0x02) != 0;
                    let new_rxlstat = (val & 0x04) != 0;
                    let new_modstat = (val & 0x08) != 0;

                    // Modem status enable transition
                    if new_modstat && !s.int_enable.modstat_enable {
                        if s.ms_ipending {
                            s.ms_interrupt = true;
                            s.ms_ipending = false;
                        }
                    } else if !new_modstat && s.int_enable.modstat_enable {
                        if s.ms_interrupt {
                            s.ms_ipending = true;
                            s.ms_interrupt = false;
                        }
                    }

                    // TX hold enable transition
                    if new_txhold && !s.int_enable.txhold_enable {
                        if s.line_status.thr_empty {
                            s.tx_interrupt = true;
                        }
                    } else if !new_txhold && s.int_enable.txhold_enable {
                        s.tx_interrupt = false;
                    }

                    // RX data enable transition
                    if new_rxdata && !s.int_enable.rxdata_enable {
                        if s.fifo_ipending {
                            s.fifo_interrupt = true;
                            s.fifo_ipending = false;
                        }
                        if s.rx_ipending {
                            s.rx_interrupt = true;
                            s.rx_ipending = false;
                        }
                    } else if !new_rxdata && s.int_enable.rxdata_enable {
                        if s.rx_interrupt {
                            s.rx_ipending = true;
                            s.rx_interrupt = false;
                        }
                        if s.fifo_interrupt {
                            s.fifo_ipending = true;
                            s.fifo_interrupt = false;
                        }
                    }

                    // RX line status enable transition
                    if new_rxlstat && !s.int_enable.rxlstat_enable {
                        if s.ls_ipending {
                            s.ls_interrupt = true;
                            s.ls_ipending = false;
                        }
                    } else if !new_rxlstat && s.int_enable.rxlstat_enable {
                        if s.ls_interrupt {
                            s.ls_ipending = true;
                            s.ls_interrupt = false;
                        }
                    }

                    s.int_enable.rxdata_enable = new_rxdata;
                    s.int_enable.txhold_enable = new_txhold;
                    s.int_enable.rxlstat_enable = new_rxlstat;
                    s.int_enable.modstat_enable = new_modstat;

                    self.raise_interrupt(port_idx, IntSource::Ier);
                    self.lower_interrupt(port_idx);
                }
            }

            REG_IIR_FCR => {
                // FCR write (IIR is read-only)
                let s = &mut self.ports[port_idx];
                let new_enable = (val & 0x01) != 0;

                if new_enable && !s.fifo_cntl.enable {
                    // Enabling FIFOs
                    s.fifo_cntl.enable = true;
                    tracing::debug!("COM{}: FIFO enabled", port_idx + 1);
                } else if !new_enable && s.fifo_cntl.enable {
                    // Disabling FIFOs
                    s.fifo_cntl.enable = false;
                    s.rx_fifo.clear();
                    s.tx_fifo.clear();
                    tracing::debug!("COM{}: FIFO disabled", port_idx + 1);
                }

                // Reset RX FIFO (bit 1, self-clearing)
                if (val & 0x02) != 0 {
                    s.rx_fifo.clear();
                }
                // Reset TX FIFO (bit 2, self-clearing)
                if (val & 0x04) != 0 {
                    s.tx_fifo.clear();
                }

                s.fifo_cntl.rxtrigger = (val >> 6) & 0x03;
            }

            REG_LCR => {
                let s = &mut self.ports[port_idx];
                let prev_dlab = s.line_cntl.dlab;

                s.line_cntl.wordlen_sel = val & 0x03;
                s.line_cntl.stopbits = (val & 0x04) != 0;
                s.line_cntl.parity_enable = (val & 0x08) != 0;
                s.line_cntl.evenparity_sel = (val & 0x10) != 0;
                s.line_cntl.stick_parity = (val & 0x20) != 0;
                s.line_cntl.break_cntl = (val & 0x40) != 0;
                s.line_cntl.dlab = (val & 0x80) != 0;

                // When DLAB transitions from 1→0, recalculate baud rate
                if prev_dlab && !s.line_cntl.dlab {
                    let divisor = ((s.divisor_msb as u16) << 8) | (s.divisor_lsb as u16);
                    if divisor > 0 {
                        let baudrate = (UART_CLOCK_XTL / (16.0 * divisor as f64)) as u32;
                        tracing::debug!(
                            "COM{}: baud rate set to {} (divisor={})",
                            port_idx + 1, baudrate, divisor
                        );
                    }
                }
            }

            REG_MCR => {
                let s = &mut self.ports[port_idx];
                let prev_loopback = s.modem_cntl.local_loopback;

                s.modem_cntl.dtr = (val & 0x01) != 0;
                s.modem_cntl.rts = (val & 0x02) != 0;
                s.modem_cntl.out1 = (val & 0x04) != 0;
                s.modem_cntl.out2 = (val & 0x08) != 0;
                s.modem_cntl.local_loopback = (val & 0x10) != 0;

                if s.modem_cntl.local_loopback {
                    // Loopback: MCR outputs reflected to MSR inputs
                    // RTS → CTS, DTR → DSR, OUT1 → RI, OUT2 → DCD
                    let new_cts = s.modem_cntl.rts;
                    let new_dsr = s.modem_cntl.dtr;
                    let new_ri = s.modem_cntl.out1;
                    let new_dcd = s.modem_cntl.out2;

                    // Detect changes for delta bits
                    if new_cts != s.modem_status.cts { s.modem_status.delta_cts = true; }
                    if new_dsr != s.modem_status.dsr { s.modem_status.delta_dsr = true; }
                    if !new_ri && s.modem_status.ri { s.modem_status.ri_trailedge = true; }
                    if new_dcd != s.modem_status.dcd { s.modem_status.delta_dcd = true; }

                    s.modem_status.cts = new_cts;
                    s.modem_status.dsr = new_dsr;
                    s.modem_status.ri = new_ri;
                    s.modem_status.dcd = new_dcd;

                    if s.modem_status.delta_cts || s.modem_status.delta_dsr
                        || s.modem_status.ri_trailedge || s.modem_status.delta_dcd
                    {
                        self.raise_interrupt(port_idx, IntSource::ModStat);
                    }
                } else if prev_loopback {
                    // Exiting loopback — restore CTS/DSR as "connected"
                    let s = &mut self.ports[port_idx];
                    s.modem_status.cts = true;
                    s.modem_status.dsr = true;
                    s.modem_status.ri = false;
                    s.modem_status.dcd = false;
                }
            }

            REG_LSR => {
                // LSR is mostly read-only. Writes are ignored per 16550 spec.
                tracing::trace!("COM{}: write to LSR ignored (value={:#04x})", port_idx + 1, val);
            }

            REG_MSR => {
                // MSR is read-only. Writes are ignored.
                tracing::trace!("COM{}: write to MSR ignored (value={:#04x})", port_idx + 1, val);
            }

            REG_SCR => {
                self.ports[port_idx].scratch = val;
            }

            _ => {}
        }
    }
}

// ============================================================================
// I/O port handler functions for the device infrastructure
// ============================================================================

/// Serial port read handler for I/O port infrastructure
pub fn serial_read_handler(this_ptr: *mut c_void, port: u16, io_len: u8) -> u32 {
    let serial = unsafe { &mut *(this_ptr as *mut BxSerialC) };
    serial.read(port, io_len)
}

/// Serial port write handler for I/O port infrastructure
pub fn serial_write_handler(this_ptr: *mut c_void, port: u16, value: u32, io_len: u8) {
    let serial = unsafe { &mut *(this_ptr as *mut BxSerialC) };
    serial.write(port, value, io_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_creation() {
        let serial = BxSerialC::new(1);
        assert_eq!(serial.num_ports, 1);
        assert_eq!(serial.ports[0].base, 0x03F8);
        assert_eq!(serial.ports[0].irq, 4);
    }

    #[test]
    fn test_scratch_register() {
        let mut serial = BxSerialC::new(1);
        // Write to scratch register
        serial.write(0x03FF, 0xA5, 1); // base + 7 = SCR
        // Read it back
        assert_eq!(serial.read(0x03FF, 1), 0xA5);
    }

    #[test]
    fn test_lsr_initial_state() {
        let mut serial = BxSerialC::new(1);
        let lsr = serial.read(0x03FD, 1); // base + 5 = LSR
        // thr_empty(bit5) + tsr_empty(bit6) should be set
        assert_eq!(lsr & 0x60, 0x60);
    }

    #[test]
    fn test_iir_no_interrupt() {
        let mut serial = BxSerialC::new(1);
        let iir = serial.read(0x03FA, 1); // base + 2 = IIR
        // ipending bit should be set (no interrupt)
        assert_eq!(iir & 0x01, 0x01);
    }

    #[test]
    fn test_divisor_latch() {
        let mut serial = BxSerialC::new(1);
        // Set DLAB=1 (LCR bit 7)
        serial.write(0x03FB, 0x80, 1); // LCR = 0x80

        // Write divisor (12 = 9600 baud)
        serial.write(0x03F8, 0x0C, 1); // DLL = 12
        serial.write(0x03F9, 0x00, 1); // DLM = 0

        // Read back
        assert_eq!(serial.read(0x03F8, 1), 0x0C);
        assert_eq!(serial.read(0x03F9, 1), 0x00);

        // Clear DLAB
        serial.write(0x03FB, 0x03, 1); // LCR = 0x03 (8-bit, no parity, 1 stop)
    }

    #[test]
    fn test_fifo_enable() {
        let mut serial = BxSerialC::new(1);
        // Enable FIFO
        serial.write(0x03FA, 0x01, 1); // FCR = 0x01 (enable)
        // Read IIR — bits 7:6 should be 0xC0 (FIFO enabled)
        let iir = serial.read(0x03FA, 1);
        assert_eq!(iir & 0xC0, 0xC0);
    }

    #[test]
    fn test_loopback() {
        let mut serial = BxSerialC::new(1);
        // Set 8-bit word length (LCR = 0x03)
        serial.write(0x03FB, 0x03, 1);
        // Enable loopback (MCR bit 4) + OUT2 (bit 3) + DTR (bit 0) + RTS (bit 1)
        serial.write(0x03FC, 0x1B, 1); // MCR = 0x1B

        // In loopback, RTS→CTS and DTR→DSR
        let msr = serial.read(0x03FE, 1); // MSR
        assert_ne!(msr & 0x10, 0, "CTS should reflect RTS"); // CTS
        assert_ne!(msr & 0x20, 0, "DSR should reflect DTR"); // DSR

        // TX should loop to RX
        serial.write(0x03F8, 0x42, 1); // Write THR
        let lsr = serial.read(0x03FD, 1); // Check LSR
        assert_ne!(lsr & 0x01, 0, "rxdata_ready should be set"); // rxdata_ready

        // Read the looped-back data
        let data = serial.read(0x03F8, 1);
        assert_eq!(data, 0x42);
    }

    #[test]
    fn test_thr_write_tx_output() {
        let mut serial = BxSerialC::new(1);
        // Set 8-bit word length (LCR = 0x03)
        serial.write(0x03FB, 0x03, 1);
        // Write a byte to THR (not loopback, not FIFO)
        serial.write(0x03F8, b'H' as u32, 1);
        serial.write(0x03F8, b'i' as u32, 1);

        // Check TX output buffer
        let output: alloc::vec::Vec<u8> = serial.drain_tx_output(0).collect();
        assert_eq!(&output, b"Hi");
    }

    #[test]
    fn test_msr_initial_connected() {
        let mut serial = BxSerialC::new(1);
        let msr = serial.read(0x03FE, 1); // MSR
        // CTS and DSR should be set (simulated connected device)
        assert_ne!(msr & 0x10, 0, "CTS should be set");
        assert_ne!(msr & 0x20, 0, "DSR should be set");
    }
}
