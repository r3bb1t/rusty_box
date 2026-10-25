//! Arm PrimeCell PL011 UART.
//!
//! Provenance (R7): QEMU `hw/char/pl011.c` — register map, reset values,
//! `pl011_id_arm`, the interrupt bits and their `pl011_update` combination,
//! and the receive FIFO (`pl011_put_fifo`, `pl011_can_receive`, the `UARTDR`
//! read): 16 entries while `UARTLCR_H.FEN` is set and one while it is clear,
//! emptied whenever `FEN` changes, with the receive interrupt raised when the
//! FIFO reaches its trigger level of one entry and lowered when it drains.
//! Transmission completes instantly: a byte written to `UARTDR` lands in the
//! output buffer the host drains, the transmit FIFO always reads empty, and
//! `INT_TX` is raised as QEMU raises it after every write.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::{RingBuffer, SnapError};

use crate::bus::{AccessWidth, RegOffset};

const PL011_ID: [u8; 8] = [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const FLAG_TXFE: u32 = 0x80;
const FLAG_RXFF: u32 = 0x40;
const FLAG_RXFE: u32 = 0x10;

const INT_TX: u32 = 1 << 5;
const INT_RX: u32 = 1 << 4;

/// `UARTLCR_H.FEN`: the FIFOs are enabled.
const LCR_FEN: u32 = 1 << 4;

/// QEMU `PL011_FIFO_DEPTH`.
pub const RX_FIFO_DEPTH: usize = 16;

/// Bytes the guest has transmitted and the host has not yet drained. A host
/// that never drains loses the oldest bytes, never stalls the guest.
pub const TX_CAPACITY: usize = 64 * 1024;

pub struct Pl011 {
    tx: RingBuffer<u8, TX_CAPACITY>,
    rx: RingBuffer<u8, RX_FIFO_DEPTH>,
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

    /// QEMU `pl011_put_fifo`, for a byte [`Pl011::can_receive`] said there was
    /// room for. Returns false, taking nothing, when there is none.
    pub fn put_fifo(&mut self, byte: u8) -> bool {
        if !self.can_receive() {
            return false;
        }
        self.rx.push_back(byte);
        // QEMU's read trigger is one entry: the first byte raises INT_RX.
        if self.rx.len() == 1 {
            self.int_level |= INT_RX;
        }
        true
    }

    fn flags(&self) -> u32 {
        let mut f = FLAG_TXFE;
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
                // With no error bits modelled, the byte is all a read
                // returns. An empty FIFO reads as 0 here, where QEMU's
                // `pl011_read_rxdata` returns whatever its ring holds at the
                // read position (a byte from an earlier lap, or 0 before the
                // first); a driver checks RXFE first, so neither is relied on.
                let c = self.rx.pop_front().map_or(0, u32::from);
                if self.rx.is_empty() {
                    self.int_level &= !INT_RX;
                }
                self.rsr = c >> 8;
                c
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
                self.int_level |= INT_TX;
            }
            1 => self.rsr = 0,
            8 => self.ilpr = v & 0xFF,
            9 => self.ibrd = v & 0xFFFF,
            10 => self.fbrd = v & 0x3F,
            11 => {
                // Changing FEN empties the receive FIFO.
                if (self.lcr ^ v) & LCR_FEN != 0 {
                    self.rx.clear();
                }
                self.lcr = v & 0xFF;
            }
            12 => self.cr = v & 0xFFFF,
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
        w.write_u8(self.rx.len() as u8)?;
        for byte in self.rx.iter() {
            w.write_u8(byte)?;
        }
        Ok(())
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut regs = [0u32; 10];
        for reg in regs.iter_mut() {
            *reg = r.read_u32()?;
        }
        let [rsr, ilpr, ibrd, fbrd, lcr, cr, ifl, int_enabled, int_level, dmacr] = regs;
        let queued = usize::from(r.read_u8()?);
        if queued > RX_FIFO_DEPTH {
            return Err(SnapError::Invalid("PL011 receive FIFO holds more than its depth"));
        }
        let mut rx = RingBuffer::new();
        for _ in 0..queued {
            rx.push_back(r.read_u8()?);
        }
        let tx = core::mem::take(&mut self.tx);
        *self = Self {
            tx,
            rx,
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
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let mut restored = Pl011::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(source.is_empty(), "the restore consumed exactly what the save wrote");
        assert_eq!(restored.read(reg(0), AccessWidth::Word), u64::from(b'x'));
        assert_eq!(restored.read(reg(0), AccessWidth::Word), u64::from(b'y'));
        assert_eq!(restored.read(reg(14), AccessWidth::Word), 0x50);
    }
}
