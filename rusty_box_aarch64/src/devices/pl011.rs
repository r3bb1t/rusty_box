//! Arm PrimeCell PL011 UART.
//!
//! Provenance (R7): QEMU 11.1.2 `hw/char/pl011.c` — register map, reset
//! values, `pl011_id_arm`, the interrupt bits and their `pl011_update`
//! combination, the flag register QEMU keeps as state, the receive FIFO
//! (`pl011_fifo_rx_put`, `pl011_read_rxdata`, `pl011_can_receive`,
//! `pl011_receive`): 16 entries while `UARTLCR_H.FEN` is set and one while it
//! is clear, emptied whenever `FEN` changes, with the receive interrupt
//! raised when the FIFO reaches its trigger level of one entry and lowered
//! when it drains; and loopback (`pl011_loopback_tx`,
//! `pl011_loopback_mdmctrl`, `pl011_loopback_break`). Transmission completes
//! instantly: a byte written to `UARTDR` lands in the output buffer the host
//! drains, the transmit FIFO always reads empty, and `INT_TX` is raised as
//! QEMU raises it after every write.
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

/// `UARTDR.BE`: the entry a break puts in the receive FIFO.
const DR_BE: u32 = 1 << 10;

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
/// `UARTCR.LBE`: loopback.
const CR_LBE: u32 = 1 << 7;

const IBRD_MASK: u32 = 0xFFFF;
const FBRD_MASK: u32 = 0x3F;

/// QEMU `PL011_FIFO_DEPTH`.
pub const RX_FIFO_DEPTH: usize = 16;
const _: () = assert!(RX_FIFO_DEPTH.is_power_of_two(), "slots are found by masking");

/// QEMU's `read_trigger`: `pl011_set_read_trigger` always chooses one entry.
const READ_TRIGGER: u32 = 1;

/// Bytes the guest has transmitted and the host has not yet drained. A host
/// that never drains loses the oldest bytes, never stalls the guest.
pub const TX_CAPACITY: usize = 64 * 1024;

pub struct Pl011 {
    tx: RingBuffer<u8, TX_CAPACITY>,
    /// QEMU `read_fifo`: each entry is a `UARTDR` value, a received byte or
    /// [`DR_BE`].
    read_fifo: [u32; RX_FIFO_DEPTH],
    /// Below [`RX_FIFO_DEPTH`], and zero while the FIFO is disabled.
    read_pos: usize,
    /// Entries queued. A loopback transmit queues without checking for room,
    /// as QEMU's `pl011_loopback_tx` does, so this can exceed the depth: the
    /// new entry overwrites the oldest slot and the count keeps growing.
    /// QEMU's counter is a signed `int` whose overflow is undefined; this one
    /// saturates.
    read_count: u32,
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
    /// written.
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
    fn fifo_depth(&self) -> u32 {
        if self.lcr & LCR_FEN != 0 {
            RX_FIFO_DEPTH as u32
        } else {
            1
        }
    }

    fn loopback(&self) -> bool {
        self.cr & CR_LBE != 0
    }

    /// QEMU `pl011_can_receive`: whether the receive side takes a host byte.
    pub fn can_receive(&self) -> bool {
        self.rx_room() > 0
    }

    /// How many host bytes the receive side takes now: QEMU
    /// `pl011_can_receive`'s `fifo_depth - read_count`. A count past the
    /// depth (see [`Pl011::read_count`]) wraps to a negative `int` there,
    /// which QEMU's character layer treats as no room; here it is zero.
    pub fn rx_room(&self) -> usize {
        self.fifo_depth().saturating_sub(self.read_count) as usize
    }

    /// QEMU `pl011_receive` for one host byte [`Pl011::can_receive`] said
    /// there was room for: queued, or consumed and dropped while loopback
    /// disconnects the receive input. Returns false, taking nothing, when
    /// there is no room.
    pub fn receive(&mut self, byte: u8) -> bool {
        if !self.can_receive() {
            return false;
        }
        if !self.loopback() {
            self.fifo_rx_put(u32::from(byte));
        }
        true
    }

