#![allow(unused_assignments, dead_code)]
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

use crate::ring_buffer::RingBuffer;
#[cfg(feature = "std")]
use std::io::{self, Error, ErrorKind, Read, Write};

#[cfg(feature = "std")]
use crate::snapshot::{
    bounds, checked_snapshot_len_add, checked_snapshot_len_mul, SnapshotReader, SnapshotWriteExt,
    SNAPSHOT_SECTION_VERSION,
};


/// UART crystal oscillator frequency (Hz) — Bochs BX_PC_CLOCK_XTL
const UART_CLOCK_HZ: u32 = 1_843_200;

/// COM port base addresses
const COM_BASES: [u16; 4] = [0x03F8, 0x02F8, 0x03E8, 0x02E8];
/// COM port IRQ assignments — COM1=IRQ4, COM2=IRQ3, COM3=IRQ4, COM4=IRQ3
const COM_IRQS: [u8; 4] = [4, 3, 4, 3];

/// FIFO size (16550A standard)
const FIFO_SIZE: usize = 16;

/// Bounded host-visible output retained when no consumer has drained it yet.
const TX_OUTPUT_CAPACITY: usize = 4096;

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
    Ier = 0,     // IER changed — re-evaluate pending interrupts
    RxData = 1,  // Received data available
    TxHold = 2,  // THR empty
    RxLstat = 3, // Receiver line status error
    ModStat = 4, // Modem status change
    Fifo = 5,    // FIFO character timeout
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
#[derive(Debug, Clone, Copy, Default)]
struct LineControl {
    wordlen_sel: u8, // 0=5, 1=6, 2=7, 3=8 bits
    stopbits: bool,  // 0=1 stop, 1=1.5/2 stop
    parity_enable: bool,
    evenparity_sel: bool,
    stick_parity: bool,
    break_cntl: bool,
    dlab: bool, // Divisor Latch Access Bit
}

/// Modem Control Register state
#[derive(Debug, Default, Clone, Copy)]
struct ModemControl {
    dtr: bool,
    rts: bool,
    out1: bool,
    out2: bool, // MUST be set for interrupts to reach PIC
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
            thr_empty: true, // THR starts empty
            tsr_empty: true, // TSR starts empty
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
    rx_fifo: RingBuffer<u8, 16>,
    tx_fifo: RingBuffer<u8, 16>,

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

    // Baud rate and character timing (Bochs serial.h)
    baudrate: u32,
    /// Microseconds per data byte, computed from baudrate and word length.
    /// Formula (Bochs serial.cc):
    ///   databyte_usec = (1000000 / baudrate) * (wordlen_sel + 7)
    /// where wordlen_sel + 7 = total bits (start + data + stop).
    /// Default: 87 usec at 115200 baud, 8-bit word (Bochs serial.cc).
    databyte_usec: u32,

    // Each configured UART owns one FIFO-timeout timer. TX/RX stay immediate.
    // The handle survives reset because the pc-system registration does too.
    fifo_timer_handle: Option<usize>,

    // `Some` means the scheduler must arm/rearm the one-shot after this I/O
    // operation. The value is captured when the byte arrives so a later baud
    // rate change cannot alter its already-established deadline.
    fifo_timeout_delay_usec: Option<u64>,
    /// True when the scheduler must consume the current activate/deactivate
    /// state. Unrelated UART I/O must not restart an existing deadline.
    fifo_timer_request_pending: bool,

    // TX output buffer — bytes written by guest, drained by host
    tx_output: RingBuffer<u8, TX_OUTPUT_CAPACITY>,
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

            rx_fifo: RingBuffer::new(),
            tx_fifo: RingBuffer::new(),

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

            // Bochs serial.cc: default 115200 baud, 87 usec/byte
            baudrate: 115200,
            databyte_usec: 87,

            fifo_timer_handle: None,
            fifo_timeout_delay_usec: None,
            fifo_timer_request_pending: false,

            tx_output: RingBuffer::new(),
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
        self.baudrate = 115200;
        self.databyte_usec = 87;
        // Timer handles persist across soft resets; timeout scheduling does not.
        self.fifo_timeout_delay_usec = None;
        self.fifo_timer_request_pending = true;

        // Simulate connected device
        self.modem_status.cts = true;
        self.modem_status.dsr = true;
    }
}

/// Computes the timing derived from the UART's architectural divisor and line
/// format. A zero divisor has no representable baud rate.
#[inline]
fn serial_timing_from_registers(
    divisor_lsb: u8,
    divisor_msb: u8,
    wordlen_sel: u8,
) -> Option<(u32, u32)> {
    if wordlen_sel > 0x03 {
        return None;
    }

    let divisor = u16::from_le_bytes([divisor_lsb, divisor_msb]);
    if divisor == 0 {
        return None;
    }

    let baudrate = UART_CLOCK_HZ / (u32::from(divisor) * 16);
    if baudrate == 0 {
        return None;
    }

    let frame_bits = u32::from(wordlen_sel) + 7;
    let databyte_usec = 1_000_000u32.checked_mul(frame_bits)? / baudrate;
    Some((baudrate, databyte_usec))
}

#[cfg(feature = "std")]
const SERIAL_SNAPSHOT_HEADER_LEN: u64 = 8;

/// Per-port bytes excluding logical FIFO/output contents and present optional
/// `u64` timer values. FIFO and output counts are included in this fixed part.
#[cfg(feature = "std")]
const SERIAL_SNAPSHOT_PORT_FIXED_LEN: u64 = 71;

