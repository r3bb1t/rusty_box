//! Arm PrimeCell PL011 UART.
//!
//! Provenance (R7): the PrimeCell UART (PL011) Technical Reference Manual,
//! ARM DDI 0183F, first; QEMU 11.1.2 `hw/char/pl011.c` second. The
//! peripheral ID is QEMU's `pl011_id_arm`, whose `UARTPeriphID2` revision
//! field (bits [7:4]) is 1: a PL011 of the revisions DDI 0183F describes
//! (up to r1p4), whose receive FIFO is 16 entries of 12 bits (§2.2.5).
//!
//! From DDI 0183F:
//!
//! - Register widths and reset values (Table 3-1). A reserved bit reads as
//!   zero (Tables 3-4, 3-5, 3-10, 3-12, 3-13, 3-14, 3-18); `UARTRSR` bits
//!   [7:4], unpredictable, read as zero too.
//! - Reception needs `UARTCR.UARTEN` and `UARTCR.RXE` (§3.3.8, Note). The
//!   host line is flow-controlled, as a QEMU character backend is: while
//!   the receiver is off, or its FIFO full, the host's bytes wait for it
//!   rather than being lost. In loopback (§2.4.3, "System and diagnostic
//!   loopback testing", and `UARTCR.LBE`) the receiver hears the
//!   transmitter, and a character it is off for is gone.
//! - The receive FIFO (§2.2.5, §2.4.3): 16 entries while `UARTLCR_H.FEN` is
//!   set, one (the holding register) while it is clear. A character that
//!   arrives while it is full is lost and sets the overrun state
//!   (§2.4.3, "Overrun bit"): `UARTRSR.OE` at once (§3.3.2), `OERIS`
//!   (§3.4.5), and `UARTDR.OE` on the next character written to the FIFO,
//!   which clears the state (Table 3-2). A break loads one 0 character with
//!   `BE` set and raises `BERIS` (Table 3-2, §3.4.5).
//! - `UARTRSR` takes the break, parity and framing bits of the character a
//!   `UARTDR` read returns; its overrun bit is set when the overrun happens
//!   and only `UARTECR` clears it (§3.3.2).
//! - `RXRIS` (§3.4.2, §3.3.9): with the FIFO enabled, set when the fill
//!   level rises to the `UARTIFLS.RXIFLSEL` level and cleared once reads
//!   take it below; with the FIFO disabled, set when the holding register
//!   fills and cleared by a read. `UARTICR` clears it either way.
//! - `RTRIS` (§3.4.4, Table 3-15 note a): set when the FIFO is not empty
//!   and nothing has arrived for 32 bit periods, and only while `RTIM` is
//!   set; cleared when reads empty the FIFO, or by `UARTICR`.
//! - The modem status interrupts (§3.4.1) are set by a change of their
//!   line and cleared only by `UARTICR`.
//!
//! Time: this model's bit period is zero. A character written to `UARTDR`
//! has been transmitted by the time the write completes (the transmit FIFO
//! always reads empty and `BUSY` clear, and `TXRIS` is raised after every
//! write, as QEMU raises it), and the host line delivers its bytes the
//! moment the receiver has room. So 32 bit periods without a character
//! have passed as soon as a delivery ends: a character's arrival raises
//! `RTRIS` while `RTIM` is set, and setting `RTIM` raises it while the FIFO
//! holds anything. A timed line would need the instant of the last arrival
//! as state.
//!
//! From QEMU 11.1.2 where DDI 0183F leaves the choice to the system: the
//! flag register QEMU keeps as state, `UARTDR` reads of an empty FIFO
//! (`pl011_read_rxdata`: the slot at the read position), the FIFO emptied
//! whenever `FEN` changes (`pl011_write`), loopback driving the modem
//! status inputs from the modem control outputs and leaving them where they
//! were when loopback ends (`pl011_loopback_mdmctrl`), a loopback break on
//! each setting of `UARTLCR_H.BRK` (`pl011_loopback_break`), and a host
//! line that sends nothing in loopback (`pl011_receive`). The host side of
//! a break has no backend here: the line neither sends nor delivers one.
//! Transmission does not wait for `UARTEN` and `TXE` (`pl011_write_txdata`);
//! README's "Where the references differ" says why.
//!
//! The region takes 4-byte accesses (`pl011_ops`' `impl.min_access_size` and
//! `impl.max_access_size`): QEMU's `access_with_adjusted_size` hands a
//! narrower access to the register at `offset >> 2` with the value
//! unshifted, and splits an 8-byte one into two, at `offset` and then
//! `offset + 4`.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::{RingBuffer, SnapError};

use crate::bus::{AccessWidth, RegOffset};