    /// QEMU `pl011_fifo_rx_put`.
    fn fifo_rx_put(&mut self, value: u32) {
        let depth = self.fifo_depth();
        let slot = (self.read_pos as u32).wrapping_add(self.read_count) & (depth - 1);
        self.read_fifo[slot as usize] = value;
        self.read_count = self.read_count.saturating_add(1);
        self.flags &= !FLAG_RXFE;
        if self.read_count == depth {
            self.flags |= FLAG_RXFF;
        }
        if self.read_count == READ_TRIGGER {
            self.int_level |= INT_RX;
        }
    }

    /// QEMU `pl011_loopback_tx`.
    fn loopback_tx(&mut self, value: u32) {
        if self.loopback() {
            self.fifo_rx_put(value);
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
        }
        if self.read_count == READ_TRIGGER - 1 {
            self.int_level &= !INT_RX;
        }
        self.rsr = c >> 8;
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
    /// drive the modem status inputs, and their interrupt bits follow. Out of
    /// loopback the flags keep whatever loopback last set.
    fn loopback_mdmctrl(&mut self) {
        if !self.loopback() {
            return;
        }
        let cr = self.cr;
        let mut fr = self.flags & !(FLAG_RI | FLAG_DCD | FLAG_DSR | FLAG_CTS);
        for (control, status) in [
            (CR_OUT2, FLAG_RI),
            (CR_OUT1, FLAG_DCD),
            (CR_RTS, FLAG_CTS),
            (CR_DTR, FLAG_DSR),
        ] {
            if cr & control != 0 {
                fr |= status;
            }
        }
        let mut il = self.int_level & !(INT_DSR | INT_DCD | INT_CTS | INT_RI);
        for (status, int) in [
            (FLAG_DSR, INT_DSR),
            (FLAG_DCD, INT_DCD),
            (FLAG_CTS, INT_CTS),
            (FLAG_RI, INT_RI),
        ] {
            if fr & status != 0 {
                il |= int;
            }
        }
        self.flags = fr;
        self.int_level = il;
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
            6 => self.flags,
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
            8 => self.ilpr = v,
            9 => self.ibrd = v & IBRD_MASK,
            10 => self.fbrd = v & FBRD_MASK,
            11 => {
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
                self.cr = v;
                self.loopback_mdmctrl();
            }
            13 => self.ifl = v,
            14 => self.int_enabled = v,
            17 => self.int_level &= !v,
            18 => self.dmacr = v,
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
    /// FIFO is disabled (QEMU `pl011_post_load`'s checks).
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
        if lcr & LCR_FEN == 0 && read_pos != 0 {
            return Err(SnapError::Invalid("PL011 read position past the one-entry receive holding register"));
        }
        let tx = core::mem::take(&mut self.tx);
        *self = Self {
            tx,
            read_fifo,
            read_pos,
            read_count,
            flags,
            rsr,
            ilpr,
            ibrd: ibrd & IBRD_MASK,
            fbrd: fbrd & FBRD_MASK,
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
    const IMSC: u64 = 14;
    const RIS: u64 = 15;

    #[test]
    fn with_the_fifo_disabled_the_receiver_holds_one_byte() {
        let mut uart = Pl011::new();
        assert!(uart.receive(b'a'));
        assert!(!uart.can_receive());
        assert!(!uart.receive(b'b'), "a second byte has nowhere to go");
        assert_eq!(read(&mut uart, FR) & FLAG_RXFF, FLAG_RXFF);
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn the_fifo_takes_sixteen_bytes_and_raises_rx_until_drained() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IMSC, INT_RX);
        for i in 0..RX_FIFO_DEPTH as u8 {
            assert!(uart.receive(i));
        }
        assert!(!uart.can_receive());
        assert!(uart.irq());
        for i in 0..RX_FIFO_DEPTH as u8 {
            assert_eq!(read(&mut uart, DR), u32::from(i));
        }
        assert!(!uart.irq(), "an empty FIFO lowers the receive interrupt");
    }

    #[test]
    fn changing_fen_empties_the_receive_fifo() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        assert!(uart.receive(1));
        assert!(uart.receive(2));
        write(&mut uart, LCR_H, 0);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn an_empty_fifo_reads_the_slot_at_the_read_position() {
        let mut uart = Pl011::new();
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
    fn registers_hold_what_the_guest_wrote() {
        let mut uart = Pl011::new();
        for n in [8, 11, 12, 13, 14, 18] {
            write(&mut uart, n, 0xFFFF_FF00);
            assert_eq!(read(&mut uart, n), 0xFFFF_FF00, "register {n:#x}");
        }
        write(&mut uart, 9, 0xFFFF_FFFF);
        write(&mut uart, 10, 0xFFFF_FFFF);
        assert_eq!(read(&mut uart, 9), IBRD_MASK);
        assert_eq!(read(&mut uart, 10), FBRD_MASK);
        write(&mut uart, FR, 0);
        assert_eq!(read(&mut uart, FR), FLAG_RXFE | FLAG_TXFE, "the flag register ignores writes");
    }

    #[test]
    fn loopback_returns_transmitted_bytes_and_drops_host_input() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, CR, 0x300 | CR_LBE);
        write(&mut uart, DR, u32::from(b'L'));
        assert_eq!(uart.drain_tx().collect::<std::vec::Vec<u8>>(), b"L", "the byte still leaves");
        assert!(uart.receive(b'h'), "host input is consumed");
        assert_eq!(read(&mut uart, DR), u32::from(b'L'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "and dropped");
        write(&mut uart, CR, 0x300);
        assert!(uart.receive(b'h'));
        assert_eq!(read(&mut uart, DR), u32::from(b'h'));
    }

    #[test]
    fn loopback_overruns_the_oldest_slot_without_checking_for_room() {
        let mut uart = Pl011::new();
        write(&mut uart, CR, 0x300 | CR_LBE);
        write(&mut uart, DR, u32::from(b'1'));
        write(&mut uart, DR, u32::from(b'2'));
        assert!(!uart.can_receive());
        // One slot, written twice: both reads return the newer byte.
        assert_eq!(read(&mut uart, DR), u32::from(b'2'));
        assert_eq!(read(&mut uart, FR) & (FLAG_RXFE | FLAG_RXFF), 0, "an entry is still counted, RXFF clears");
        assert_eq!(read(&mut uart, DR), u32::from(b'2'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn loopback_wires_modem_outputs_to_status_inputs() {
        let mut uart = Pl011::new();
        let outputs = CR_OUT2 | CR_OUT1 | CR_RTS | CR_DTR;
        write(&mut uart, CR, 0x300 | outputs);
        assert_eq!(read(&mut uart, FR) & 0x10F, 0, "not in loopback");
        write(&mut uart, CR, 0x300 | outputs | CR_LBE);
        let modem = FLAG_RI | FLAG_DCD | FLAG_DSR | FLAG_CTS;
        assert_eq!(read(&mut uart, FR) & modem, modem);
        let ints = INT_RI | INT_DCD | INT_DSR | INT_CTS;
        assert_eq!(read(&mut uart, RIS) & ints, ints);
        write(&mut uart, CR, 0x300 | CR_LBE | CR_RTS);
        assert_eq!(read(&mut uart, FR) & modem, FLAG_CTS);
        assert_eq!(read(&mut uart, RIS) & ints, INT_CTS);
        write(&mut uart, CR, 0x300);
        assert_eq!(read(&mut uart, FR) & modem, FLAG_CTS, "leaving loopback keeps the status");
    }

    #[test]
    fn a_loopback_break_queues_a_break_entry() {
        let mut uart = Pl011::new();
        write(&mut uart, CR, 0x300 | CR_LBE);
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, DR), DR_BE);
        assert_eq!(read(&mut uart, RSR), DR_BE >> 8, "the break error lands in UARTRSR");
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "only setting BRK sends a break");
    }

    #[test]
    fn an_eight_byte_access_is_two_registers() {
        let mut uart = Pl011::new();
        write(&mut uart, CR, 0x300 | CR_LBE);
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
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IMSC, 0x50);
        assert!(uart.receive(b'w'));
        assert_eq!(read(&mut uart, DR), u32::from(b'w'));
        assert!(uart.receive(b'x'));
        assert!(uart.receive(b'y'));
        write(&mut uart, CR, 0x300 | CR_LBE | CR_RTS);
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
}