#[cfg(feature = "std")]
fn invalid_serial_snapshot(message: &'static str) -> io::Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[cfg(feature = "std")]
fn checked_serial_count(count: usize) -> io::Result<u32> {
    if count > bounds::MAX_SNAPSHOT_QUEUE_LEN {
        return Err(invalid_serial_snapshot(
            "serial snapshot count exceeds implementation bound",
        ));
    }
    u32::try_from(count)
        .map_err(|_| invalid_serial_snapshot("serial snapshot count does not fit u32"))
}

#[cfg(feature = "std")]
fn validate_serial_ring_len<const N: usize>(ring: &RingBuffer<u8, N>) -> io::Result<()> {
    if ring.len() > N.min(bounds::MAX_SNAPSHOT_QUEUE_LEN) {
        return Err(invalid_serial_snapshot(
            "serial snapshot ring length exceeds live capacity",
        ));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn write_serial_ring<W: Write, const N: usize>(
    writer: &mut W,
    ring: &RingBuffer<u8, N>,
) -> io::Result<()> {
    validate_serial_ring_len(ring)?;
    writer.write_u32(checked_serial_count(ring.len())?)?;
    for byte in ring.iter() {
        writer.write_u8(byte)?;
    }
    Ok(())
}

#[cfg(feature = "std")]
fn read_serial_ring<R: Read, const N: usize>(
    reader: &mut SnapshotReader<R>,
) -> io::Result<RingBuffer<u8, N>> {
    let count = reader.read_count(N.min(bounds::MAX_SNAPSHOT_QUEUE_LEN))?;
    let mut ring = RingBuffer::new();
    for _ in 0..count {
        ring.push_back(reader.read_u8()?);
    }
    Ok(ring)
}

#[cfg(feature = "std")]
fn write_optional_handle<W: Write>(writer: &mut W, handle: Option<usize>) -> io::Result<()> {
    match handle {
        Some(handle) => {
            writer.write_bool(true)?;
            writer.write_u64(
                u64::try_from(handle)
                    .map_err(|_| invalid_serial_snapshot("serial timer handle does not fit u64"))?,
            )
        }
        None => writer.write_bool(false),
    }
}

#[cfg(feature = "std")]
fn read_optional_handle<R: Read>(reader: &mut SnapshotReader<R>) -> io::Result<Option<usize>> {
    if !reader.read_bool()? {
        return Ok(None);
    }
    usize::try_from(reader.read_u64()?)
        .map(Some)
        .map_err(|_| invalid_serial_snapshot("serial timer handle does not fit usize"))
}

#[cfg(feature = "std")]
fn write_optional_u64<W: Write>(writer: &mut W, value: Option<u64>) -> io::Result<()> {
    match value {
        Some(value) => {
            writer.write_bool(true)?;
            writer.write_u64(value)
        }
        None => writer.write_bool(false),
    }
}

#[cfg(feature = "std")]
fn read_optional_u64<R: Read>(reader: &mut SnapshotReader<R>) -> io::Result<Option<u64>> {
    if reader.read_bool()? {
        Ok(Some(reader.read_u64()?))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "std")]
fn valid_interrupt_id(value: u8) -> bool {
    matches!(value, 0 | 1 | 2 | 3 | 6)
}

#[cfg(feature = "std")]
fn validate_fifo_timeout_state(
    fifo_cntl: FifoControl,
    rx_fifo: &RingBuffer<u8, FIFO_SIZE>,
    fifo_timeout_delay_usec: Option<u64>,
) -> io::Result<()> {
    let Some(delay) = fifo_timeout_delay_usec else {
        return Ok(());
    };
    if delay == 0 {
        return Err(invalid_serial_snapshot(
            "serial FIFO timeout delay must be nonzero",
        ));
    }
    if !fifo_cntl.enable || rx_fifo.is_empty() {
        return Err(invalid_serial_snapshot(
            "serial FIFO timeout is armed without receive data",
        ));
    }
    let trigger = RX_FIFO_TRIGGERS
        .get(usize::from(fifo_cntl.rxtrigger))
        .copied()
        .ok_or_else(|| invalid_serial_snapshot("serial FIFO trigger is invalid"))?;
    if rx_fifo.len() >= usize::from(trigger) {
        return Err(invalid_serial_snapshot(
            "serial FIFO timeout is armed at receive trigger",
        ));
    }
    Ok(())
}

#[cfg(feature = "std")]
fn validate_serial_port_for_snapshot(port: &SerialPort) -> io::Result<()> {
    validate_serial_ring_len(&port.rx_fifo)?;
    validate_serial_ring_len(&port.tx_fifo)?;
    validate_serial_ring_len(&port.tx_output)?;
    if !port.fifo_cntl.enable && (!port.rx_fifo.is_empty() || !port.tx_fifo.is_empty()) {
        return Err(invalid_serial_snapshot(
            "serial FIFOs contain data while FIFO mode is disabled",
        ));
    }
    if !valid_interrupt_id(port.int_ident.int_id) {
        return Err(invalid_serial_snapshot(
            "serial interrupt identifier is invalid",
        ));
    }
    if port.fifo_cntl.rxtrigger > 0x03 {
        return Err(invalid_serial_snapshot("serial FIFO trigger is invalid"));
    }
    if serial_timing_from_registers(
        port.divisor_lsb,
        port.divisor_msb,
        port.line_cntl.wordlen_sel,
    )
    .is_none()
    {
        return Err(invalid_serial_snapshot(
            "serial divisor or line format is invalid",
        ));
    }
    validate_fifo_timeout_state(
        port.fifo_cntl,
        &port.rx_fifo,
        port.fifo_timeout_delay_usec,
    )
}

#[cfg(feature = "std")]
fn serial_port_snapshot_v3_len(port: &SerialPort) -> io::Result<u64> {
    validate_serial_port_for_snapshot(port)?;
    let rx_len = checked_snapshot_len_mul(
        u64::try_from(port.rx_fifo.len())
            .map_err(|_| invalid_serial_snapshot("serial RX FIFO length does not fit u64"))?,
        1,
    )?;
    let tx_len = checked_snapshot_len_mul(
        u64::try_from(port.tx_fifo.len())
            .map_err(|_| invalid_serial_snapshot("serial TX FIFO length does not fit u64"))?,
        1,
    )?;
    let output_len = checked_snapshot_len_mul(
        u64::try_from(port.tx_output.len())
            .map_err(|_| invalid_serial_snapshot("serial TX output length does not fit u64"))?,
        1,
    )?;

    let mut len = checked_snapshot_len_add(SERIAL_SNAPSHOT_PORT_FIXED_LEN, rx_len)?;
    len = checked_snapshot_len_add(len, tx_len)?;
    len = checked_snapshot_len_add(len, output_len)?;
    if port.fifo_timer_handle.is_some() {
        len = checked_snapshot_len_add(len, 8)?;
    }
    if port.fifo_timeout_delay_usec.is_some() {
        len = checked_snapshot_len_add(len, 8)?;
    }
    Ok(len)
}

#[cfg(feature = "std")]
fn save_serial_port_snapshot_v3<W: Write>(
    port: &SerialPort,
    pending_irq_raise: bool,
    pending_irq_lower: bool,
    writer: &mut W,
) -> io::Result<()> {
    validate_serial_port_for_snapshot(port)?;

    writer.write_u16(port.base)?;
    writer.write_u8(port.irq)?;

    writer.write_bool(port.ls_interrupt)?;
    writer.write_bool(port.ms_interrupt)?;
    writer.write_bool(port.rx_interrupt)?;
    writer.write_bool(port.tx_interrupt)?;
    writer.write_bool(port.fifo_interrupt)?;
    writer.write_bool(port.ls_ipending)?;
    writer.write_bool(port.ms_ipending)?;
    writer.write_bool(port.rx_ipending)?;
    writer.write_bool(port.fifo_ipending)?;

    write_serial_ring(writer, &port.rx_fifo)?;
    write_serial_ring(writer, &port.tx_fifo)?;

    writer.write_u8(port.rxbuffer)?;
    writer.write_u8(port.thrbuffer)?;
    writer.write_u8(port.tsrbuffer)?;

    writer.write_bool(port.int_enable.rxdata_enable)?;
    writer.write_bool(port.int_enable.txhold_enable)?;
    writer.write_bool(port.int_enable.rxlstat_enable)?;
    writer.write_bool(port.int_enable.modstat_enable)?;

    writer.write_bool(port.int_ident.ipending)?;
    writer.write_u8(port.int_ident.int_id)?;

    writer.write_bool(port.fifo_cntl.enable)?;
    writer.write_u8(port.fifo_cntl.rxtrigger)?;

    writer.write_u8(port.line_cntl.wordlen_sel)?;
    writer.write_bool(port.line_cntl.stopbits)?;
    writer.write_bool(port.line_cntl.parity_enable)?;
    writer.write_bool(port.line_cntl.evenparity_sel)?;
    writer.write_bool(port.line_cntl.stick_parity)?;
    writer.write_bool(port.line_cntl.break_cntl)?;
    writer.write_bool(port.line_cntl.dlab)?;

    writer.write_bool(port.modem_cntl.dtr)?;
    writer.write_bool(port.modem_cntl.rts)?;
    writer.write_bool(port.modem_cntl.out1)?;
    writer.write_bool(port.modem_cntl.out2)?;
    writer.write_bool(port.modem_cntl.local_loopback)?;

    writer.write_bool(port.line_status.rxdata_ready)?;
    writer.write_bool(port.line_status.overrun_error)?;
    writer.write_bool(port.line_status.parity_error)?;
    writer.write_bool(port.line_status.framing_error)?;
    writer.write_bool(port.line_status.break_int)?;
    writer.write_bool(port.line_status.thr_empty)?;
    writer.write_bool(port.line_status.tsr_empty)?;
    writer.write_bool(port.line_status.fifo_error)?;

    writer.write_bool(port.modem_status.delta_cts)?;
    writer.write_bool(port.modem_status.delta_dsr)?;
    writer.write_bool(port.modem_status.ri_trailedge)?;
    writer.write_bool(port.modem_status.delta_dcd)?;
    writer.write_bool(port.modem_status.cts)?;
    writer.write_bool(port.modem_status.dsr)?;
    writer.write_bool(port.modem_status.ri)?;
    writer.write_bool(port.modem_status.dcd)?;

    writer.write_u8(port.scratch)?;
    writer.write_u8(port.divisor_lsb)?;
    writer.write_u8(port.divisor_msb)?;
    write_optional_handle(writer, port.fifo_timer_handle)?;
    write_optional_u64(writer, port.fifo_timeout_delay_usec)?;
    writer.write_bool(port.fifo_timer_request_pending)?;
    write_serial_ring(writer, &port.tx_output)?;
    writer.write_bool(pending_irq_raise)?;
    writer.write_bool(pending_irq_lower)
}

#[cfg(feature = "std")]
struct SerialPortSnapshot {
    ls_interrupt: bool,
    ms_interrupt: bool,
    rx_interrupt: bool,
    tx_interrupt: bool,
    fifo_interrupt: bool,
    ls_ipending: bool,
    ms_ipending: bool,
    rx_ipending: bool,
    fifo_ipending: bool,
    rx_fifo: RingBuffer<u8, FIFO_SIZE>,
    tx_fifo: RingBuffer<u8, FIFO_SIZE>,
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
    divisor_lsb: u8,
    divisor_msb: u8,
    fifo_timer_handle: Option<usize>,
    fifo_timeout_delay_usec: Option<u64>,
    fifo_timer_request_pending: bool,
    tx_output: RingBuffer<u8, TX_OUTPUT_CAPACITY>,
    pending_irq_raise: bool,
    pending_irq_lower: bool,
}

#[cfg(feature = "std")]
impl SerialPortSnapshot {
    fn read<R: Read>(
        reader: &mut SnapshotReader<R>,
        expected_base: u16,
        expected_irq: u8,
    ) -> io::Result<Self> {
        let base = reader.read_u16()?;
        let irq = reader.read_u8()?;
        if base != expected_base || irq != expected_irq {
            return Err(invalid_serial_snapshot(
                "serial port topology does not match live configuration",
            ));
        }

        let ls_interrupt = reader.read_bool()?;
        let ms_interrupt = reader.read_bool()?;
        let rx_interrupt = reader.read_bool()?;
        let tx_interrupt = reader.read_bool()?;
        let fifo_interrupt = reader.read_bool()?;
        let ls_ipending = reader.read_bool()?;
        let ms_ipending = reader.read_bool()?;
        let rx_ipending = reader.read_bool()?;
        let fifo_ipending = reader.read_bool()?;
        let rx_fifo = read_serial_ring(reader)?;
        let tx_fifo = read_serial_ring(reader)?;

        let rxbuffer = reader.read_u8()?;
        let thrbuffer = reader.read_u8()?;
        let tsrbuffer = reader.read_u8()?;

        let int_enable = IntEnable {
            rxdata_enable: reader.read_bool()?,
            txhold_enable: reader.read_bool()?,
            rxlstat_enable: reader.read_bool()?,
            modstat_enable: reader.read_bool()?,
        };
        let int_ident_ipending = reader.read_bool()?;
        let int_ident_id = reader.read_u8()?;
        if !valid_interrupt_id(int_ident_id) {
            return Err(invalid_serial_snapshot(
                "serial interrupt identifier is invalid",
            ));
        }
        let int_ident = IntIdent {
            ipending: int_ident_ipending,
            int_id: int_ident_id,
        };

        let fifo_enable = reader.read_bool()?;
        let fifo_rxtrigger = reader.read_u8()?;
        if fifo_rxtrigger > 0x03 {
            return Err(invalid_serial_snapshot("serial FIFO trigger is invalid"));
        }
        let fifo_cntl = FifoControl {
            enable: fifo_enable,
            rxtrigger: fifo_rxtrigger,
        };
        if !fifo_enable && (!rx_fifo.is_empty() || !tx_fifo.is_empty()) {
            return Err(invalid_serial_snapshot(
                "serial FIFOs contain data while FIFO mode is disabled",
            ));
        }

        let line_wordlen_sel = reader.read_u8()?;
        if line_wordlen_sel > 0x03 {
            return Err(invalid_serial_snapshot(
                "serial line word length selector is invalid",
            ));
        }
        let line_cntl = LineControl {
            wordlen_sel: line_wordlen_sel,
            stopbits: reader.read_bool()?,
            parity_enable: reader.read_bool()?,
            evenparity_sel: reader.read_bool()?,
            stick_parity: reader.read_bool()?,
            break_cntl: reader.read_bool()?,
            dlab: reader.read_bool()?,
        };

        let modem_cntl = ModemControl {
            dtr: reader.read_bool()?,
            rts: reader.read_bool()?,
            out1: reader.read_bool()?,
            out2: reader.read_bool()?,
            local_loopback: reader.read_bool()?,
        };
        let line_status = LineStatus {
            rxdata_ready: reader.read_bool()?,
            overrun_error: reader.read_bool()?,
            parity_error: reader.read_bool()?,
            framing_error: reader.read_bool()?,
            break_int: reader.read_bool()?,
            thr_empty: reader.read_bool()?,
            tsr_empty: reader.read_bool()?,
            fifo_error: reader.read_bool()?,
        };
        let modem_status = ModemStatus {
            delta_cts: reader.read_bool()?,
            delta_dsr: reader.read_bool()?,
            ri_trailedge: reader.read_bool()?,
            delta_dcd: reader.read_bool()?,
            cts: reader.read_bool()?,
            dsr: reader.read_bool()?,
            ri: reader.read_bool()?,
            dcd: reader.read_bool()?,
        };

        let scratch = reader.read_u8()?;
        let divisor_lsb = reader.read_u8()?;
        let divisor_msb = reader.read_u8()?;
        if serial_timing_from_registers(divisor_lsb, divisor_msb, line_wordlen_sel).is_none() {
            return Err(invalid_serial_snapshot(
                "serial divisor or line format is invalid",
            ));
        }

        let fifo_timer_handle = read_optional_handle(reader)?;
        let fifo_timeout_delay_usec = read_optional_u64(reader)?;
        validate_fifo_timeout_state(fifo_cntl, &rx_fifo, fifo_timeout_delay_usec)?;
        let fifo_timer_request_pending = reader.read_bool()?;
        let tx_output = read_serial_ring(reader)?;
        let pending_irq_raise = reader.read_bool()?;
        let pending_irq_lower = reader.read_bool()?;

        Ok(Self {
            ls_interrupt,
            ms_interrupt,
            rx_interrupt,
            tx_interrupt,
            fifo_interrupt,
            ls_ipending,
            ms_ipending,
            rx_ipending,
            fifo_ipending,
            rx_fifo,
            tx_fifo,
            rxbuffer,
            thrbuffer,
            tsrbuffer,
            int_enable,
            int_ident,
            fifo_cntl,
            line_cntl,
            modem_cntl,
            line_status,
            modem_status,
            scratch,
            divisor_lsb,
            divisor_msb,
            fifo_timer_handle,
            fifo_timeout_delay_usec,
            fifo_timer_request_pending,
            tx_output,
            pending_irq_raise,
            pending_irq_lower,
        })
    }

    fn apply_to(self, port: &mut SerialPort) {
        port.ls_interrupt = self.ls_interrupt;
        port.ms_interrupt = self.ms_interrupt;
        port.rx_interrupt = self.rx_interrupt;
        port.tx_interrupt = self.tx_interrupt;
        port.fifo_interrupt = self.fifo_interrupt;
        port.ls_ipending = self.ls_ipending;
        port.ms_ipending = self.ms_ipending;
        port.rx_ipending = self.rx_ipending;
        port.fifo_ipending = self.fifo_ipending;
        port.rx_fifo = self.rx_fifo;
        port.tx_fifo = self.tx_fifo;
        port.rxbuffer = self.rxbuffer;
        port.thrbuffer = self.thrbuffer;
        port.tsrbuffer = self.tsrbuffer;
        port.int_enable = self.int_enable;
        port.int_ident = self.int_ident;
        port.fifo_cntl = self.fifo_cntl;
        port.line_cntl = self.line_cntl;
        port.modem_cntl = self.modem_cntl;
        port.line_status = self.line_status;
        port.modem_status = self.modem_status;
        port.scratch = self.scratch;
        port.divisor_lsb = self.divisor_lsb;
        port.divisor_msb = self.divisor_msb;
        port.fifo_timer_handle = self.fifo_timer_handle;
        port.fifo_timeout_delay_usec = self.fifo_timeout_delay_usec;
        port.fifo_timer_request_pending = self.fifo_timer_request_pending;
        port.tx_output = self.tx_output;
    }
}

/// Stack-allocated iterator for pending serial IRQ actions.
/// Avoids heap allocation on the hot tick path.
pub struct PendingIrqs {
    buf: [(u8, bool); 8],
    len: usize,
    pos: usize,
}

impl Iterator for PendingIrqs {
    type Item = (u8, bool);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.len {
            let item = self.buf[self.pos];
            self.pos += 1;
            Some(item)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for PendingIrqs {}

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
        self.ports[port_index].tx_output.drain()
    }

    pub fn tx_output_len(&self, port_index: usize) -> usize {
        self.ports[port_index].tx_output.len()
    }

    /// Number of UARTs that were configured at construction time.
    #[inline]
    pub(crate) const fn configured_port_count(&self) -> usize {
        self.num_ports
    }

    /// Attach or clear this port's fixed `TimerOwner::SerialFifo` handle.
    ///
    /// The central scheduler owns registration. Keeping the handle here makes
    /// serial state snapshot-able and lets it verify the owner-to-port mapping.
    #[inline]
    pub(crate) fn set_fifo_timer_handle(&mut self, port_index: usize, handle: Option<usize>) {
        if port_index < self.num_ports {
            self.ports[port_index].fifo_timer_handle = handle;
        }
    }

    /// Return the registered fixed FIFO-timeout handle for one configured port.
    #[inline]
    pub(crate) fn fifo_timer_handle(&self, port_index: usize) -> Option<usize> {
        self.ports
            .get(port_index)
            .filter(|_| port_index < self.num_ports)
            .and_then(|port| port.fifo_timer_handle)
    }

    /// Return the relative delay for the port's pending FIFO timeout.
    ///
    /// `Some(delay)` directs the scheduler to activate/restart the fixed
    /// one-shot owner; `None` directs it to deactivate the owner.
    #[inline]
    pub(crate) fn fifo_timeout_delay_usec(&self, port_index: usize) -> Option<u64> {
        self.ports
            .get(port_index)
            .filter(|_| port_index < self.num_ports)
            .and_then(|port| port.fifo_timeout_delay_usec)
    }
    /// Drain one scheduler update without restarting the deadline on unrelated I/O.
    #[inline]
    pub(crate) fn take_fifo_timer_update(
        &mut self,
        port_index: usize,
    ) -> Option<Option<u64>> {
        let port = self
            .ports
            .get_mut(port_index)
            .filter(|_| port_index < self.num_ports)?;
        if !core::mem::take(&mut port.fifo_timer_request_pending) {
            return None;
        }
        Some(port.fifo_timeout_delay_usec)
    }

    /// Service a fixed `TimerOwner::SerialFifo(port_index)` callback.
    ///
    /// A stale or canceled callback is harmless. A live timeout only becomes a
    /// FIFO interrupt while data remains below its configured RX trigger.
    pub(crate) fn fifo_timer_fired(&mut self, port_index: usize) -> bool {
        if port_index >= self.num_ports {
            return false;
        }

        let should_assert = {
            let port = &mut self.ports[port_index];
            let was_armed = port.fifo_timeout_delay_usec.take().is_some();
            port.fifo_timer_request_pending = false;
            let trigger = RX_FIFO_TRIGGERS[port.fifo_cntl.rxtrigger as usize] as usize;
            was_armed
                && port.fifo_cntl.enable
                && !port.rx_fifo.is_empty()
                && port.rx_fifo.len() < trigger
        };

        if should_assert {
            self.ports[port_index].line_status.rxdata_ready = true;
            self.raise_interrupt(port_index, IntSource::Fifo);
        }

        should_assert
    }

    /// Check if any IRQ actions are pending, and return them.
    /// Returns (irq_number, raise) pairs to process.
    /// Uses a fixed-size buffer to avoid heap allocation on every tick.
    #[inline]
    pub fn take_pending_irqs(&mut self) -> PendingIrqs {
        let mut buf = [(0u8, false); 8];
        let mut len = 0usize;
        for i in 0..self.num_ports {
            if self.pending_irq_raise[i] {
                self.pending_irq_raise[i] = false;
                buf[len] = (self.ports[i].irq, true);
                len += 1;
            }
            if self.pending_irq_lower[i] {
                self.pending_irq_lower[i] = false;
                buf[len] = (self.ports[i].irq, false);
                len += 1;
            }
        }
        PendingIrqs { buf, len, pos: 0 }
    }

    /// Return the configured UART index which owns an I/O address.
    #[inline]
    pub(crate) fn port_index_for_address(&self, port: u16) -> Option<usize> {
        self.port_for_addr(port)
    }

    /// Identify which COM port a given I/O address belongs to
    fn port_for_addr(&self, addr: u16) -> Option<usize> {
        let base = addr & 0xFFF8; // Mask off low 3 bits
        COM_BASES[..self.num_ports].iter().position(|&b| b == base)
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
                // Bochs serial.cc: only promote to interrupt when
                // ms_ipending is already set AND modstat_enable is on
                if s.ms_ipending && s.int_enable.modstat_enable {
                    s.ms_interrupt = true;
                    s.ms_ipending = false;
                    gen_int = true;
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
        if !s.ls_interrupt
            && !s.ms_interrupt
            && !s.rx_interrupt
            && !s.tx_interrupt
            && !s.fifo_interrupt
        {
            self.pending_irq_lower[port_idx] = true;
        }
    }

    // ========================================================================
    // RX FIFO enqueue (matching Bochs serial.cc rx_fifo_enq)
    // ========================================================================

    fn rx_fifo_enq(&mut self, port_idx: usize, data: u8) {
        if self.ports[port_idx].fifo_cntl.enable {
            if self.ports[port_idx].rx_fifo.len() >= FIFO_SIZE {
                self.ports[port_idx].line_status.overrun_error = true;
                self.raise_interrupt(port_idx, IntSource::RxLstat);
                return;
            }

            let reached_trigger = {
                let port = &mut self.ports[port_idx];
                port.rx_fifo.push_back(data);
                let trigger = RX_FIFO_TRIGGERS[port.fifo_cntl.rxtrigger as usize] as usize;
                let reached_trigger = port.rx_fifo.len() >= trigger;

                if reached_trigger {
                    // A receive-data interrupt supersedes a pending timeout.
                    port.fifo_timeout_delay_usec = None;
                    port.line_status.rxdata_ready = true;
                } else {
                    // Bochs arms a fresh one-shot for every byte below trigger.
                    port.fifo_timeout_delay_usec =
                        Some(u64::from(port.databyte_usec) * 3);
                }
                port.fifo_timer_request_pending = true;

                reached_trigger
            };

            if reached_trigger {
                self.raise_interrupt(port_idx, IntSource::RxData);
            }
            return;
        }

        if self.ports[port_idx].line_status.rxdata_ready {
            self.ports[port_idx].line_status.overrun_error = true;
            self.raise_interrupt(port_idx, IntSource::RxLstat);
            return;
        }
        self.ports[port_idx].rxbuffer = data;
        self.ports[port_idx].line_status.rxdata_ready = true;
        self.raise_interrupt(port_idx, IntSource::RxData);
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
                        // Any receive-data read cancels the outstanding timeout;
                        // a later byte below trigger starts a fresh one-shot.
                        self.ports[port_idx].fifo_timeout_delay_usec = None;
                        self.ports[port_idx].fifo_timer_request_pending = true;
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
                    if s.int_enable.rxdata_enable {
                        val |= 0x01;
                    }
                    if s.int_enable.txhold_enable {
                        val |= 0x02;
                    }
                    if s.int_enable.rxlstat_enable {
                        val |= 0x04;
                    }
                    if s.int_enable.modstat_enable {
                        val |= 0x08;
                    }
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
                let iir_val = (if ipending { 1u8 } else { 0 }) | ((int_id & 0x07) << 1) | fifo_bits;

                self.ports[port_idx].tx_interrupt = false;
                self.lower_interrupt(port_idx);
                iir_val as u32
            }

            REG_LCR => {
                let s = &self.ports[port_idx];
                let mut val = s.line_cntl.wordlen_sel;
                if s.line_cntl.stopbits {
                    val |= 0x04;
                }
                if s.line_cntl.parity_enable {
                    val |= 0x08;
                }
                if s.line_cntl.evenparity_sel {
                    val |= 0x10;
                }
                if s.line_cntl.stick_parity {
                    val |= 0x20;
                }
                if s.line_cntl.break_cntl {
                    val |= 0x40;
                }
                if s.line_cntl.dlab {
                    val |= 0x80;
                }
                val as u32
            }

            REG_MCR => {
                let s = &self.ports[port_idx];
                let mut val = 0u8;
                if s.modem_cntl.dtr {
                    val |= 0x01;
                }
                if s.modem_cntl.rts {
                    val |= 0x02;
                }
                if s.modem_cntl.out1 {
                    val |= 0x04;
                }
                if s.modem_cntl.out2 {
                    val |= 0x08;
                }
                if s.modem_cntl.local_loopback {
                    val |= 0x10;
                }
                val as u32
            }

            REG_LSR => {
                let s = &self.ports[port_idx];
                let mut val = 0u8;
                if s.line_status.rxdata_ready {
                    val |= 0x01;
                }
                if s.line_status.overrun_error {
                    val |= 0x02;
                }
                if s.line_status.parity_error {
                    val |= 0x04;
                }
                if s.line_status.framing_error {
                    val |= 0x08;
                }
                if s.line_status.break_int {
                    val |= 0x10;
                }
                if s.line_status.thr_empty {
                    val |= 0x20;
                }
                if s.line_status.tsr_empty {
                    val |= 0x40;
                }
                if s.line_status.fifo_error {
                    val |= 0x80;
                }

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
                if s.modem_status.delta_cts {
                    val |= 0x01;
                }
                if s.modem_status.delta_dsr {
                    val |= 0x02;
                }
                if s.modem_status.ri_trailedge {
                    val |= 0x04;
                }
                if s.modem_status.delta_dcd {
                    val |= 0x08;
                }
                if s.modem_status.cts {
                    val |= 0x10;
                }
                if s.modem_status.dsr {
                    val |= 0x20;
                }
                if s.modem_status.ri {
                    val |= 0x40;
                }
                if s.modem_status.dcd {
                    val |= 0x80;
                }

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
                                let ch = s.tsrbuffer;
                                s.tx_output.push_back(ch);
                                s.line_status.tsr_empty = true;
                            }
                        } else {
                            // TSR busy — clear TX interrupt, data queued
                            let s = &mut self.ports[port_idx];
                            s.tx_interrupt = false;
                            self.lower_interrupt(port_idx);
                        }
                    } else if s.fifo_cntl.enable {
                        // THR already has data, FIFO mode — queue the byte
                        if s.tx_fifo.len() < FIFO_SIZE {
                            s.tx_fifo.push_back(data);
                        }
                        // Drain FIFO immediately — we're an emulator, no real baud timing
                        let s = &mut self.ports[port_idx];
                        while let Some(byte) = s.tx_fifo.pop_front() {
                            s.tx_output.push_back(byte);
                        }
                        s.line_status.thr_empty = true;
                        s.line_status.tsr_empty = true;
                        self.raise_interrupt(port_idx, IntSource::TxHold);
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
                    } else if !new_modstat && s.int_enable.modstat_enable && s.ms_interrupt {
                        s.ms_ipending = true;
                        s.ms_interrupt = false;
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
                    } else if !new_rxlstat && s.int_enable.rxlstat_enable && s.ls_interrupt {
                        s.ls_ipending = true;
                        s.ls_interrupt = false;
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
                    tracing::trace!("COM{}: FIFO enabled", port_idx + 1);
                } else if !new_enable && s.fifo_cntl.enable {
                    // Disabling FIFOs
                    s.fifo_cntl.enable = false;
                    s.rx_fifo.clear();
                    s.tx_fifo.clear();
                    tracing::trace!("COM{}: FIFO disabled", port_idx + 1);
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
                let trigger = RX_FIFO_TRIGGERS[s.fifo_cntl.rxtrigger as usize] as usize;
                if !s.fifo_cntl.enable
                    || s.rx_fifo.is_empty()
                    || s.rx_fifo.len() >= trigger
                {
                    s.fifo_timeout_delay_usec = None;
                    s.fifo_timer_request_pending = true;
                }
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

                // Bochs serial.cc: break in loopback mode
                let need_break_enq = s.modem_cntl.local_loopback && s.line_cntl.break_cntl;
                let check_dlab = prev_dlab && !s.line_cntl.dlab;
                if need_break_enq {
                    s.line_status.break_int = true;
                    s.line_status.framing_error = true;
                    self.rx_fifo_enq(port_idx, 0x00);
                }

                // When DLAB transitions from 1→0, recalculate baud rate and
                // databyte_usec (Bochs serial.cc).
                if check_dlab {
                    let s = &mut self.ports[port_idx];
                    if let Some((new_baudrate, databyte_usec)) = serial_timing_from_registers(
                        s.divisor_lsb,
                        s.divisor_msb,
                        s.line_cntl.wordlen_sel,
                    ) {
                        if new_baudrate != s.baudrate {
                            s.baudrate = new_baudrate;
                            tracing::trace!(
                                "COM{}: baud rate set to {}",
                                port_idx + 1,
                                new_baudrate
                            );
                        }
                        s.databyte_usec = databyte_usec;
                        tracing::trace!("COM{}: databyte_usec={}", port_idx + 1, databyte_usec);
                    } else {
                        tracing::trace!("COM{}: ignoring invalid baud rate divisor", port_idx + 1);
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

                // Bochs serial.cc: detect loopback transition
                let need_break_enq_mcr =
                    !prev_loopback && s.modem_cntl.local_loopback && s.line_cntl.break_cntl;
                let is_loopback = s.modem_cntl.local_loopback;
                if need_break_enq_mcr {
                    // Transition to loopback mode with break_cntl active
                    // Bochs serial.cc
                    s.line_status.break_int = true;
                    s.line_status.framing_error = true;
                    self.rx_fifo_enq(port_idx, 0x00);
                }

                if is_loopback {
                    let s = &mut self.ports[port_idx];
                    // Bochs serial.cc: Loopback MCR→MSR reflection
                    // Save previous MSR state before updating
                    let prev_cts = s.modem_status.cts;
                    let prev_dsr = s.modem_status.dsr;
                    let prev_ri = s.modem_status.ri;
                    let prev_dcd = s.modem_status.dcd;

                    // RTS → CTS, DTR → DSR, OUT1 → RI, OUT2 → DCD
                    s.modem_status.cts = s.modem_cntl.rts;
                    s.modem_status.dsr = s.modem_cntl.dtr;
                    s.modem_status.ri = s.modem_cntl.out1;
                    s.modem_status.dcd = s.modem_cntl.out2;

                    // Detect changes — set delta bits AND ms_ipending for each
                    if s.modem_status.cts != prev_cts {
                        s.modem_status.delta_cts = true;
                        s.ms_ipending = true;
                    }
                    if s.modem_status.dsr != prev_dsr {
                        s.modem_status.delta_dsr = true;
                        s.ms_ipending = true;
                    }
                    if s.modem_status.ri != prev_ri {
                        s.ms_ipending = true;
                    }
                    if !s.modem_status.ri && prev_ri {
                        s.modem_status.ri_trailedge = true;
                    }
                    if s.modem_status.dcd != prev_dcd {
                        s.modem_status.delta_dcd = true;
                        s.ms_ipending = true;
                    }

                    // Bochs always calls raise_interrupt here (it checks ms_ipending inside)
                    self.raise_interrupt(port_idx, IntSource::ModStat);
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
            }

            REG_MSR => {
                // MSR is read-only. Writes are ignored.
            }

            REG_SCR => {
                self.ports[port_idx].scratch = val;
            }

            _ => {}
        }
    }
}

#[cfg(feature = "std")]
impl BxSerialC {
    /// Encoded byte length of the complete SERIAL v3 section payload.
    pub(crate) fn snapshot_v3_len(&self) -> io::Result<u64> {
        if self.num_ports > self.ports.len() {
            return Err(invalid_serial_snapshot(
                "serial live port count exceeds controller capacity",
            ));
        }
        checked_serial_count(self.num_ports)?;

        let mut len = SERIAL_SNAPSHOT_HEADER_LEN;
        for port in self.ports.iter().take(self.num_ports) {
            len = checked_snapshot_len_add(len, serial_port_snapshot_v3_len(port)?)?;
        }
        Ok(len)
    }

    /// Streams the complete SERIAL v3 section payload without staging a
    /// payload buffer or changing host output/callback wiring.
    pub(crate) fn save_snapshot_v3<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.snapshot_v3_len()?;
        writer.write_u32(SNAPSHOT_SECTION_VERSION)?;
        writer.write_u32(checked_serial_count(self.num_ports)?)?;

        for ((port, pending_irq_raise), pending_irq_lower) in self
            .ports
            .iter()
            .take(self.num_ports)
            .zip(self.pending_irq_raise.iter())
            .zip(self.pending_irq_lower.iter())
        {
            save_serial_port_snapshot_v3(
                port,
                *pending_irq_raise,
                *pending_irq_lower,
                writer,
            )?;
        }
        Ok(())
    }

    /// Decodes the complete SERIAL v3 section payload. Timer owner validation
    /// and derived timing are deliberately deferred to the restore hooks.
    pub(crate) fn restore_snapshot_v3<R: Read>(
        &mut self,
        reader: &mut SnapshotReader<R>,
    ) -> io::Result<()> {
        if reader.read_u32()? != SNAPSHOT_SECTION_VERSION {
            return Err(invalid_serial_snapshot(
                "serial snapshot section version is unsupported",
            ));
        }
        let saved_num_ports = usize::try_from(reader.read_u32()?)
            .map_err(|_| invalid_serial_snapshot("serial port count does not fit usize"))?;
        if saved_num_ports != self.num_ports || saved_num_ports > self.ports.len() {
            return Err(invalid_serial_snapshot(
                "serial configured port count does not match live configuration",
            ));
        }

        let mut slots = self
            .ports
            .iter_mut()
            .take(self.num_ports)
            .zip(self.pending_irq_raise.iter_mut())
            .zip(self.pending_irq_lower.iter_mut());
        for _ in 0..saved_num_ports {
            let ((port, pending_irq_raise), pending_irq_lower) = slots.next().ok_or_else(|| {
                invalid_serial_snapshot("serial live port topology is incomplete")
            })?;
            let snapshot = SerialPortSnapshot::read(reader, port.base, port.irq)?;
            let restored_pending_irq_raise = snapshot.pending_irq_raise;
            let restored_pending_irq_lower = snapshot.pending_irq_lower;
            snapshot.apply_to(port);
            *pending_irq_raise = restored_pending_irq_raise;
            *pending_irq_lower = restored_pending_irq_lower;
        }

        reader.finish_exact()
    }

    /// Validates decoded FIFO-timeout handles after PC_SYSTEM owns have been
    /// restored. The closure must reject non-SerialFifo(port) owners.
    pub(crate) fn validate_snapshot_v3_timer_handles<F>(
        &self,
        mut validate_owner: F,
    ) -> io::Result<()>
    where
        F: FnMut(usize, usize) -> io::Result<()>,
    {
        if self.num_ports > self.ports.len() {
            return Err(invalid_serial_snapshot(
                "serial live port count exceeds controller capacity",
            ));
        }
        for (port_index, port) in self.ports.iter().take(self.num_ports).enumerate() {
            if let Some(handle) = port.fifo_timer_handle {
                validate_owner(port_index, handle)?;
            }
        }
        Ok(())
    }

    /// Rebuilds deterministic UART timing once every section and timer owner
    /// has restored. It does not schedule timers or emit IRQ edges.
    pub(crate) fn after_restore_snapshot_v3(&mut self) -> io::Result<()> {
        if self.num_ports > self.ports.len() {
            return Err(invalid_serial_snapshot(
                "serial live port count exceeds controller capacity",
            ));
        }
        for port in self.ports.iter_mut().take(self.num_ports) {
            let (baudrate, databyte_usec) = serial_timing_from_registers(
                port.divisor_lsb,
                port.divisor_msb,
                port.line_cntl.wordlen_sel,
            )
            .ok_or_else(|| invalid_serial_snapshot("serial divisor or line format is invalid"))?;
            port.baudrate = baudrate;
            port.databyte_usec = databyte_usec;
        }
        Ok(())
    }
}

// ============================================================================
// I/O port handler functions for the device infrastructure
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serial_creation() {
        let serial = BxSerialC::new(1);
        assert_eq!(serial.num_ports, 1);
        assert_eq!(serial.ports[0].base, 0x03F8);
        assert_eq!(serial.ports[0].irq, 4);
        assert_eq!(serial.port_index_for_address(0x03F8), Some(0));
        assert_eq!(serial.port_index_for_address(0x02F8), None);
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
        let mut output = [0u8; 2];
        let mut i = 0;
        for b in serial.drain_tx_output(0) {
            output[i] = b;
            i += 1;
        }
        assert_eq!(i, 2);
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

    #[test]
    fn serial_configured_ports_keep_fifo_owner_handles() {
        let mut serial = BxSerialC::new(4);

        assert_eq!(serial.configured_port_count(), 4);
        for port_index in 0..serial.configured_port_count() {
            assert_eq!(serial.fifo_timer_handle(port_index), None);
            serial.set_fifo_timer_handle(port_index, Some(100 + port_index));
            assert_eq!(
                serial.fifo_timer_handle(port_index),
                Some(100 + port_index)
            );
        }
    }

    fn fifo_serial_with_four_byte_trigger() -> BxSerialC {
        let mut serial = BxSerialC::new(1);
        serial.write(COM_BASES[0] + REG_IIR_FCR, 0x41, 1);
        serial.write(COM_BASES[0] + REG_MCR, 0x08, 1);
        serial.write(COM_BASES[0] + REG_IER_DLM, 0x01, 1);
        let _ = serial.take_pending_irqs().count();
        serial
    }

    #[test]
    fn serial_fifo_timeout_uses_three_character_deadline() {
        let mut serial = fifo_serial_with_four_byte_trigger();

        serial.receive_byte(0, 0x11);

        assert_eq!(serial.fifo_timeout_delay_usec(0), Some(3 * 87));
        assert_eq!(serial.read(COM_BASES[0] + REG_IIR_FCR, 1) & 0x01, 0x01);

        let _ = serial.take_pending_irqs().count();
        assert!(serial.fifo_timer_fired(0));
        let mut irqs = serial.take_pending_irqs();
        assert_eq!(irqs.next(), Some((COM_IRQS[0], true)));
        assert_eq!(irqs.next(), None);
        assert_eq!(serial.fifo_timeout_delay_usec(0), None);
        assert_eq!(
            serial.read(COM_BASES[0] + REG_IIR_FCR, 1) & 0x0e,
            0x0c
        );
    }

    #[test]
    fn serial_fifo_byte_rearms_three_character_timeout() {
        let mut serial = fifo_serial_with_four_byte_trigger();

        serial.receive_byte(0, 0x11);
        assert_eq!(serial.fifo_timeout_delay_usec(0), Some(3 * 87));

        serial.receive_byte(0, 0x22);
        assert_eq!(serial.fifo_timeout_delay_usec(0), Some(3 * 87));
        assert_eq!(serial.read(COM_BASES[0] + REG_IIR_FCR, 1) & 0x01, 0x01);
    }

    #[test]
    fn serial_fifo_trigger_cancels_timeout() {
        let mut serial = fifo_serial_with_four_byte_trigger();

        for byte in 0..4 {
            serial.receive_byte(0, byte);
        }

        assert_eq!(serial.fifo_timeout_delay_usec(0), None);
        assert!(!serial.fifo_timer_fired(0));
        assert_eq!(
            serial.read(COM_BASES[0] + REG_IIR_FCR, 1) & 0x0e,
            0x04
        );
    }

    #[test]
    fn serial_fifo_drain_cancels_timeout() {
        let mut serial = fifo_serial_with_four_byte_trigger();

        serial.receive_byte(0, 0x11);
        assert_eq!(serial.fifo_timeout_delay_usec(0), Some(3 * 87));

        assert_eq!(serial.read(COM_BASES[0] + REG_RBR_THR, 1), 0x11);
        assert_eq!(serial.fifo_timeout_delay_usec(0), None);
        assert!(!serial.fifo_timer_fired(0));
        assert_eq!(serial.read(COM_BASES[0] + REG_IIR_FCR, 1) & 0x01, 0x01);
    }

    #[cfg(feature = "std")]
    #[test]
    fn serial_snapshot_resumes_partial_fifo_and_irq_state() {
        let mut serial = BxSerialC::new(1);
        let base = COM_BASES[0];

        serial.set_fifo_timer_handle(0, Some(73));
        serial.write(base + REG_LCR, 0x9b, 1);
        serial.write(base + REG_RBR_THR, 12, 1);
        serial.write(base + REG_IER_DLM, 0, 1);
        serial.write(base + REG_LCR, 0x1b, 1);
        serial.write(base + REG_IIR_FCR, 0x41, 1);
        serial.write(base + REG_MCR, 0x0b, 1);
        serial.write(base + REG_IER_DLM, 0x0f, 1);
        assert_eq!(serial.read(base + REG_IIR_FCR, 1) & 0x0e, 0x02);

        serial.receive_byte(0, 0x10);
        serial.receive_byte(0, 0x11);
        serial.receive_byte(0, 0x12);
        assert_eq!(serial.read(base + REG_RBR_THR, 1), 0x10);
        serial.receive_byte(0, 0x13);

        serial.write(base + REG_RBR_THR, b'a' as u32, 1);
        serial.write(base + REG_RBR_THR, b'b' as u32, 1);
        serial.write(base + REG_RBR_THR, b'c' as u32, 1);
        assert_eq!(serial.ports[0].tx_output.pop_front(), Some(b'a'));
        serial.write(base + REG_RBR_THR, b'd' as u32, 1);

        let port = &mut serial.ports[0];
        port.tx_fifo.push_back(0x20);
        port.tx_fifo.push_back(0x21);
        port.tx_fifo.push_back(0x22);
        assert_eq!(port.tx_fifo.pop_front(), Some(0x20));
        port.tx_fifo.push_back(0x23);

        assert_eq!(serial.fifo_timeout_delay_usec(0), Some(3_123));
        assert!(serial.pending_irq_raise[0]);
        assert!(serial.pending_irq_lower[0]);

        let mut saved = Vec::new();
        serial.save_snapshot_v3(&mut saved).unwrap();
        assert_eq!(saved.len() as u64, serial.snapshot_v3_len().unwrap());

        serial.reset();
        serial.write(base + REG_SCR, 0xff, 1);
        serial.receive_byte(0, 0xee);

        let mut reader = SnapshotReader::new(saved.as_slice(), saved.len() as u64).unwrap();
        serial.restore_snapshot_v3(&mut reader).unwrap();
        serial.after_restore_snapshot_v3().unwrap();

        assert_eq!(serial.fifo_timer_handle(0), Some(73));
        assert_eq!(serial.ports[0].baudrate, 9_600);
        assert_eq!(serial.ports[0].databyte_usec, 1_041);
        assert_eq!(serial.read(base + REG_LCR, 1), 0x1b);
        assert_eq!(serial.read(base + REG_IER_DLM, 1), 0x0f);
        assert_eq!(serial.read(base + REG_MCR, 1), 0x0b);
        assert_eq!(serial.read(base + REG_SCR, 1), 0);
        assert_eq!(
            serial.ports[0].tx_fifo.iter().collect::<Vec<_>>(),
            vec![0x21, 0x22, 0x23]
        );
        assert_eq!(serial.fifo_timeout_delay_usec(0), Some(3_123));
        assert_eq!(serial.take_fifo_timer_update(0), Some(Some(3_123)));

        assert_eq!(serial.read(base + REG_RBR_THR, 1), 0x11);
        assert_eq!(serial.read(base + REG_RBR_THR, 1), 0x12);
        assert_eq!(serial.read(base + REG_RBR_THR, 1), 0x13);
        assert_eq!(serial.fifo_timeout_delay_usec(0), None);
        assert_eq!(
            serial.drain_tx_output(0).collect::<Vec<_>>(),
            vec![b'b', b'c', b'd']
        );
        assert_eq!(
            serial.take_pending_irqs().collect::<Vec<_>>(),
            vec![(COM_IRQS[0], true), (COM_IRQS[0], false)]
        );
    }
}