const PL011_ID: [u8; 8] = [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const FLAG_RI: u32 = 0x100;
const FLAG_TXFE: u32 = 0x80;
const FLAG_RXFF: u32 = 0x40;
const FLAG_TXFF: u32 = 0x20;
const FLAG_RXFE: u32 = 0x10;
const FLAG_DCD: u32 = 0x04;
const FLAG_DSR: u32 = 0x02;
const FLAG_CTS: u32 = 0x01;
/// The modem status inputs in `UARTFR`.
const FLAG_MODEM: u32 = FLAG_RI | FLAG_DCD | FLAG_DSR | FLAG_CTS;
/// `UARTFR`'s nine bits (Table 3-4).
const FR_MASK: u32 = 0x1FF;

/// `UARTDR.OE`: the overrun state, copied into the next character written
/// to the FIFO (Table 2-1, bit 11).
const DR_OE: u32 = 1 << 11;
/// `UARTDR.BE`: the entry a break puts in the receive FIFO.
const DR_BE: u32 = 1 << 10;

/// `UARTRSR.OE`.
const RSR_OE: u32 = 1 << 3;
/// `UARTRSR.BE`, `PE` and `FE`: the status of the character last read
/// from `UARTDR` (§3.3.2).
const RSR_CHARACTER_ERRORS: u32 = 0x7;

const INT_OE: u32 = 1 << 10;
const INT_BE: u32 = 1 << 9;
const INT_RT: u32 = 1 << 6;
const INT_TX: u32 = 1 << 5;
const INT_RX: u32 = 1 << 4;
const INT_DSR: u32 = 1 << 3;
const INT_DCD: u32 = 1 << 2;
const INT_CTS: u32 = 1 << 1;
const INT_RI: u32 = 1 << 0;

/// `UARTLCR_H.FEN`: the FIFOs are enabled.
const LCR_FEN: u32 = 1 << 4;
/// `UARTLCR_H.BRK`: send a break.
const LCR_BRK: u32 = 1 << 0;

const CR_OUT2: u32 = 1 << 13;
const CR_OUT1: u32 = 1 << 12;
const CR_RTS: u32 = 1 << 11;
const CR_DTR: u32 = 1 << 10;
/// `UARTCR.RXE`: the receive section is enabled.
const CR_RXE: u32 = 1 << 9;
/// `UARTCR.LBE`: loopback.
const CR_LBE: u32 = 1 << 7;
/// `UARTCR.UARTEN`: the UART is enabled.
const CR_UARTEN: u32 = 1 << 0;

/// The bits each read/write register keeps; the rest read as zero
/// (Table 3-1's widths, and bits [6:3] of `UARTCR`, Table 3-12).
const ILPR_MASK: u32 = 0xFF;
const IBRD_MASK: u32 = 0xFFFF;
const FBRD_MASK: u32 = 0x3F;
const LCR_MASK: u32 = 0xFF;
const CR_MASK: u32 = 0xFF87;
const IFLS_MASK: u32 = 0x3F;
const IMSC_MASK: u32 = 0x7FF;
const DMACR_MASK: u32 = 0x7;

/// The receive FIFO's depth (DDI 0183F §2.2.5; QEMU `PL011_FIFO_DEPTH`).
pub const RX_FIFO_DEPTH: usize = 16;
const _: () = assert!(RX_FIFO_DEPTH.is_power_of_two(), "slots are found by masking");

/// The overrun state (DDI 0183F §2.4.3, "Overrun bit"): a character was
/// lost to a full FIFO, and none has been written to it since. It is kept
/// in bit 31 of the flag register's word, which `UARTFR` does not have (a
/// read returns its nine bits), so a snapshot carries it in the word it
/// always wrote, zero for a UART that never overran.
const OVERRUN_PENDING: u32 = 1 << 31;

/// Bytes the guest has transmitted and the host has not yet drained. A host
/// that never drains loses the oldest bytes, never stalls the guest.
pub const TX_CAPACITY: usize = 64 * 1024;

pub struct Pl011 {
    tx: RingBuffer<u8, TX_CAPACITY>,
    /// QEMU `read_fifo`: each entry is a `UARTDR` value, a received byte or
    /// [`DR_BE`], with [`DR_OE`] when an overrun came before it.
    read_fifo: [u32; RX_FIFO_DEPTH],
    /// Below [`RX_FIFO_DEPTH`], and zero while the FIFO is disabled.
    read_pos: usize,
    /// Entries queued: at most the FIFO's depth.
    read_count: u32,
    /// `UARTFR`, and [`OVERRUN_PENDING`].
    flags: u32,
    rsr: u32,
    ilpr: u32,
    ibrd: u32,
    fbrd: u32,
    lcr: u32,
    cr: u32,
    ifl: u32,
    int_enabled: u32,
    int_level: u32,
    dmacr: u32,
}

impl Default for Pl011 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pl011 {
    /// QEMU `pl011_reset`, on a device whose receive FIFO has never been
    /// written: DDI 0183F Table 3-1's reset values.
    pub fn new() -> Self {
        Self {
            tx: RingBuffer::new(),
            read_fifo: [0; RX_FIFO_DEPTH],
            read_pos: 0,
            read_count: 0,
            flags: FLAG_RXFE | FLAG_TXFE,
            rsr: 0,
            ilpr: 0,
            ibrd: 0,
            fbrd: 0,
            lcr: 0,
            cr: 0x300,
            ifl: 0x12,
            int_enabled: 0,
            int_level: 0,
            dmacr: 0,
        }
    }

    /// QEMU `pl011_reset`: the registers and the FIFO bookkeeping return to
    /// their reset state; the FIFO's slots keep what they held, as QEMU's
    /// `read_fifo` does. Bytes already transmitted stay for the host: they
    /// left the device before the reset.
    pub fn reset(&mut self) {
        let tx = core::mem::take(&mut self.tx);
        let read_fifo = self.read_fifo;
        *self = Self {
            tx,
            read_fifo,
            ..Self::new()
        };
    }

    /// The interrupt line (DDI 0183F §3.4.6 `UARTINTR`).
    pub fn irq(&self) -> bool {
        self.int_level & self.int_enabled != 0
    }

    /// Transmitted bytes waiting for the host.
    pub fn tx_len(&self) -> usize {
        self.tx.len()
    }

    /// Take the transmitted bytes, oldest first.
    pub fn drain_tx(&mut self) -> rusty_box_core::ring_buffer::Drain<'_, u8, TX_CAPACITY> {
        self.tx.drain()
    }

    /// QEMU `pl011_get_fifo_depth`.
    fn fifo_depth(&self) -> u32 {
        if self.lcr & LCR_FEN != 0 {
            RX_FIFO_DEPTH as u32
        } else {
            1
        }
    }

    /// The fill level whose crossing raises `RXRIS` (DDI 0183F §3.4.2,
    /// Table 3-13): one character with the FIFO disabled, else the eighths
    /// `UARTIFLS.RXIFLSEL` selects. The reserved encodings 0b101–0b111
    /// select the half-full level, as reset does.
    fn rx_trigger(&self) -> u32 {
        if self.lcr & LCR_FEN == 0 {
            return 1;
        }
        let eighths = match (self.ifl >> 3) & 7 {
            0 => 1,
            1 => 2,
            3 => 6,
            4 => 7,
            _ => 4,
        };
        RX_FIFO_DEPTH as u32 * eighths / 8
    }

    fn loopback(&self) -> bool {
        self.cr & CR_LBE != 0
    }

    /// DDI 0183F §3.3.8, Note: reception needs `UARTEN` and `RXE`.
    fn receiving(&self) -> bool {
        self.cr & (CR_UARTEN | CR_RXE) == CR_UARTEN | CR_RXE
    }

    /// Whether the receive side takes a host byte.
    pub fn can_receive(&self) -> bool {
        self.rx_room() > 0
    }

    /// How many host bytes the receive side takes now: none while the
    /// receiver is off (DDI 0183F §3.3.8), else the FIFO's free entries,
    /// QEMU `pl011_can_receive`'s `fifo_depth - read_count`. Loopback
    /// disconnects the host line, which QEMU still lets fill that room with
    /// bytes it drops.
    pub fn rx_room(&self) -> usize {
        if !self.loopback() && !self.receiving() {
            return 0;
        }
        self.fifo_depth().saturating_sub(self.read_count) as usize
    }

    /// QEMU `pl011_receive` for one host byte [`Pl011::can_receive`] said
    /// there was room for: received, or consumed and dropped while loopback
    /// disconnects the receive input. Returns false, taking nothing, when
    /// there is no room.
    pub fn receive(&mut self, byte: u8) -> bool {
        if !self.can_receive() {
            return false;
        }
        if !self.loopback() {
            self.receive_character(u32::from(byte));
        }
        true
    }

    /// The receive logic taking one character off the line, the receiver
    /// enabled (DDI 0183F §2.4.3; QEMU `pl011_fifo_rx_put`). A full FIFO
    /// keeps its entries and the character is lost to an overrun.
    fn receive_character(&mut self, value: u32) {
        if value & DR_BE != 0 {
            self.int_level |= INT_BE;
        }
        let depth = self.fifo_depth();
        if self.read_count == depth {
            self.flags |= OVERRUN_PENDING;
            self.rsr |= RSR_OE;
            self.int_level |= INT_OE;
            return;
        }
        let entry = if self.flags & OVERRUN_PENDING != 0 { value | DR_OE } else { value };
        self.flags &= !OVERRUN_PENDING;
        let slot = (self.read_pos as u32 + self.read_count) & (depth - 1);
        self.read_fifo[slot as usize] = entry;
        self.read_count += 1;
        self.flags &= !FLAG_RXFE;
        if self.read_count == depth {
            self.flags |= FLAG_RXFF;
        }
        if self.read_count == self.rx_trigger() {
            self.int_level |= INT_RX;
        }
        if self.int_enabled & INT_RT != 0 {
            self.int_level |= INT_RT;
        }
    }

    /// QEMU `pl011_loopback_tx`: in loopback the receiver hears what is
    /// transmitted, when it is enabled.
    fn loopback_tx(&mut self, value: u32) {
        if self.loopback() && self.receiving() {
            self.receive_character(value);
        }
    }

    /// QEMU `pl011_read_rxdata`. An empty FIFO returns the slot at the read
    /// position, whatever it last held.
    fn read_rxdata(&mut self) -> u32 {
        let depth = self.fifo_depth();
        self.flags &= !FLAG_RXFF;
        let c = self.read_fifo[self.read_pos];
        if self.read_count > 0 {
            self.read_count -= 1;
            self.read_pos = (self.read_pos + 1) & (depth as usize - 1);
        }
        if self.read_count == 0 {
            self.flags |= FLAG_RXFE;
            self.int_level &= !INT_RT;
        }
        if self.read_count < self.rx_trigger() {
            self.int_level &= !INT_RX;
        }
        self.rsr = (self.rsr & RSR_OE) | ((c >> 8) & RSR_CHARACTER_ERRORS);
        c
    }

    /// QEMU `pl011_reset_rx_fifo` and `pl011_reset_tx_fifo`.
    fn reset_fifos(&mut self) {
        self.read_count = 0;
        self.read_pos = 0;
        self.flags &= !(FLAG_RXFF | FLAG_TXFF);
        self.flags |= FLAG_RXFE | FLAG_TXFE;
    }

    /// QEMU `pl011_loopback_mdmctrl`: in loopback the modem control outputs
    /// drive the modem status inputs. Each input that changes raises its
    /// interrupt (DDI 0183F §3.4.1). Out of loopback the inputs keep
    /// whatever loopback last set.
    fn loopback_mdmctrl(&mut self) {
        if !self.loopback() {
            return;
        }
        let cr = self.cr;
        let mut status = 0;
        for (control, input) in [
            (CR_OUT2, FLAG_RI),
            (CR_OUT1, FLAG_DCD),
            (CR_RTS, FLAG_CTS),
            (CR_DTR, FLAG_DSR),
        ] {
            if cr & control != 0 {
                status |= input;
            }
        }
        let changed = (self.flags ^ status) & FLAG_MODEM;
        for (input, int) in [
            (FLAG_DSR, INT_DSR),
            (FLAG_DCD, INT_DCD),
            (FLAG_CTS, INT_CTS),
            (FLAG_RI, INT_RI),
        ] {
            if changed & input != 0 {
                self.int_level |= int;
            }
        }
        self.flags = (self.flags & !FLAG_MODEM) | status;
    }

    /// A `UARTIMSC` write. `RTIM` enables the receive timeout as well as
    /// masking it (DDI 0183F Table 3-15, note a): clearing it clears
    /// `RTRIS`, and setting it starts the timeout, which with the FIFO
    /// holding anything has expired at once (see the module's "Time").
    fn set_interrupt_mask(&mut self, v: u32) {
        let mask = v & IMSC_MASK;
        if mask & INT_RT == 0 {
            self.int_level &= !INT_RT;
        } else if self.int_enabled & INT_RT == 0 && self.read_count > 0 {
            self.int_level |= INT_RT;
        }
        self.int_enabled = mask;
    }

    pub fn read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        match width {
            AccessWidth::Dword => {
                let low = self.read_register(offset.get());
                let high = self.read_register(offset.get() + 4);
                u64::from(low) | (u64::from(high) << 32)
            }
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => u64::from(self.read_register(offset.get())),
        }
    }

    /// QEMU `pl011_read`.
    fn read_register(&mut self, off: u64) -> u32 {
        match off >> 2 {
            0 => self.read_rxdata(),
            1 => self.rsr,
            6 => self.flags & FR_MASK,
            8 => self.ilpr,
            9 => self.ibrd,
            10 => self.fbrd,
            11 => self.lcr,
            12 => self.cr,
            13 => self.ifl,
            14 => self.int_enabled,
            15 => self.int_level,
            16 => self.int_level & self.int_enabled,
            18 => self.dmacr,
            0x3F8..=0x3FF => u32::from(PL011_ID[((off - 0xFE0) >> 2) as usize & 7]),
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: RegOffset, width: AccessWidth, value: u64) {
        match width {
            AccessWidth::Dword => {
                self.write_register(offset.get(), value as u32);
                self.write_register(offset.get() + 4, (value >> 32) as u32);
            }
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => {
                self.write_register(offset.get(), value as u32);
            }
        }
    }

    /// QEMU `pl011_write`.
    fn write_register(&mut self, off: u64, v: u32) {
        match off >> 2 {
            0 => {
                // QEMU `pl011_write_txdata`.
                let byte = v as u8;
                self.tx.push_back(byte);
                self.loopback_tx(u32::from(byte));
                self.int_level |= INT_TX;
            }
            1 => self.rsr = 0,
            8 => self.ilpr = v & ILPR_MASK,
            9 => self.ibrd = v & IBRD_MASK,
            10 => self.fbrd = v & FBRD_MASK,
            11 => {
                let v = v & LCR_MASK;
                if (self.lcr ^ v) & LCR_FEN != 0 {
                    self.reset_fifos();
                }
                // QEMU `pl011_loopback_break`; the host side of a break
                // (`CHR_IOCTL_SERIAL_SET_BREAK`) has no backend here.
                if (self.lcr ^ v) & LCR_BRK != 0 && v & LCR_BRK != 0 {
                    self.loopback_tx(DR_BE);
                }
                self.lcr = v;
            }
            12 => {
                self.cr = v & CR_MASK;
                self.loopback_mdmctrl();
            }
            13 => self.ifl = v & IFLS_MASK,
            14 => self.set_interrupt_mask(v),
            17 => self.int_level &= !v,
            18 => self.dmacr = v & DMACR_MASK,
            _ => {}
        }
    }

    /// The registers and the receive FIFO, for a snapshot. Bytes waiting in
    /// the transmit buffer belong to the host and are not part of it.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for reg in [
            self.flags,
            self.rsr,
            self.ilpr,
            self.ibrd,
            self.fbrd,
            self.lcr,
            self.cr,
            self.ifl,
            self.int_enabled,
            self.int_level,
            self.dmacr,
        ] {
            w.write_u32(reg)?;
        }
        for entry in self.read_fifo {
            w.write_u32(entry)?;
        }
        w.write_u8(self.read_pos as u8)?;
        w.write_u32(self.read_count)
    }

    /// Refuses a read position outside the FIFO, and a nonzero one while the
    /// FIFO is disabled (QEMU `pl011_post_load`'s checks), and more entries
    /// than the FIFO holds. Reserved register bits are dropped, as a guest
    /// write drops them.
    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut regs = [0u32; 11];
        for reg in regs.iter_mut() {
            *reg = r.read_u32()?;
        }
        let [flags, rsr, ilpr, ibrd, fbrd, lcr, cr, ifl, int_enabled, int_level, dmacr] = regs;
        let mut read_fifo = [0u32; RX_FIFO_DEPTH];
        for entry in read_fifo.iter_mut() {
            *entry = r.read_u32()?;
        }
        let read_pos = usize::from(r.read_u8()?);
        let read_count = r.read_u32()?;
        if read_pos >= RX_FIFO_DEPTH {
            return Err(SnapError::Invalid("PL011 read position outside the receive FIFO"));
        }
        let depth = if lcr & LCR_FEN != 0 { RX_FIFO_DEPTH as u32 } else { 1 };
        if depth == 1 && read_pos != 0 {
            return Err(SnapError::Invalid("PL011 read position past the one-entry receive holding register"));
        }
        if read_count > depth {
            return Err(SnapError::Invalid("PL011 receive count beyond the receive FIFO's depth"));
        }
        let tx = core::mem::take(&mut self.tx);
        *self = Self {
            tx,
            read_fifo,
            read_pos,
            read_count,
            flags: flags & (FR_MASK | OVERRUN_PENDING),
            rsr: rsr & (RSR_OE | RSR_CHARACTER_ERRORS),
            ilpr: ilpr & ILPR_MASK,
            ibrd: ibrd & IBRD_MASK,
            fbrd: fbrd & FBRD_MASK,
            lcr: lcr & LCR_MASK,
            cr: cr & CR_MASK,
            ifl: ifl & IFLS_MASK,
            int_enabled: int_enabled & IMSC_MASK,
            int_level: int_level & IMSC_MASK,
            dmacr: dmacr & DMACR_MASK,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(n: u64) -> RegOffset {
        RegOffset::new(n * 4)
    }

    fn read(uart: &mut Pl011, n: u64) -> u32 {
        uart.read(reg(n), AccessWidth::Word) as u32
    }

    fn write(uart: &mut Pl011, n: u64, value: u32) {
        uart.write(reg(n), AccessWidth::Word, u64::from(value));
    }

    const DR: u64 = 0;
    const RSR: u64 = 1;
    const FR: u64 = 6;
    const LCR_H: u64 = 11;
    const CR: u64 = 12;
    const IFLS: u64 = 13;
    const IMSC: u64 = 14;
    const RIS: u64 = 15;
    const MIS: u64 = 16;
    const ICR: u64 = 17;

    /// `UARTCR` with the UART, its transmitter and its receiver enabled.
    const ENABLED: u32 = 0x300 | CR_UARTEN;

    /// A UART a driver has switched on.
    fn enabled() -> Pl011 {
        let mut uart = Pl011::new();
        write(&mut uart, CR, ENABLED);
        uart
    }

    #[test]
    fn the_receiver_takes_nothing_until_uarten_and_rxe_are_set() {
        let mut uart = Pl011::new();
        assert_eq!(read(&mut uart, CR), 0x300, "reset leaves UARTEN clear");
        for cr in [0x300, CR_UARTEN | 0x100, CR_RXE] {
            write(&mut uart, CR, cr);
            assert!(!uart.can_receive(), "UARTCR {cr:#x}");
            assert_eq!(uart.rx_room(), 0);
            assert!(!uart.receive(b'a'), "the host's byte waits");
            assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
        }
        write(&mut uart, CR, CR_UARTEN | CR_RXE);
        assert_eq!(uart.rx_room(), 1);
        assert!(uart.receive(b'a'));
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
    }

    #[test]
    fn a_loopback_receiver_that_is_off_loses_what_is_transmitted() {
        let mut uart = Pl011::new();
        write(&mut uart, CR, 0x300 | CR_LBE);
        write(&mut uart, DR, u32::from(b'x'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "UARTEN is clear");
        write(&mut uart, CR, CR_UARTEN | 0x100 | CR_LBE);
        write(&mut uart, DR, u32::from(b'y'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "RXE is clear");
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'z'));
        assert_eq!(read(&mut uart, DR), u32::from(b'z'));
        assert_eq!(uart.drain_tx().collect::<std::vec::Vec<u8>>(), b"xyz", "each byte still left");
    }

    #[test]
    fn with_the_fifo_disabled_the_receiver_holds_one_byte() {
        let mut uart = enabled();
        assert!(uart.receive(b'a'));
        assert!(!uart.can_receive());
        assert!(!uart.receive(b'b'), "a second byte has nowhere to go");
        assert_eq!(read(&mut uart, FR) & FLAG_RXFF, FLAG_RXFF);
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn the_receive_interrupt_rises_at_the_trigger_level_and_falls_below_it() {
        // RXIFLSEL: 1/8, 1/4, 1/2, 3/4 and 7/8 of 16 entries; reserved
        // encodings take the half-full level.
        for (rxiflsel, level) in [(0, 2), (1, 4), (2, 8), (3, 12), (4, 14), (5, 8), (7, 8)] {
            let mut uart = enabled();
            write(&mut uart, LCR_H, LCR_FEN);
            write(&mut uart, IFLS, rxiflsel << 3);
            write(&mut uart, IMSC, INT_RX);
            for i in 0..level - 1 {
                assert!(uart.receive(i as u8));
            }
            assert_eq!(read(&mut uart, RIS) & INT_RX, 0, "RXIFLSEL {rxiflsel}: below the level");
            assert!(uart.receive(0xAA));
            assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX, "RXIFLSEL {rxiflsel}: at the level");
            assert!(uart.irq());
            assert_eq!(read(&mut uart, DR), 0);
            assert_eq!(read(&mut uart, RIS) & INT_RX, 0, "RXIFLSEL {rxiflsel}: a read takes it below");
            assert!(!uart.irq());
        }
    }

    #[test]
    fn the_receive_interrupt_marks_a_crossing_of_the_level() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IFLS, 0); // 1/8: two entries
        for byte in [1, 2] {
            assert!(uart.receive(byte));
        }
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX);
        write(&mut uart, ICR, INT_RX);
        assert!(uart.receive(3));
        assert_eq!(read(&mut uart, RIS) & INT_RX, 0, "filling further crosses nothing");
        // Back below the level, then up through it again.
        assert_eq!(read(&mut uart, DR), 1);
        assert_eq!(read(&mut uart, DR), 2);
        assert!(uart.receive(4));
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX);
    }

    #[test]
    fn with_the_fifo_disabled_each_character_raises_the_receive_interrupt() {
        let mut uart = enabled();
        write(&mut uart, IFLS, 4 << 3);
        assert!(uart.receive(b'a'));
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX, "UARTIFLS has no say");
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(read(&mut uart, RIS) & INT_RX, 0, "a single read clears it");
        assert!(uart.receive(b'b'));
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX);
    }

    #[test]
    fn the_receive_timeout_follows_an_arrival_while_rtim_enables_it() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        assert!(uart.receive(b'a'));
        assert_eq!(read(&mut uart, RIS) & (INT_RT | INT_RX), 0, "RTIM clear: RTRIS cannot be set");
        write(&mut uart, IMSC, INT_RT);
        assert_eq!(read(&mut uart, RIS) & INT_RT, INT_RT, "enabling it with data waiting");
        assert_eq!(read(&mut uart, MIS), INT_RT);
        assert!(uart.irq());
        write(&mut uart, ICR, INT_RT);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0);
        write(&mut uart, IMSC, INT_RT);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0, "nothing new arrived");
        assert!(uart.receive(b'b'));
        assert_eq!(read(&mut uart, RIS) & INT_RT, INT_RT, "an arrival restarts it");
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(read(&mut uart, RIS) & INT_RT, INT_RT, "the FIFO still holds a byte");
        assert_eq!(read(&mut uart, DR), u32::from(b'b'));
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0, "reading it empty clears it");
        assert!(uart.receive(b'c'));
        write(&mut uart, IMSC, 0);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0, "clearing RTIM clears it");
    }

    #[test]
    fn changing_fen_empties_the_receive_fifo() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        assert!(uart.receive(1));
        assert!(uart.receive(2));
        write(&mut uart, LCR_H, 0);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn an_empty_fifo_reads_the_slot_at_the_read_position() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        assert!(uart.receive(b'p'));
        assert_eq!(read(&mut uart, DR), u32::from(b'p'));
        assert!(uart.receive(b'q'));
        assert_eq!(read(&mut uart, DR), u32::from(b'q'));
        // The read position is now slot 2, which nothing has written.
        assert_eq!(read(&mut uart, DR), 0);
        write(&mut uart, LCR_H, 0);
        // FIFO off: one slot, slot 0, which still holds the first byte.
        assert_eq!(read(&mut uart, DR), u32::from(b'p'));
    }

    #[test]
    fn reserved_register_bits_read_as_zero() {
        let mut uart = Pl011::new();
        for (n, width) in [
            (8, ILPR_MASK),
            (9, IBRD_MASK),
            (10, FBRD_MASK),
            (11, LCR_MASK),
            (12, CR_MASK),
            (13, IFLS_MASK),
            (14, IMSC_MASK),
            (18, DMACR_MASK),
        ] {
            write(&mut uart, n, 0xFFFF_FFFF);
            assert_eq!(read(&mut uart, n), width, "register {:#x}", n * 4);
        }
        assert_eq!(CR_MASK, 0xFF87, "UARTCR bits [6:3] are reserved");
        write(&mut uart, ICR, 0xFFFF_FFFF);
        write(&mut uart, LCR_H, 0);
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, DR), DR_BE);
        assert_eq!(read(&mut uart, RSR), DR_BE >> 8, "UARTRSR keeps four bits");
        assert_eq!(read(&mut uart, RIS) & !IMSC_MASK, 0);
        write(&mut uart, FR, 0);
        assert_eq!(read(&mut uart, FR) & !FR_MASK, 0, "the flag register ignores writes");
    }

    #[test]
    fn loopback_returns_transmitted_bytes_and_drops_host_input() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'L'));
        assert_eq!(uart.drain_tx().collect::<std::vec::Vec<u8>>(), b"L", "the byte still leaves");
        assert!(uart.receive(b'h'), "host input is consumed");
        assert_eq!(read(&mut uart, DR), u32::from(b'L'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "and dropped");
        write(&mut uart, CR, ENABLED);
        assert!(uart.receive(b'h'));
        assert_eq!(read(&mut uart, DR), u32::from(b'h'));
    }

    #[test]
    fn a_full_fifo_keeps_its_entries_and_flags_the_overrun() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, IMSC, INT_OE);
        for i in 0..RX_FIFO_DEPTH as u32 {
            write(&mut uart, DR, i);
        }
        assert_eq!(read(&mut uart, RIS) & INT_OE, 0);
        write(&mut uart, DR, 0x40);
        assert_eq!(read(&mut uart, RSR), RSR_OE, "UARTRSR.OE is set at once");
        assert_eq!(read(&mut uart, RIS) & INT_OE, INT_OE);
        assert!(uart.irq());
        assert_eq!(read(&mut uart, FR) & (FLAG_RXFF | FLAG_RXFE), FLAG_RXFF);
        assert_eq!(read(&mut uart, DR), 0, "the oldest entry survived");
        assert_eq!(read(&mut uart, RSR), RSR_OE, "a read leaves the overrun");
        write(&mut uart, DR, 0x41);
        for i in 1..RX_FIFO_DEPTH as u32 {
            assert_eq!(read(&mut uart, DR), i, "entry {i} kept");
        }
        assert_eq!(read(&mut uart, DR), DR_OE | 0x41, "the next character carries the overrun");
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "the lost character never arrives");
        write(&mut uart, DR, 0x42);
        assert_eq!(read(&mut uart, DR), 0x42, "and only that one");
        write(&mut uart, RSR, 0);
        assert_eq!(read(&mut uart, RSR), 0, "UARTECR clears it");
        write(&mut uart, ICR, INT_OE);
        assert!(!uart.irq());
    }

    #[test]
    fn a_full_holding_register_overruns_the_same_way() {
        let mut uart = enabled();
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'1'));
        write(&mut uart, DR, u32::from(b'2'));
        assert!(!uart.can_receive());
        assert_eq!(read(&mut uart, RIS) & INT_OE, INT_OE);
        assert_eq!(read(&mut uart, DR), u32::from(b'1'), "the unread byte survived");
        assert_eq!(read(&mut uart, FR) & (FLAG_RXFE | FLAG_RXFF), FLAG_RXFE);
        write(&mut uart, DR, u32::from(b'3'));
        assert_eq!(read(&mut uart, DR), DR_OE | u32::from(b'3'));
    }

    #[test]
    fn modem_interrupts_mark_changes_of_the_status_lines() {
        let mut uart = Pl011::new();
        let outputs = CR_OUT2 | CR_OUT1 | CR_RTS | CR_DTR;
        let modem = FLAG_RI | FLAG_DCD | FLAG_DSR | FLAG_CTS;
        let ints = INT_RI | INT_DCD | INT_DSR | INT_CTS;
        write(&mut uart, CR, 0x300 | outputs);
        assert_eq!(read(&mut uart, FR) & modem, 0, "not in loopback");
        assert_eq!(read(&mut uart, RIS) & ints, 0);
        write(&mut uart, CR, 0x300 | outputs | CR_LBE);
        assert_eq!(read(&mut uart, FR) & modem, modem);
        assert_eq!(read(&mut uart, RIS) & ints, ints, "entering loopback changed every line");
        write(&mut uart, ICR, ints);
        write(&mut uart, CR, 0x300 | outputs | CR_LBE);
        assert_eq!(read(&mut uart, RIS) & ints, 0, "a rewrite changes no line");
        for (control, status, int) in [
            (CR_RTS, FLAG_CTS, INT_CTS),
            (CR_DTR, FLAG_DSR, INT_DSR),
            (CR_OUT1, FLAG_DCD, INT_DCD),
            (CR_OUT2, FLAG_RI, INT_RI),
        ] {
            write(&mut uart, CR, 0x300 | CR_LBE | (outputs & !control));
            assert_eq!(read(&mut uart, FR) & modem, modem & !status);
            assert_eq!(read(&mut uart, RIS) & ints, int, "deasserting raises it");
            write(&mut uart, ICR, int);
            write(&mut uart, CR, 0x300 | CR_LBE | outputs);
            assert_eq!(read(&mut uart, RIS) & ints, int, "and so does asserting");
            write(&mut uart, ICR, int);
        }
        write(&mut uart, IMSC, INT_CTS);
        write(&mut uart, CR, 0x300 | CR_LBE);
        assert!(uart.irq());
        write(&mut uart, ICR, ints);
        assert!(!uart.irq(), "UARTICR clears what the level would not");
        write(&mut uart, CR, 0x300 | CR_LBE | CR_RTS);
        write(&mut uart, CR, 0x300);
        assert_eq!(read(&mut uart, FR) & modem, FLAG_CTS, "leaving loopback keeps the status");
    }

    #[test]
    fn a_loopback_break_loads_one_break_character_and_raises_beris() {
        let mut uart = enabled();
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, IMSC, INT_BE);
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, RIS) & INT_BE, INT_BE);
        assert!(uart.irq());
        assert_eq!(read(&mut uart, DR), DR_BE);
        assert_eq!(read(&mut uart, RSR), DR_BE >> 8, "the break error lands in UARTRSR");
        write(&mut uart, ICR, INT_BE);
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "only setting BRK sends a break");
        assert_eq!(read(&mut uart, RIS) & INT_BE, 0);
        write(&mut uart, CR, 0x300 | CR_LBE);
        write(&mut uart, LCR_H, 0);
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, RIS) & INT_BE, 0, "a receiver that is off hears no break");
    }

    #[test]
    fn an_eight_byte_access_is_two_registers() {
        let mut uart = enabled();
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, LCR_H, LCR_BRK);
        // UARTDR first, which pops the break and sets UARTRSR, then UARTRSR.
        let pair = uart.read(reg(0), AccessWidth::Dword);
        assert_eq!(pair, u64::from(DR_BE) | (u64::from(DR_BE >> 8) << 32));
        uart.write(reg(13), AccessWidth::Dword, 0x0000_0050_0000_0012);
        assert_eq!(read(&mut uart, 13), 0x12);
        assert_eq!(read(&mut uart, IMSC), 0x50);
    }

    #[test]
    fn a_snapshot_round_trips_registers_and_queued_entries() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IMSC, 0x50);
        assert!(uart.receive(b'w'));
        assert_eq!(read(&mut uart, DR), u32::from(b'w'));
        assert!(uart.receive(b'x'));
        assert!(uart.receive(b'y'));
        write(&mut uart, CR, ENABLED | CR_LBE | CR_RTS);
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let mut restored = Pl011::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty(), "the restore consumed exactly what the save wrote");
        assert_eq!(read(&mut restored, FR) & FLAG_CTS, FLAG_CTS);
        assert_eq!(read(&mut restored, DR), u32::from(b'x'));
        assert_eq!(read(&mut restored, DR), u32::from(b'y'));
        assert_eq!(read(&mut restored, IMSC), 0x50);
        // The slot before the read position kept its byte too.
        write(&mut restored, LCR_H, 0);
        assert_eq!(read(&mut restored, DR), u32::from(b'w'));
    }

    #[test]
    fn a_snapshot_keeps_an_overrun_for_the_next_character() {
        let mut uart = enabled();
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'1'));
        write(&mut uart, DR, u32::from(b'2'));
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let mut restored = Pl011::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert_eq!(read(&mut restored, FR) & !FR_MASK, 0, "the state is not a flag");
        assert_eq!(read(&mut restored, DR), u32::from(b'1'));
        write(&mut restored, DR, u32::from(b'3'));
        assert_eq!(read(&mut restored, DR), DR_OE | u32::from(b'3'));
        let mut quiet = std::vec::Vec::new();
        Pl011::new().save(&mut quiet).expect("save");
        assert_eq!(&quiet[..4], &(FLAG_RXFE | FLAG_TXFE).to_le_bytes(), "no overrun, no extra bit");
    }

    #[test]
    fn a_snapshot_with_a_read_position_past_the_holding_register_is_refused() {
        let mut uart = Pl011::new();
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let pos = 11 * 4 + RX_FIFO_DEPTH * 4;
        image[pos] = 1;
        let mut source: &[u8] = &image;
        assert!(uart.restore(&mut source).is_err(), "FEN is clear, so only slot 0 exists");
        image[pos] = RX_FIFO_DEPTH as u8;
        let mut source: &[u8] = &image;
        assert!(uart.restore(&mut source).is_err());
    }

    #[test]
    fn a_snapshot_holding_more_entries_than_the_fifo_is_refused() {
        let mut uart = Pl011::new();
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let count = 11 * 4 + RX_FIFO_DEPTH * 4 + 1;
        image[count] = 2;
        let mut source: &[u8] = &image;
        assert!(uart.restore(&mut source).is_err(), "FEN is clear: one entry at most");
        image[count] = 1;
        let mut source: &[u8] = &image;
        assert!(uart.restore(&mut source).is_ok());
    }
}
