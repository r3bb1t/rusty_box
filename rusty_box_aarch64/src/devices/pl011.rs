//! Arm PrimeCell PL011 UART.
//!
//! Provenance (R7): QEMU 11.1.2 `hw/char/pl011.c` — register map, reset
//! values, `pl011_id_arm`, the interrupt bits and their `pl011_update`
//! combination, and the receive FIFO (`pl011_fifo_rx_put`,
//! `pl011_can_receive`, `pl011_read_rxdata`): 16 entries while
//! `UARTLCR_H.FEN` is set and one while it is clear, emptied whenever `FEN`
//! changes, with the receive interrupt raised when the FIFO reaches its
//! trigger level of one entry and lowered when it drains. An entry is a
//! character and its error bits, `UARTDR` bits 11:8, as the PL011's 12-bit
//! receive FIFO holds it. Transmission completes instantly: a byte written
//! to `UARTDR` lands in the output buffer the host drains, the transmit FIFO
//! always reads empty, and `INT_TX` is raised as QEMU raises it after every
//! write.
//!
//! Loopback (`UARTCR.LBE`), as QEMU's `pl011_loopback_tx`,
//! `pl011_loopback_break` and `pl011_loopback_mdmctrl` have it: a written
//! character also enters the receive FIFO (and still reaches the host), a
//! break set in `UARTLCR_H.BRK` enters it as a break character, the modem
//! outputs `UARTCR.Out2`, `Out1`, `RTS` and `DTR` read back as the modem
//! inputs `UARTFR.RI`, `DCD`, `CTS` and `DSR`, raising their interrupts
//! while asserted, and host input is ignored (`pl011_receive`). A
//! character looped into a full FIFO is an overrun, as the PL011 TRM (ARM
//! DDI 0183) describes it and QEMU does not model: the character is lost,
//! the FIFO's contents stay valid, and the overrun is flagged in
//! `UARTRSR.OE` until a write to `UARTECR`, in the overrun interrupt, and in
//! `UARTDR.OE` until a character next enters the FIFO.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::{RingBuffer, SnapError};

use crate::bus::{AccessWidth, RegOffset};

