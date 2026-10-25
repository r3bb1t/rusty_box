//! Arm PrimeCell PL011 UART.
//!
//! Provenance (R7): QEMU `hw/char/pl011.c` — register map, reset values,
//! `pl011_id_arm`, the interrupt bits and their `pl011_update` combination.
//! Transmission completes instantly: a byte written to `UARTDR` lands in the
//! output buffer the host drains, the transmit FIFO always reads empty, and
//! `INT_TX` is raised as QEMU raises it after every write.

use rusty_box_core::RingBuffer;

use crate::bus::{AccessWidth, RegOffset};

const PL011_ID: [u8; 8] = [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const FLAG_TXFE: u32 = 0x80;
const FLAG_RXFF: u32 = 0x40;
const FLAG_RXFE: u32 = 0x10;

const INT_TX: u32 = 1 << 5;
const INT_RX: u32 = 1 << 4;

/// Bytes the guest has transmitted and the host has not yet drained.
pub const TX_CAPACITY: usize = 64 * 1024;
/// Bytes the host has queued for the guest to receive.
pub const RX_CAPACITY: usize = 256;

pub struct Pl011 {
    tx: RingBuffer<u8, TX_CAPACITY>,
    rx: RingBuffer<u8, RX_CAPACITY>,
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

    /// Queue a byte for the guest to receive. Returns false (and drops the
    /// byte) when the receive FIFO is full, as a line with no flow control
    /// would.
    pub fn push_rx(&mut self, byte: u8) -> bool {
        if self.rx.len() >= RX_CAPACITY {
            return false;
        }
        self.rx.push_back(byte);
        self.int_level |= INT_RX;
        true
    }

    fn flags(&self) -> u32 {
        let mut f = FLAG_TXFE;
        if self.rx.is_empty() {
            f |= FLAG_RXFE;
        }
        if self.rx.len() >= RX_CAPACITY {
            f |= FLAG_RXFF;
        }
        f
    }

    pub fn read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        let off = offset.get();
        u64::from(match off >> 2 {
            0 => {
                let c = self.rx.pop_front().map_or(0, u32::from);
                if self.rx.is_empty() {
                    self.int_level &= !INT_RX;
                }
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
            11 => self.lcr = v & 0xFF,
            12 => self.cr = v & 0xFFFF,
            13 => self.ifl = v & 0x3F,
            14 => self.int_enabled = v & 0x7FF,
            17 => self.int_level &= !v,
            18 => self.dmacr = v & 7,
            _ => {}
        }
    }
}