const PL011_ID: [u8; 8] = [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const FLAG_RI: u32 = 0x100;
const FLAG_TXFE: u32 = 0x80;
const FLAG_RXFF: u32 = 0x40;
const FLAG_RXFE: u32 = 0x10;
const FLAG_DCD: u32 = 0x04;
const FLAG_DSR: u32 = 0x02;
const FLAG_CTS: u32 = 0x01;
/// The modem status inputs `UARTFR` reports.
const FLAG_MODEM: u32 = FLAG_RI | FLAG_DCD | FLAG_DSR | FLAG_CTS;

const INT_OE: u32 = 1 << 10;
const INT_TX: u32 = 1 << 5;
const INT_RX: u32 = 1 << 4;
const INT_DSR: u32 = 1 << 3;
const INT_DCD: u32 = 1 << 2;
const INT_CTS: u32 = 1 << 1;
const INT_RI: u32 = 1 << 0;
const INT_MODEM: u32 = INT_DSR | INT_DCD | INT_CTS | INT_RI;

/// `UARTDR`'s error bits above the character: overrun and break.
const DR_OE: u16 = 1 << 11;
const DR_BE: u16 = 1 << 10;
/// `UARTRSR.OE`: an overrun since `UARTECR` was last written.
const RSR_OE: u32 = 1 << 3;

/// `UARTLCR_H.FEN`: the FIFOs are enabled. `UARTLCR_H.BRK`: send a break.
const LCR_FEN: u32 = 1 << 4;
const LCR_BRK: u32 = 1 << 0;

/// `UARTCR`'s modem outputs and loopback enable.
const CR_OUT2: u32 = 1 << 13;
const CR_OUT1: u32 = 1 << 12;
const CR_RTS: u32 = 1 << 11;
const CR_DTR: u32 = 1 << 10;
const CR_LBE: u32 = 1 << 7;

/// QEMU `PL011_FIFO_DEPTH`.
pub const RX_FIFO_DEPTH: usize = 16;

/// Bytes the guest has transmitted and the host has not yet drained. A host
/// that never drains loses the oldest bytes, never stalls the guest.
pub const TX_CAPACITY: usize = 64 * 1024;

pub struct Pl011 {
    tx: RingBuffer<u8, TX_CAPACITY>,
    /// Each entry a character and its `UARTDR` error bits.
    rx: RingBuffer<u16, RX_FIFO_DEPTH>,
    /// `UARTDR.OE`: a character was lost to a full FIFO, and none has
    /// entered it since.
    overrun: bool,
    /// The modem status inputs (`FLAG_MODEM` bits of `UARTFR`), as the
    /// loopback last set them; nothing drives them otherwise.
    modem: u32,
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
    /// QEMU `pl011_reset`.
    pub fn new() -> Self {
        Self {
            tx: RingBuffer::new(),
            rx: RingBuffer::new(),
            overrun: false,
            modem: 0,
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

    /// Return the registers and the receive FIFO to their reset state. Bytes
    /// already transmitted stay for the host: they left the device before
    /// the reset.
    pub fn reset(&mut self) {
        let tx = core::mem::take(&mut self.tx);
        *self = Self { tx, ..Self::new() };
    }

    /// The interrupt line (QEMU's combined `UARTINTR`).
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
    fn rx_depth(&self) -> usize {
        if self.lcr & LCR_FEN != 0 {
            RX_FIFO_DEPTH
        } else {
            1
        }
    }

    /// QEMU `pl011_can_receive`: whether the receive FIFO has room for a byte.
    pub fn can_receive(&self) -> bool {
        self.rx.len() < self.rx_depth()
    }

    /// How many more bytes the receive FIFO holds now.
    pub fn rx_room(&self) -> usize {
        self.rx_depth().saturating_sub(self.rx.len())
    }

    /// QEMU `pl011_receive`, for a byte from the host that
    /// [`Pl011::can_receive`] said there was room for. Returns false,
    /// taking nothing, when there is none. In loopback the receiver hears
    /// only the transmitter, and the byte is taken and dropped.
    pub fn put_fifo(&mut self, byte: u8) -> bool {
        if !self.can_receive() {
            return false;
        }
        if !self.loopback() {
            self.rx_put(u16::from(byte));
        }
        true
    }

    fn loopback(&self) -> bool {
        self.cr & CR_LBE != 0
    }

    /// QEMU `pl011_fifo_rx_put` for a character that has room; one that
    /// finds the FIFO full is an overrun.
    fn rx_put(&mut self, entry: u16) {
        if self.rx.len() >= self.rx_depth() {
            self.overrun = true;
            self.rsr |= RSR_OE;
            self.int_level |= INT_OE;
            return;
        }
        self.overrun = false;
        self.rx.push_back(entry);
        // QEMU's read trigger is one entry: the first raises INT_RX.
        if self.rx.len() == 1 {
            self.int_level |= INT_RX;
        }
    }

    /// QEMU `pl011_loopback_mdmctrl`: the modem outputs read back as the
    /// modem inputs, each input's interrupt raised while it is asserted.
    fn loop_modem_lines(&mut self) {
        let mut modem = 0;
        let mut ints = 0;
        for (out, input, int) in [
            (CR_OUT2, FLAG_RI, INT_RI),
            (CR_OUT1, FLAG_DCD, INT_DCD),
            (CR_RTS, FLAG_CTS, INT_CTS),
            (CR_DTR, FLAG_DSR, INT_DSR),
        ] {
            if self.cr & out != 0 {
                modem |= input;
                ints |= int;
            }
        }
        self.modem = modem;
        self.int_level = (self.int_level & !INT_MODEM) | ints;
    }

    fn flags(&self) -> u32 {
        let mut f = FLAG_TXFE | self.modem;
        if self.rx.is_empty() {
            f |= FLAG_RXFE;
        }
        if self.rx.len() >= self.rx_depth() {
            f |= FLAG_RXFF;
        }
        f
    }

    pub fn read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        let off = offset.get();
        u64::from(match off >> 2 {
            0 => {
                // An empty FIFO reads as 0 and leaves UARTRSR as it was,
                // where QEMU's `pl011_read_rxdata` returns whatever its ring
                // holds at the read position (a character from an earlier
                // lap, or 0 before the first); a driver checks RXFE first,
                // so neither is relied on.
                let Some(character) = self.rx.pop_front() else {
                    return 0;
                };
                let entry = character | if self.overrun { DR_OE } else { 0 };
                if self.rx.is_empty() {
                    self.int_level &= !INT_RX;
                }
                // UARTRSR: the character's framing, parity and break
                // errors; its overrun bit stays until UARTECR is written.
                self.rsr = (u32::from(entry) >> 8 & !RSR_OE) | (self.rsr & RSR_OE);
                u32::from(entry)
            }
            1 => self.rsr,
            6 => self.flags(),
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
        })
    }

    pub fn write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64) {
        let v = value as u32;
        match offset.get() >> 2 {
            0 => {
                self.tx.push_back(v as u8);
                if self.loopback() {
                    self.rx_put(u16::from(v as u8));
                }
                self.int_level |= INT_TX;
            }
            1 => self.rsr = 0,
            8 => self.ilpr = v & 0xFF,
            9 => self.ibrd = v & 0xFFFF,
            10 => self.fbrd = v & 0x3F,
            11 => {
                // Changing FEN empties the receive FIFO, which leaves no
                // overrun for UARTDR to report.
                if (self.lcr ^ v) & LCR_FEN != 0 {
                    self.rx.clear();
                    self.overrun = false;
                }
                // A break starting in loopback arrives as a break
                // character: no data, UARTDR.BE set.
                if (self.lcr ^ v) & v & LCR_BRK != 0 && self.loopback() {
                    self.rx_put(DR_BE);
                }
                self.lcr = v & 0xFF;
            }
            12 => {
                self.cr = v & 0xFFFF;
                if self.loopback() {
                    self.loop_modem_lines();
                }
            }
            13 => self.ifl = v & 0x3F,
            14 => self.int_enabled = v & 0x7FF,
            17 => self.int_level &= !v,
            18 => self.dmacr = v & 7,
            _ => {}
        }
    }

    /// The registers and the receive FIFO, for a snapshot. Bytes waiting in
    /// the transmit buffer belong to the host and are not part of it.
    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        for reg in [
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
        w.write_u32(self.modem)?;
        w.write_u8(u8::from(self.overrun))?;
        w.write_u8(self.rx.len() as u8)?;
        for entry in self.rx.iter() {
            w.write_u16(entry)?;
        }
        Ok(())
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut regs = [0u32; 10];
        for reg in regs.iter_mut() {
            *reg = r.read_u32()?;
        }
        let [rsr, ilpr, ibrd, fbrd, lcr, cr, ifl, int_enabled, int_level, dmacr] = regs;
        let modem = r.read_u32()?;
        if modem & !FLAG_MODEM != 0 {
            return Err(SnapError::Invalid("PL011 modem status holds a bit that is not a modem input"));
        }
        let overrun = match r.read_u8()? {
            0 => false,
            1 => true,
            _ => return Err(SnapError::Invalid("PL011 overrun flag is neither 0 nor 1")),
        };
        let queued = usize::from(r.read_u8()?);
        if queued > RX_FIFO_DEPTH {
            return Err(SnapError::Invalid("PL011 receive FIFO holds more than its depth"));
        }
        let mut rx = RingBuffer::new();
        for _ in 0..queued {
            let entry = r.read_u16()?;
            if entry & !(DR_BE | 0xFF) != 0 {
                return Err(SnapError::Invalid("PL011 receive FIFO entry holds an error the receiver cannot record"));
            }
            rx.push_back(entry);
        }
        let tx = core::mem::take(&mut self.tx);
        *self = Self {
            tx,
            rx,
            overrun,
            modem,
            rsr,
            ilpr,
            ibrd,
            fbrd,
            lcr,
            cr,
            ifl,
            int_enabled,
            int_level,
            dmacr,
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

    #[test]
    fn with_the_fifo_disabled_the_receiver_holds_one_byte() {
        let mut uart = Pl011::new();
        assert!(uart.put_fifo(b'a'));
        assert!(!uart.can_receive());
        assert!(!uart.put_fifo(b'b'), "a second byte has nowhere to go");
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFF, FLAG_RXFF);
        assert_eq!(uart.read(reg(0), AccessWidth::Word), u64::from(b'a'));
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn the_fifo_takes_sixteen_bytes_and_raises_rx_until_drained() {
        let mut uart = Pl011::new();
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN));
        uart.write(reg(14), AccessWidth::Word, u64::from(INT_RX));
        for i in 0..RX_FIFO_DEPTH as u8 {
            assert!(uart.put_fifo(i));
        }
        assert!(!uart.can_receive());
        assert!(uart.irq());
        for i in 0..RX_FIFO_DEPTH as u8 {
            assert_eq!(uart.read(reg(0), AccessWidth::Word), u64::from(i));
        }
        assert!(!uart.irq(), "an empty FIFO lowers the receive interrupt");
    }

    #[test]
    fn changing_fen_empties_the_receive_fifo() {
        let mut uart = Pl011::new();
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN));
        assert!(uart.put_fifo(1));
        assert!(uart.put_fifo(2));
        uart.write(reg(11), AccessWidth::Word, 0);
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn a_snapshot_round_trips_registers_and_queued_bytes() {
        let mut uart = Pl011::new();
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN));
        uart.write(reg(14), AccessWidth::Word, 0x50);
        assert!(uart.put_fifo(b'x'));
        assert!(uart.put_fifo(b'y'));
        // A looped break character and the modem lines the loopback set.
        uart.write(reg(12), AccessWidth::Word, u64::from(CR_ENABLED | CR_LBE | CR_OUT1));
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN | LCR_BRK));
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let mut restored = Pl011::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty(), "the restore consumed exactly what the save wrote");
        assert_eq!(restored.read(reg(0), AccessWidth::Word), u64::from(b'x'));
        assert_eq!(restored.read(reg(0), AccessWidth::Word), u64::from(b'y'));
        assert_eq!(restored.read(reg(0), AccessWidth::Word), u64::from(DR_BE));
        assert_eq!(restored.read(reg(6), AccessWidth::Word) as u32 & FLAG_MODEM, FLAG_DCD);
        assert_eq!(restored.read(reg(14), AccessWidth::Word), 0x50);
    }

    /// `UARTCR` as reset leaves it, UARTEN aside: TXE and RXE.
    const CR_ENABLED: u32 = 0x301;

    fn loopback_uart() -> Pl011 {
        let mut uart = Pl011::new();
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN));
        uart.write(reg(12), AccessWidth::Word, u64::from(CR_ENABLED | CR_LBE));
        uart
    }

    /// QEMU `pl011_loopback_tx`: in loopback a written character reaches
    /// the host and the receiver both, raising the receive interrupt.
    #[test]
    fn in_loopback_a_written_character_is_received_too() {
        let mut uart = loopback_uart();
        uart.write(reg(14), AccessWidth::Word, u64::from(INT_RX));
        uart.write(reg(0), AccessWidth::Word, u64::from(b'a'));
        assert!(uart.irq());
        assert_eq!(uart.read(reg(0), AccessWidth::Word), u64::from(b'a'));
        assert_eq!(uart.drain_tx().collect::<std::vec::Vec<u8>>(), b"a");
        // Out of loopback the transmitter reaches the host alone.
        uart.write(reg(12), AccessWidth::Word, u64::from(CR_ENABLED));
        uart.write(reg(0), AccessWidth::Word, u64::from(b'b'));
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFE, FLAG_RXFE);
        assert_eq!(uart.tx_len(), 1);
    }

    /// QEMU `pl011_receive`: in loopback the receiver hears only the
    /// transmitter; a host byte is taken and dropped.
    #[test]
    fn in_loopback_host_input_is_ignored() {
        let mut uart = loopback_uart();
        assert!(uart.put_fifo(b'h'));
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFE, FLAG_RXFE);
    }

    /// The PL011's overrun: a 17th character finds the 16-entry FIFO full
    /// and is lost, the 16 stay. UARTRSR.OE holds until UARTECR is written,
    /// the overrun interrupt is raised, and UARTDR.OE accompanies what is
    /// read until a character enters the FIFO again.
    #[test]
    fn a_character_looped_into_a_full_fifo_is_an_overrun() {
        let mut uart = loopback_uart();
        for c in b'a'..=b'q' {
            uart.write(reg(0), AccessWidth::Word, u64::from(c));
        }
        assert_eq!(uart.read(reg(15), AccessWidth::Word) as u32 & INT_OE, INT_OE);
        assert_eq!(uart.read(reg(1), AccessWidth::Word) as u32, RSR_OE);
        let first = uart.read(reg(0), AccessWidth::Word) as u16;
        assert_eq!(first, DR_OE | u16::from(b'a'));
        assert_eq!(uart.read(reg(1), AccessWidth::Word) as u32, RSR_OE, "OE stays across a read");
        uart.write(reg(1), AccessWidth::Word, 0);
        assert_eq!(uart.read(reg(1), AccessWidth::Word), 0, "a UARTECR write clears it");
        // A character entering the FIFO clears UARTDR.OE.
        uart.write(reg(0), AccessWidth::Word, u64::from(b'r'));
        let mut received = std::vec::Vec::new();
        while uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFE == 0 {
            received.push(uart.read(reg(0), AccessWidth::Word) as u16);
        }
        let expected: std::vec::Vec<u16> = (b'b'..=b'p').chain(*b"r").map(u16::from).collect();
        assert_eq!(received, expected);
    }

    /// A FEN change empties the FIFO and with it the overrun UARTDR reports;
    /// a read of an empty FIFO returns 0 and leaves UARTRSR as the last
    /// character left it.
    #[test]
    fn an_emptied_fifo_reports_no_overrun() {
        let mut uart = loopback_uart();
        for c in b'a'..=b'q' {
            uart.write(reg(0), AccessWidth::Word, u64::from(c));
        }
        uart.write(reg(11), AccessWidth::Word, 0);
        assert_eq!(uart.read(reg(0), AccessWidth::Word), 0);
        assert_eq!(uart.read(reg(1), AccessWidth::Word) as u32, RSR_OE, "UARTRSR keeps the overrun");
        // A break character into the one-entry FIFO, then an empty read.
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_BRK));
        assert_eq!(uart.read(reg(0), AccessWidth::Word), u64::from(DR_BE));
        assert_eq!(uart.read(reg(0), AccessWidth::Word), 0);
        assert_eq!(uart.read(reg(1), AccessWidth::Word) as u32, RSR_OE | 1 << 2, "BE stays");
    }

    /// A snapshot is refused whose FIFO holds an error the receiver cannot
    /// record: only a character and a break enter it.
    #[test]
    fn a_snapshot_with_an_impossible_fifo_entry_is_refused() {
        let mut uart = Pl011::new();
        assert!(uart.put_fifo(b'A'));
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let at = image.len() - 2;
        image[at..].copy_from_slice(&(u16::from(b'A') | DR_OE).to_le_bytes());
        let mut source: &[u8] = &image;
        assert!(Pl011::new().restore(&mut source).is_err());
    }

    /// QEMU `pl011_loopback_break`: a break starting in loopback arrives as
    /// a break character, whose UARTRSR.BE a read reports.
    #[test]
    fn in_loopback_a_break_arrives_as_a_break_character() {
        let mut uart = loopback_uart();
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN | LCR_BRK));
        uart.write(reg(11), AccessWidth::Word, u64::from(LCR_FEN | LCR_BRK));
        assert_eq!(uart.read(reg(0), AccessWidth::Word), u64::from(DR_BE));
        assert_eq!(uart.read(reg(1), AccessWidth::Word), 1 << 2, "UARTRSR.BE");
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_RXFE, FLAG_RXFE, "one break, one character");
    }

    /// QEMU `pl011_loopback_mdmctrl`: in loopback Out2, Out1, RTS and DTR
    /// read back as RI, DCD, CTS and DSR, each interrupt raised while its
    /// input is asserted; out of loopback the outputs reach no input.
    #[test]
    fn in_loopback_the_modem_outputs_are_the_modem_inputs() {
        let mut uart = Pl011::new();
        let modem_outputs = CR_OUT2 | CR_OUT1 | CR_RTS | CR_DTR;
        uart.write(reg(12), AccessWidth::Word, u64::from(CR_ENABLED | modem_outputs));
        assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_MODEM, 0);
        uart.write(reg(14), AccessWidth::Word, u64::from(INT_MODEM));
        for (out, input, int) in [
            (CR_OUT2, FLAG_RI, INT_RI),
            (CR_OUT1, FLAG_DCD, INT_DCD),
            (CR_RTS, FLAG_CTS, INT_CTS),
            (CR_DTR, FLAG_DSR, INT_DSR),
        ] {
            uart.write(reg(12), AccessWidth::Word, u64::from(CR_ENABLED | CR_LBE | out));
            assert_eq!(uart.read(reg(6), AccessWidth::Word) as u32 & FLAG_MODEM, input, "{out:#x}");
            assert_eq!(uart.read(reg(15), AccessWidth::Word) as u32 & INT_MODEM, int, "{out:#x}");
            assert!(uart.irq());
        }
        uart.write(reg(12), AccessWidth::Word, u64::from(CR_ENABLED | CR_LBE));
        assert_eq!(uart.read(reg(15), AccessWidth::Word) as u32 & INT_MODEM, 0, "a deasserted input lowers its interrupt");
        assert!(!uart.irq());
    }
}
