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
//!   zero (Tables 3-4, 3-10, 3-12, 3-13, 3-14, 3-18). `UARTILPR` is 8 bits
//!   wide (Table 3-1); Table 3-5 defines its bits [7:0] and says nothing of
//!   the rest, which read as zero here. `UARTRSR` bits [7:4],
//!   unpredictable, read as zero too.
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
//!   set; cleared when reads empty the FIFO, or by `UARTICR`. The count
//!   runs on the machine's clock: 32 bit periods are 512 × (`UARTIBRD` +
//!   `UARTFBRD` / 64) cycles of `UARTCLK` (§2.2.3, §2.4.3), at the
//!   divisor the last `UARTLCR_H` write latched (§3.3.7), and `RTRIS` is
//!   set at the first tick by which they have passed since the last
//!   character arrived, or since `RTIM` was set with characters waiting.
//!   It fires once; the next character, or setting `RTIM` again, starts
//!   another count. Without a baud clock (`UARTIBRD` 0, or 0xFFFF with
//!   `UARTFBRD` above 0, §3.3.6) no count starts. Where DDI 0183F is
//!   silent: a count running when a new divisor is latched keeps the
//!   deadline it started with, whatever that divisor, one without a baud
//!   clock included; and a count runs whether or not the receiver is
//!   enabled (`UARTEN`, `RXE`), since the FIFO still holds what it
//!   received and nothing more arrives.
//! - The modem status interrupts (§3.4.1) are set by a change of their
//!   line and cleared only by `UARTICR`.
//! - Transmission needs `UARTCR.UARTEN` and `UARTCR.TXE` (§3.3.8, Note).
//!   While either is clear, a character written to `UARTDR` waits in the
//!   transmit FIFO (§2.2.4, §3.3.1: 16 bytes, or the one-byte holding
//!   register with the FIFO disabled), and `UARTFR` reports it: `BUSY` set
//!   "regardless of whether the UART is enabled or not", `TXFE` clear, and
//!   `TXFF` once it is full (Table 3-4). Enabling transmission sends what
//!   waits; clearing `FEN` flushes it (§3.3.8, Note: "Flush the transmit
//!   FIFO by disabling bit 4 (FEN)").
//! - `TXRIS` (§3.4.3, §3.3.9): marks a transition of the transmit FIFO's
//!   fill level through the trigger level, not the level itself. With the
//!   FIFO enabled it is set when the level falls from above the
//!   `UARTIFLS.TXIFLSEL` level (2, 4, 8, 12 or 14 of 16; 8 at reset) to it
//!   or below, and cleared when writes take it above; with the FIFO
//!   disabled, set when the holding register empties and cleared by a
//!   write that fills it. `UARTICR` clears it either way. Enabling the UART
//!   and the interrupt with nothing written sets nothing (§3.4.3, Note).
//!
//! Time on the line: characters take no time to cross it, whatever the
//! divisor, `UARTIBRD` 0 (no baud clock) included. A character goes
//! out the moment transmission is enabled to send it: the transmit FIFO
//! empties at once, so while transmission is enabled a write never takes
//! the FIFO above one character, and only the holding register (FIFO
//! disabled) or characters held back while transmission was off can take
//! the level through the trigger level. The host line delivers its bytes
//! the moment the receiver has room, as a QEMU character backend does. The
//! receive timeout is the one interval the UART times, as above.
//!
//! From QEMU 11.1.2 where DDI 0183F leaves the choice to the system: the
//! flag register QEMU keeps as state, `UARTDR` reads of an empty FIFO
//! (`pl011_read_rxdata`: the slot at the read position), both FIFOs
//! emptied whenever `FEN` changes, set or cleared (`pl011_write`; DDI
//! 0183F names only the transmit FIFO's flush, on clearing), loopback
//! driving the modem status inputs from the modem control outputs and
//! leaving them where they were when loopback ends
//! (`pl011_loopback_mdmctrl`), a loopback break on each setting of
//! `UARTLCR_H.BRK` (`pl011_loopback_break`), and a host line that sends
//! nothing in loopback (`pl011_receive`). The host side of a break has no
//! backend here: the line neither sends nor delivers one. QEMU transmits
//! whatever `UARTEN` and `TXE` say, and raises `TXRIS` after every write
//! (`pl011_write_txdata`); README's "Where the references differ" has
//! this model's differences, and the direct-boot set-up
//! ([`Pl011::set_up_console`]) that goes with it.
//!
//! Where DDI 0183F is silent and this model does not follow QEMU, its
//! reading: emptying the receive FIFO through `FEN` clears `RXRIS` and
//! `RTRIS`, since an empty FIFO is below every trigger level and §3.4.4
//! asserts `RTRIS` only while the FIFO is not empty (DDI 0183F names reads
//! and `UARTICR` as what clears them; QEMU leaves both as they were).
//!
//! The region takes 4-byte accesses (`pl011_ops`' `impl.min_access_size` and
//! `impl.max_access_size`): QEMU's `access_with_adjusted_size` hands a
//! narrower access to the register at `offset >> 2` with the value
//! unshifted, and splits an 8-byte one into two, at `offset` and then
//! `offset + 4`.

use core::num::NonZeroU32;

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};
use rusty_box_core::{ClockHz, Cycles, RingBuffer, SnapError, VmClock, VmInstant};

use crate::bus::{AccessWidth, RegOffset};

/// The registers' offsets in the region (DDI 0183F Table 3-1).
const UARTDR: RegOffset = RegOffset::new(0x000);
/// `UARTRSR` when read, `UARTECR` when written.
const UARTRSR: RegOffset = RegOffset::new(0x004);
const UARTFR: RegOffset = RegOffset::new(0x018);
const UARTILPR: RegOffset = RegOffset::new(0x020);
const UARTIBRD: RegOffset = RegOffset::new(0x024);
const UARTFBRD: RegOffset = RegOffset::new(0x028);
const UARTLCR_H: RegOffset = RegOffset::new(0x02C);
const UARTCR: RegOffset = RegOffset::new(0x030);
const UARTIFLS: RegOffset = RegOffset::new(0x034);
const UARTIMSC: RegOffset = RegOffset::new(0x038);
const UARTRIS: RegOffset = RegOffset::new(0x03C);
const UARTMIS: RegOffset = RegOffset::new(0x040);
const UARTICR: RegOffset = RegOffset::new(0x044);
const UARTDMACR: RegOffset = RegOffset::new(0x048);
/// `UARTPeriphID0` to `UARTPCellID3`: the eight identification registers
/// that end the region, one byte of [`PL011_ID`] each.
const UARTPERIPHID0: RegOffset = RegOffset::new(0xFE0);
const UARTPCELLID3: RegOffset = RegOffset::new(0xFFC);

/// The register an access at `offset` reaches: the region takes 4-byte
/// accesses (QEMU `pl011_ops`, as the module documentation says), so a
/// narrower one reaches the register that holds it.
const fn register_at(offset: RegOffset) -> RegOffset {
    RegOffset::new(offset.get() & !3)
}

const PL011_ID: [u8; 8] = [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

const FLAG_RI: u32 = 0x100;
const FLAG_TXFE: u32 = 0x80;
const FLAG_RXFF: u32 = 0x40;
const FLAG_TXFF: u32 = 0x20;
const FLAG_RXFE: u32 = 0x10;
const FLAG_BUSY: u32 = 0x08;
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

/// `UARTLCR_H.WLEN` = 0b11: eight data bits.
const LCR_WLEN_8: u32 = 0b11 << 5;
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
/// `UARTCR.TXE`: the transmit section is enabled.
const CR_TXE: u32 = 1 << 8;
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

/// The transmit FIFO's depth (DDI 0183F §2.2.4): 16 bytes, one (the
/// transmit holding register) while `UARTLCR_H.FEN` is clear.
pub const TX_FIFO_DEPTH: usize = 16;

/// The overrun state (DDI 0183F §2.4.3, "Overrun bit"): a character was
/// lost to a full FIFO, and none has been written to it since. It is kept
/// in bit 31 of the flag register's word, which `UARTFR` does not have (a
/// read returns its nine bits), so a snapshot carries it in the word it
/// always wrote, zero for a UART that never overran. Only
/// [`Pl011::note_overrun`] and [`Pl011::take_overrun`] change it, and a
/// restore keeps it.
const OVERRUN_PENDING: u32 = 1 << 31;

/// Bytes the guest has transmitted and the host has not yet drained. A host
/// that never drains loses the oldest bytes, never stalls the guest.
pub const TX_CAPACITY: usize = 64 * 1024;

/// The clocks a UART access is timed by: the machine's, at the access, and
/// the UART's reference clock `UARTCLK`, from which the baud rate divisor
/// makes the bit period (DDI 0183F §2.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartClocks {
    pub machine: VmClock,
    pub uartclk: ClockHz,
}

/// What [`Pl011::receive`] did with a host byte.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    /// The receive side took it: into the FIFO, or, while loopback
    /// disconnects the host line, consumed and dropped.
    Taken,
    /// There was no room; nothing changed.
    NoRoom,
}

/// A line rate, in bits per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaudRate(NonZeroU32);

impl BaudRate {
    pub const fn new(bits_per_second: u32) -> Option<Self> {
        match NonZeroU32::new(bits_per_second) {
            Some(rate) => Some(Self(rate)),
            None => None,
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// The baud rate divisor, `UARTCLK` cycles per `Baud16` period in 64ths:
/// `64 × UARTIBRD + UARTFBRD` (DDI 0183F §3.3.6, Figure 2-3), 22 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BaudDivisor(u32);

impl BaudDivisor {
    /// The widest divisor: `UARTIBRD`'s 16 bits and `UARTFBRD`'s 6.
    const MAX: u32 = (IBRD_MASK << 6) | FBRD_MASK;
    /// The slowest divisor that makes a baud clock (§3.3.6: `UARTIBRD`
    /// 0xFFFF with a fraction aborts transmission and reception).
    const SLOWEST: Self = Self(IBRD_MASK << 6);

    /// The divisor `UARTIBRD` and `UARTFBRD` hold.
    const fn of_registers(ibrd: u32, fbrd: u32) -> Self {
        Self(((ibrd & IBRD_MASK) << 6) | (fbrd & FBRD_MASK))
    }

    /// The divisor TF-A's `console_pl011_core_init` and EDK2's
    /// `PL011UartInitializePort` program for `baud` from `uartclk`:
    /// `UARTCLK × 4 / baud` 64ths, rounded down. A rate too slow for
    /// `UARTIBRD`'s 16 bits gets [`BaudDivisor::SLOWEST`].
    fn for_rate(uartclk: ClockHz, baud: BaudRate) -> Self {
        let sixty_fourths = uartclk.get() * 4 / u64::from(baud.get());
        match u32::try_from(sixty_fourths) {
            Ok(d) if d <= Self::SLOWEST.0 => Self(d),
            _ => Self::SLOWEST,
        }
    }

    /// `UARTIBRD`'s part.
    const fn integer(self) -> u32 {
        self.0 >> 6
    }

    /// `UARTFBRD`'s part.
    const fn fraction(self) -> u32 {
        self.0 & FBRD_MASK
    }

    /// The whole divisor, in 64ths.
    const fn sixty_fourths(self) -> u32 {
        self.0
    }
}

/// The UART: its registers, both FIFOs and the bytes transmitted.
pub struct Pl011 {
    /// Bytes transmitted, for the host.
    tx: RingBuffer<u8, TX_CAPACITY>,
    /// The transmit FIFO: characters written while transmission is off,
    /// oldest first, in `tx_fifo[..tx_count]`. Transmission empties it
    /// whole, so it never wraps.
    tx_fifo: [u8; TX_FIFO_DEPTH],
    /// Characters waiting: at most the FIFO's depth, and zero while
    /// transmission is on.
    tx_count: u8,
    /// QEMU `read_fifo`: each entry is a `UARTDR` value, a received byte or
    /// [`DR_BE`], with [`DR_OE`] when an overrun came before it.
    read_fifo: [u32; RX_FIFO_DEPTH],
    /// Below [`RX_FIFO_DEPTH`], and zero while the FIFO is disabled.
    read_pos: u8,
    /// Entries queued: at most the FIFO's depth.
    read_count: u32,
    /// The baud rate divisor as the last `UARTLCR_H` write latched it:
    /// `UARTLCR_H`, `UARTIBRD` and `UARTFBRD` form one register that only a
    /// `UARTLCR_H` write updates (DDI 0183F §3.3.7, Note).
    divisor: BaudDivisor,
    /// The receive timeout's count, while one runs: the first instant at
    /// which 32 bit periods (§3.4.4) have passed since it started. It runs
    /// only while the receive FIFO holds something and `RTIM` is set, and
    /// keeps that instant whatever divisor is latched after it started.
    rx_timeout: Option<VmInstant>,
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
            tx_fifo: [0; TX_FIFO_DEPTH],
            tx_count: 0,
            read_fifo: [0; RX_FIFO_DEPTH],
            read_pos: 0,
            read_count: 0,
            divisor: BaudDivisor::of_registers(0, 0),
            rx_timeout: None,
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
    /// left the device before the reset. Characters waiting in the transmit
    /// FIFO never left, and the reset empties it.
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

    /// The fill level `TXRIS` marks (DDI 0183F §3.4.3, Table 3-13): with
    /// the FIFO enabled, the eighths `UARTIFLS.TXIFLSEL` selects ("Transmit
    /// FIFO becomes <= 1/8 full" ... "<= 7/8 full"); the reserved encodings
    /// 0b101–0b111 select the half-full level, as reset does. With the FIFO
    /// disabled, zero: the holding register empty.
    fn tx_trigger(&self) -> u32 {
        if self.lcr & LCR_FEN == 0 {
            return 0;
        }
        let eighths = match self.ifl & 7 {
            0 => 1,
            1 => 2,
            3 => 6,
            4 => 7,
            _ => 4,
        };
        TX_FIFO_DEPTH as u32 * eighths / 8
    }

    /// The transmit FIFO's fill level has moved from `before` to what it
    /// holds now. `TXRIS` marks a transition through the trigger level, not
    /// the level itself (DDI 0183F §3.3.9, and §3.4.3's Note: enabling the
    /// UART and the interrupt before anything is written sets nothing): it
    /// is set when the level falls from above the trigger level to it or
    /// below, and cleared when writes take it from there to above (§3.4.3:
    /// "cleared by writing data to the transmit FIFO until it becomes
    /// greater than the trigger level"), or by `UARTICR`. With the FIFO
    /// disabled the trigger level is zero, so the holding register emptying
    /// sets it and a write filling it clears it (§3.4.3, second case).
    fn tx_level_moved(&mut self, before: u32) {
        let level = self.tx_trigger();
        let after = u32::from(self.tx_count);
        if before > level && after <= level {
            self.int_level |= INT_TX;
        } else if before <= level && after > level {
            self.int_level &= !INT_TX;
        }
    }

    fn loopback(&self) -> bool {
        self.cr & CR_LBE != 0
    }

    /// DDI 0183F §3.3.8, Note: reception needs `UARTEN` and `RXE`.
    fn receiving(&self) -> bool {
        self.cr & (CR_UARTEN | CR_RXE) == CR_UARTEN | CR_RXE
    }

    /// DDI 0183F §3.3.8, Note: transmission needs `UARTEN` and `TXE`.
    fn transmitting(&self) -> bool {
        self.cr & (CR_UARTEN | CR_TXE) == CR_UARTEN | CR_TXE
    }

    /// A `UARTDR` write: the character goes into the transmit FIFO, or the
    /// holding register with the FIFO disabled (DDI 0183F §3.3.1), and is
    /// transmitted from there while transmission is enabled. DDI 0183F does
    /// not say what a write to a full transmit FIFO does; here the
    /// character is lost and the FIFO keeps what it holds, as the receive
    /// FIFO does on an overrun.
    fn write_txdata(&mut self, byte: u8, clocks: UartClocks) {
        if u32::from(self.tx_count) == self.fifo_depth() {
            return;
        }
        let before = u32::from(self.tx_count);
        self.tx_fifo[usize::from(self.tx_count)] = byte;
        self.tx_count += 1;
        self.tx_level_moved(before);
        self.transmit(clocks);
    }

    /// Send what the transmit FIFO holds, if transmission is enabled. With
    /// a zero bit period the whole FIFO goes at once, each character to the
    /// host and, in loopback, to the receiver (QEMU `pl011_write_txdata`,
    /// `pl011_loopback_tx`), and the fill level falls to zero at once:
    /// `TXRIS` is set if that takes it through the trigger level
    /// ([`Pl011::tx_level_moved`]). While transmission is enabled every
    /// write leaves at once, so with the FIFO enabled the level never rises
    /// above one character and `TXRIS` stays as it was; with it disabled
    /// each write fills and empties the holding register and sets `TXRIS`.
    /// `UARTFR` reports the FIFO (Table 3-4): `TXFE` while it is empty,
    /// `TXFF` while it is full, and `BUSY` while it holds anything, whether
    /// or not the UART is enabled.
    fn transmit(&mut self, clocks: UartClocks) {
        if self.transmitting() && self.tx_count > 0 {
            let before = u32::from(self.tx_count);
            for i in 0..usize::from(self.tx_count) {
                let byte = self.tx_fifo[i];
                self.tx.push_back(byte);
                self.loopback_tx(u32::from(byte), clocks);
            }
            self.tx_count = 0;
            self.tx_level_moved(before);
        }
        self.flags &= !(FLAG_TXFE | FLAG_TXFF | FLAG_BUSY);
        self.flags |= self.tx_flags();
    }

    /// `UARTFR`'s transmit bits for the transmit FIFO as it is.
    fn tx_flags(&self) -> u32 {
        if self.tx_count == 0 {
            FLAG_TXFE
        } else if u32::from(self.tx_count) == self.fifo_depth() {
            FLAG_TXFF | FLAG_BUSY
        } else {
            FLAG_BUSY
        }
    }

    /// Leave the UART as boot firmware leaves a console it has set up, the
    /// state a kernel booted without firmware expects (Linux's
    /// `earlycon=pl011` writes to the port as it finds it): `baud` from
    /// `UARTCLK` with the divisor TF-A's `console_pl011_core_init` and
    /// EDK2's `PL011UartInitializePort` program, `UARTCLK × 4 / baud` in
    /// 64ths (`UARTIBRD` its integer part, `UARTFBRD` its fraction), latched
    /// by a `UARTLCR_H` write of eight data bits, no parity, one stop bit and
    /// the FIFOs enabled (TF-A's `PL011_LINE_CONTROL`, 0x70), then
    /// `UARTCR` with the UART, its transmitter and its receiver enabled
    /// (0x301, as both write it).
    pub(crate) fn set_up_console(&mut self, baud: BaudRate, clocks: UartClocks) {
        let divisor = BaudDivisor::for_rate(clocks.uartclk, baud);
        self.write_register(UARTIBRD, divisor.integer(), clocks);
        self.write_register(UARTFBRD, divisor.fraction(), clocks);
        self.write_register(UARTLCR_H, LCR_WLEN_8 | LCR_FEN, clocks);
        self.write_register(UARTCR, CR_UARTEN | CR_TXE | CR_RXE, clocks);
    }

    /// 32 bit periods (DDI 0183F §3.4.4) at the latched divisor, in
    /// `UARTCLK` cycles: a bit is 16 periods of `Baud16`, whose period is the
    /// divisor `UARTIBRD + UARTFBRD / 64` in `UARTCLK` cycles (§2.2.3,
    /// §2.4.3, Figure 2-3), so 32 bits are 512 × that, 8 × the divisor in
    /// 64ths. `None` while the divisor makes no baud clock: `UARTIBRD` 0 is
    /// invalid, and `UARTIBRD` 0xFFFF with `UARTFBRD` above 0 aborts
    /// reception (§3.3.6).
    fn timeout_cycles(&self) -> Option<Cycles> {
        let ibrd = self.divisor.integer();
        if ibrd == 0 || (ibrd == IBRD_MASK && self.divisor.fraction() != 0) {
            None
        } else {
            Some(Cycles::new(8 * u64::from(self.divisor.sixty_fourths())))
        }
    }

    /// Start the receive timeout's count now: it expires at the first tick
    /// of the machine's clock by which 32 bit periods of `UARTCLK` time have
    /// passed, and not at all without a baud clock.
    fn start_rx_timeout(&mut self, clocks: UartClocks) {
        self.rx_timeout = self.timeout_cycles().map(|cycles| {
            clocks.machine.after(
                clocks
                    .machine
                    .rate()
                    .ticks_from_cycles_ceil(cycles, clocks.uartclk),
            )
        });
    }

    /// Let the receive timeout catch up to `clock`: a count that has run
    /// its 32 bit periods sets `RTRIS` and stops (DDI 0183F §3.4.4); the
    /// next character, or `RTIM` set again, starts another.
    pub fn update(&mut self, clock: VmClock) {
        if self.rx_timeout.is_some_and(|at| clock.now() >= at) {
            self.rx_timeout = None;
            self.int_level |= INT_RT;
        }
    }

    /// When the running receive timeout expires, if one runs.
    pub fn next_deadline(&self) -> Option<VmInstant> {
        self.rx_timeout
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
    /// disconnects the receive input.
    pub fn receive(&mut self, byte: u8, clocks: UartClocks) -> Received {
        if !self.can_receive() {
            return Received::NoRoom;
        }
        if !self.loopback() {
            self.receive_character(u32::from(byte), clocks);
        }
        Received::Taken
    }

    /// Enter the overrun state: a character was lost to a full FIFO.
    fn note_overrun(&mut self) {
        self.flags |= OVERRUN_PENDING;
    }

    /// Leave the overrun state, as the next character written to the FIFO
    /// does; whether it was in it, so that the character carries `OE`.
    fn take_overrun(&mut self) -> bool {
        let pending = self.flags & OVERRUN_PENDING != 0;
        self.flags &= !OVERRUN_PENDING;
        pending
    }

    /// The receive logic taking one character off the line, the receiver
    /// enabled (DDI 0183F §2.4.3; QEMU `pl011_fifo_rx_put`). A full FIFO
    /// keeps its entries and the character is lost to an overrun. Either
    /// way a character has been received, and while `RTIM` is set the
    /// receive timeout counts its 32 bit periods afresh from now (§3.4.4:
    /// "no further data is received over a 32-bit period").
    fn receive_character(&mut self, value: u32, clocks: UartClocks) {
        if value & DR_BE != 0 {
            self.int_level |= INT_BE;
        }
        if self.int_enabled & INT_RT != 0 {
            self.start_rx_timeout(clocks);
        }
        let depth = self.fifo_depth();
        if self.read_count == depth {
            self.note_overrun();
            self.rsr |= RSR_OE;
            self.int_level |= INT_OE;
            return;
        }
        let entry = if self.take_overrun() {
            value | DR_OE
        } else {
            value
        };
        let slot = (u32::from(self.read_pos) + self.read_count) & (depth - 1);
        self.read_fifo[slot as usize] = entry;
        self.read_count += 1;
        self.flags &= !FLAG_RXFE;
        if self.read_count == depth {
            self.flags |= FLAG_RXFF;
        }
        if self.read_count == self.rx_trigger() {
            self.int_level |= INT_RX;
        }
    }

    /// QEMU `pl011_loopback_tx`: in loopback the receiver hears what is
    /// transmitted, when it is enabled.
    fn loopback_tx(&mut self, value: u32, clocks: UartClocks) {
        if self.loopback() && self.receiving() {
            self.receive_character(value, clocks);
        }
    }

    /// QEMU `pl011_read_rxdata`. An empty FIFO returns the slot at the read
    /// position, whatever it last held.
    fn read_rxdata(&mut self) -> u32 {
        let depth = self.fifo_depth();
        self.flags &= !FLAG_RXFF;
        let c = self.read_fifo[usize::from(self.read_pos)];
        if self.read_count > 0 {
            self.read_count -= 1;
            self.read_pos = (self.read_pos + 1) & (depth as u8 - 1);
        }
        // Emptied, the FIFO has no receive timeout (§3.4.4).
        if self.read_count == 0 {
            self.flags |= FLAG_RXFE;
            self.int_level &= !INT_RT;
            self.rx_timeout = None;
        }
        if self.read_count < self.rx_trigger() {
            self.int_level &= !INT_RX;
        }
        self.rsr = (self.rsr & RSR_OE) | ((c >> 8) & RSR_CHARACTER_ERRORS);
        c
    }

    /// QEMU `pl011_reset_rx_fifo` and `pl011_reset_tx_fifo`, on every
    /// change of `FEN`. DDI 0183F flushes the transmit FIFO when `FEN` is
    /// cleared (§3.3.8, Note: "Flush the transmit FIFO by disabling bit 4
    /// (FEN)") and is silent on the receive FIFO and on setting `FEN`;
    /// emptying both either way is QEMU's choice. DDI 0183F gives no flush
    /// rule for the receive interrupts either (§3.4.2 and §3.4.4 name reads
    /// and `UARTICR`); this model's reading is that the emptied FIFO, below
    /// every trigger level and empty, holds no `RXRIS` and no `RTRIS`,
    /// where QEMU leaves both as they were. Nor does it say what a flush
    /// does to `TXRIS`; here it is a fall of the fill level like any other,
    /// judged against the trigger level of the FIFO being flushed
    /// ([`Pl011::tx_level_moved`]).
    fn reset_fifos(&mut self) {
        self.read_count = 0;
        self.read_pos = 0;
        let before = u32::from(self.tx_count);
        self.tx_count = 0;
        self.tx_level_moved(before);
        self.flags &= !(FLAG_RXFF | FLAG_TXFF | FLAG_BUSY);
        self.flags |= FLAG_RXFE | FLAG_TXFE;
        self.int_level &= !(INT_RT | INT_RX);
        self.rx_timeout = None;
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
    /// masking it (DDI 0183F Table 3-15, note a: "the mask acts as an enable
    /// for power saving"): clearing it clears `RTRIS` and stops the count,
    /// and setting it with the FIFO holding anything starts counting 32 bit
    /// periods from now.
    fn set_interrupt_mask(&mut self, v: u32, clocks: UartClocks) {
        let mask = v & IMSC_MASK;
        if mask & INT_RT == 0 {
            self.int_level &= !INT_RT;
            self.rx_timeout = None;
        } else if self.int_enabled & INT_RT == 0 && self.read_count > 0 {
            self.start_rx_timeout(clocks);
        }
        self.int_enabled = mask;
    }

    pub fn read(&mut self, offset: RegOffset, width: AccessWidth) -> u64 {
        match width {
            AccessWidth::Dword => {
                let low = self.read_register(offset);
                let high = self.read_register(RegOffset::new(offset.get() + 4));
                u64::from(low) | (u64::from(high) << 32)
            }
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => {
                u64::from(self.read_register(offset))
            }
        }
    }

    /// QEMU `pl011_read`.
    fn read_register(&mut self, offset: RegOffset) -> u32 {
        let register = register_at(offset);
        match register {
            UARTDR => self.read_rxdata(),
            UARTRSR => self.rsr,
            UARTFR => self.flags & FR_MASK,
            UARTILPR => self.ilpr,
            UARTIBRD => self.ibrd,
            UARTFBRD => self.fbrd,
            UARTLCR_H => self.lcr,
            UARTCR => self.cr,
            UARTIFLS => self.ifl,
            UARTIMSC => self.int_enabled,
            UARTRIS => self.int_level,
            UARTMIS => self.int_level & self.int_enabled,
            UARTDMACR => self.dmacr,
            _ if (UARTPERIPHID0..=UARTPCELLID3).contains(&register) => {
                u32::from(PL011_ID[((register.get() - UARTPERIPHID0.get()) >> 2) as usize])
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: RegOffset, width: AccessWidth, value: u64, clocks: UartClocks) {
        match width {
            AccessWidth::Dword => {
                self.write_register(offset, value as u32, clocks);
                self.write_register(
                    RegOffset::new(offset.get() + 4),
                    (value >> 32) as u32,
                    clocks,
                );
            }
            AccessWidth::Byte | AccessWidth::Half | AccessWidth::Word => {
                self.write_register(offset, value as u32, clocks);
            }
        }
    }

    /// QEMU `pl011_write`.
    fn write_register(&mut self, offset: RegOffset, v: u32, clocks: UartClocks) {
        match register_at(offset) {
            UARTDR => self.write_txdata(v as u8, clocks),
            UARTRSR => self.rsr = 0,
            UARTILPR => self.ilpr = v & ILPR_MASK,
            UARTIBRD => self.ibrd = v & IBRD_MASK,
            UARTFBRD => self.fbrd = v & FBRD_MASK,
            UARTLCR_H => {
                let v = v & LCR_MASK;
                if (self.lcr ^ v) & LCR_FEN != 0 {
                    self.reset_fifos();
                }
                // QEMU `pl011_loopback_break`; the host side of a break
                // (`CHR_IOCTL_SERIAL_SET_BREAK`) has no backend here.
                if (self.lcr ^ v) & LCR_BRK != 0 && v & LCR_BRK != 0 {
                    self.loopback_tx(DR_BE, clocks);
                }
                self.lcr = v;
                // The write strobe that updates `UARTIBRD` and `UARTFBRD`
                // (§3.3.7, Note). A count already running keeps the
                // deadline it started with, even where the new divisor
                // makes no baud clock.
                self.divisor = BaudDivisor::of_registers(self.ibrd, self.fbrd);
            }
            UARTCR => {
                self.cr = v & CR_MASK;
                self.loopback_mdmctrl();
                // Enabling transmission sends what waits in the FIFO.
                self.transmit(clocks);
            }
            UARTIFLS => self.ifl = v & IFLS_MASK,
            UARTIMSC => self.set_interrupt_mask(v, clocks),
            UARTICR => self.int_level &= !v,
            UARTDMACR => self.dmacr = v & DMACR_MASK,
            _ => {}
        }
    }

    /// The registers and both FIFOs, for a snapshot. Bytes already
    /// transmitted belong to the host and are not part of it.
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
        w.write_u8(self.read_pos)?;
        w.write_u32(self.read_count)?;
        w.write_u8(self.tx_count)?;
        w.write_bytes(&self.tx_fifo)?;
        w.write_u32(self.divisor.sixty_fourths())?;
        match self.rx_timeout {
            Some(at) => {
                w.write_bool(true)?;
                w.write_u64(at.ticks())
            }
            None => w.write_bool(false),
        }
    }

    /// Refuses a read position outside the FIFO, and a nonzero one while the
    /// FIFO is disabled (QEMU `pl011_post_load`'s checks), more entries in
    /// either FIFO than it holds, characters waiting to be transmitted while
    /// transmission is enabled, a latched divisor wider than its 22 bits,
    /// and a receive timeout running where none can: with the FIFO empty or
    /// `RTIM` clear. A count running with a latched divisor that makes no
    /// baud clock is a state the UART reaches (a count keeps its deadline
    /// when a divisor is latched after it started), and restores. Reserved
    /// register bits are dropped, as a guest write drops them, and
    /// `UARTFR`'s transmit bits are those of the transmit FIFO restored.
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
        let read_pos = r.read_u8()?;
        let read_count = r.read_u32()?;
        let tx_count = r.read_u8()?;
        let mut tx_fifo = [0u8; TX_FIFO_DEPTH];
        r.read_bytes(&mut tx_fifo)?;
        let divisor = r.read_u32()?;
        let rx_timeout = if r.read_bool()? {
            Some(VmInstant::from_ticks(r.read_u64()?))
        } else {
            None
        };
        if usize::from(read_pos) >= RX_FIFO_DEPTH {
            return Err(SnapError::Invalid("PL011 read position outside the receive FIFO"));
        }
        let depth = if lcr & LCR_FEN != 0 { RX_FIFO_DEPTH as u32 } else { 1 };
        if depth == 1 && read_pos != 0 {
            return Err(SnapError::Invalid("PL011 read position past the one-entry receive holding register"));
        }
        if read_count > depth {
            return Err(SnapError::Invalid("PL011 receive count beyond the receive FIFO's depth"));
        }
        if u32::from(tx_count) > depth {
            return Err(SnapError::Invalid("PL011 transmit count beyond the transmit FIFO's depth"));
        }
        let cr = cr & CR_MASK;
        if tx_count > 0 && cr & (CR_UARTEN | CR_TXE) == CR_UARTEN | CR_TXE {
            return Err(SnapError::Invalid(
                "PL011 characters waiting to be transmitted while transmission is enabled",
            ));
        }
        if divisor > BaudDivisor::MAX {
            return Err(SnapError::Invalid(
                "PL011 baud rate divisor wider than UARTIBRD and UARTFBRD",
            ));
        }
        let divisor = BaudDivisor(divisor);
        let int_enabled = int_enabled & IMSC_MASK;
        if rx_timeout.is_some() && (read_count == 0 || int_enabled & INT_RT == 0) {
            return Err(SnapError::Invalid(
                "PL011 receive timeout running with nothing to time",
            ));
        }
        let mut restored = Self {
            tx: RingBuffer::new(),
            tx_fifo,
            tx_count,
            read_fifo,
            read_pos,
            read_count,
            divisor,
            rx_timeout,
            flags: flags & (FR_MASK | OVERRUN_PENDING) & !(FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
            rsr: rsr & (RSR_OE | RSR_CHARACTER_ERRORS),
            ilpr: ilpr & ILPR_MASK,
            ibrd: ibrd & IBRD_MASK,
            fbrd: fbrd & FBRD_MASK,
            lcr: lcr & LCR_MASK,
            cr,
            ifl: ifl & IFLS_MASK,
            int_enabled,
            int_level: int_level & IMSC_MASK,
            dmacr: dmacr & DMACR_MASK,
        };
        restored.flags |= restored.tx_flags();
        restored.tx = core::mem::take(&mut self.tx);
        *self = restored;
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

    /// This board's `UARTCLK`, and a machine clock as fast: a cycle of one
    /// is a tick of the other.
    const UARTCLK: ClockHz = crate::board::UART_CLOCK;

    fn at(ticks: u64) -> UartClocks {
        UartClocks {
            machine: VmClock::new(VmInstant::from_ticks(ticks), UARTCLK),
            uartclk: UARTCLK,
        }
    }

    fn write(uart: &mut Pl011, n: u64, value: u32) {
        write_at(uart, n, value, 0);
    }

    fn write_at(uart: &mut Pl011, n: u64, value: u32, ticks: u64) {
        uart.write(reg(n), AccessWidth::Word, u64::from(value), at(ticks));
    }

    /// A host byte arriving at tick 0: whether the UART took it.
    fn rx(uart: &mut Pl011, byte: u8) -> bool {
        Pl011::receive(uart, byte, at(0)) == Received::Taken
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
            assert!(!rx(&mut uart, b'a'), "the host's byte waits");
            assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
        }
        write(&mut uart, CR, CR_UARTEN | CR_RXE);
        assert_eq!(uart.rx_room(), 1);
        assert!(rx(&mut uart, b'a'));
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
    }

    #[test]
    fn a_loopback_receiver_that_is_off_loses_what_is_transmitted() {
        let mut uart = Pl011::new();
        write(&mut uart, CR, 0x300 | CR_LBE);
        write(&mut uart, DR, u32::from(b'x'));
        assert_eq!(
            read(&mut uart, FR) & FLAG_RXFE,
            FLAG_RXFE,
            "UARTEN is clear"
        );
        write(&mut uart, CR, CR_UARTEN | 0x100 | CR_LBE);
        write(&mut uart, DR, u32::from(b'y'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "RXE is clear");
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'z'));
        assert_eq!(read(&mut uart, DR), u32::from(b'z'));
        assert_eq!(
            uart.drain_tx().collect::<std::vec::Vec<u8>>(),
            b"xyz",
            "each byte still left"
        );
    }

    fn sent(uart: &mut Pl011) -> std::vec::Vec<u8> {
        uart.drain_tx().collect()
    }

    #[test]
    fn the_transmit_fifo_holds_what_is_written_until_uarten_and_txe_are_set() {
        for off in [0x300, CR_UARTEN | CR_RXE, CR_TXE | CR_RXE] {
            let mut uart = Pl011::new();
            write(&mut uart, LCR_H, LCR_FEN);
            write(&mut uart, CR, off);
            write(&mut uart, IMSC, INT_TX);
            assert_eq!(
                read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
                FLAG_TXFE
            );
            for i in 0..TX_FIFO_DEPTH as u32 {
                write(&mut uart, DR, u32::from(b'a') + i);
                let full = if i + 1 == TX_FIFO_DEPTH as u32 {
                    FLAG_TXFF
                } else {
                    0
                };
                assert_eq!(
                    read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
                    FLAG_BUSY | full,
                    "UARTCR {off:#x}, {} queued",
                    i + 1
                );
            }
            write(&mut uart, DR, u32::from(b'!'));
            assert_eq!(sent(&mut uart), b"", "UARTCR {off:#x}: nothing leaves");
            assert_eq!(read(&mut uart, RIS) & INT_TX, 0, "nothing was transmitted");
            write(&mut uart, CR, ENABLED);
            assert_eq!(
                sent(&mut uart),
                b"abcdefghijklmnop",
                "UARTCR {off:#x}: all sixteen, in order; the write to the full FIFO was lost"
            );
            assert_eq!(
                read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
                FLAG_TXFE
            );
            assert_eq!(read(&mut uart, RIS) & INT_TX, INT_TX);
            assert!(uart.irq());
            write(&mut uart, DR, u32::from(b'z'));
            assert_eq!(sent(&mut uart), b"z", "enabled, a character goes at once");
        }
    }

    /// DDI 0183F §3.4.3 and §3.3.9: with the FIFO enabled, `TXRIS` is set
    /// when the fill level falls through the `TXIFLSEL` level ("Transmit
    /// FIFO becomes <= n/8 full"), not by a level already at or below it.
    #[test]
    fn the_transmit_interrupt_marks_the_fifo_draining_through_the_trigger_level() {
        // TXIFLSEL: 1/8, 1/4, 1/2, 3/4 and 7/8 of 16; reserved encodings
        // take the half-full level.
        for (txiflsel, level) in [
            (0, 2),
            (1, 4),
            (2, 8),
            (3, 12),
            (4, 14),
            (5, 8),
            (6, 8),
            (7, 8),
        ] {
            for queued in [level, level + 1] {
                let mut uart = Pl011::new();
                write(&mut uart, LCR_H, LCR_FEN);
                write(&mut uart, IFLS, txiflsel);
                write(&mut uart, IMSC, INT_TX);
                for i in 0..queued {
                    write(&mut uart, DR, u32::from(b'a') + i);
                }
                assert_eq!(
                    read(&mut uart, RIS) & INT_TX,
                    0,
                    "TXIFLSEL {txiflsel}: nothing has left"
                );
                write(&mut uart, CR, ENABLED);
                assert_eq!(sent(&mut uart).len(), queued as usize);
                let crossed = if queued > level { INT_TX } else { 0 };
                assert_eq!(
                    read(&mut uart, RIS) & INT_TX,
                    crossed,
                    "TXIFLSEL {txiflsel}, {queued} queued"
                );
                assert_eq!(uart.irq(), crossed != 0);
            }
        }
    }

    #[test]
    fn filling_the_transmit_fifo_above_the_trigger_level_clears_the_transmit_interrupt() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IFLS, 0); // 1/8: two entries
        for byte in *b"abc" {
            write(&mut uart, DR, u32::from(byte));
        }
        write(&mut uart, CR, ENABLED);
        assert_eq!(read(&mut uart, RIS) & INT_TX, INT_TX);
        write(&mut uart, CR, 0x300);
        write(&mut uart, DR, u32::from(b'd'));
        write(&mut uart, DR, u32::from(b'e'));
        assert_eq!(
            read(&mut uart, RIS) & INT_TX,
            INT_TX,
            "at the level: still set"
        );
        write(&mut uart, DR, u32::from(b'f'));
        assert_eq!(read(&mut uart, RIS) & INT_TX, 0, "above the level: cleared");
        write(&mut uart, CR, ENABLED);
        assert_eq!(
            read(&mut uart, RIS) & INT_TX,
            INT_TX,
            "draining through it again"
        );
        write(&mut uart, ICR, INT_TX);
        assert_eq!(read(&mut uart, RIS) & INT_TX, 0, "UARTICR clears it");
    }

    /// While transmission is enabled each character leaves as it is
    /// written, so the FIFO never rises above one entry: no transition
    /// through any trigger level, and `TXRIS` stays as it was. Enabling the
    /// UART and the interrupt with nothing written sets nothing (§3.4.3,
    /// Note).
    #[test]
    fn writes_through_an_enabled_fifo_cross_no_trigger_level() {
        for txiflsel in 0..8 {
            let mut uart = Pl011::new();
            write(&mut uart, LCR_H, LCR_FEN);
            write(&mut uart, IFLS, txiflsel);
            write(&mut uart, IMSC, INT_TX);
            write(&mut uart, CR, ENABLED);
            assert_eq!(
                read(&mut uart, RIS) & INT_TX,
                0,
                "enabled with nothing written"
            );
            for byte in *b"hello, world" {
                write(&mut uart, DR, u32::from(byte));
                assert_eq!(read(&mut uart, RIS) & INT_TX, 0, "TXIFLSEL {txiflsel}");
                assert_eq!(
                    read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
                    FLAG_TXFE,
                    "it left at once"
                );
            }
            assert_eq!(sent(&mut uart), b"hello, world");
        }
    }

    /// DDI 0183F §3.4.3, FIFO disabled: `TXRIS` is set when the holding
    /// register's character leaves it empty and cleared by a write that
    /// fills it.
    #[test]
    fn with_the_fifo_disabled_the_transmit_interrupt_marks_the_holding_register_emptying() {
        let mut uart = Pl011::new();
        write(&mut uart, IMSC, INT_TX);
        write(&mut uart, CR, ENABLED);
        assert_eq!(
            read(&mut uart, RIS) & INT_TX,
            0,
            "enabled with nothing written"
        );
        write(&mut uart, DR, u32::from(b'1'));
        assert_eq!(
            read(&mut uart, RIS) & INT_TX,
            INT_TX,
            "it filled and emptied"
        );
        assert!(uart.irq());
        write(&mut uart, ICR, INT_TX);
        write(&mut uart, DR, u32::from(b'2'));
        assert_eq!(read(&mut uart, RIS) & INT_TX, INT_TX, "each character");
        write(&mut uart, CR, 0x300);
        write(&mut uart, DR, u32::from(b'3'));
        assert_eq!(
            read(&mut uart, RIS) & INT_TX,
            0,
            "the write that fills it clears it"
        );
        write(&mut uart, CR, ENABLED);
        assert_eq!(read(&mut uart, RIS) & INT_TX, INT_TX, "sent once enabled");
        assert_eq!(sent(&mut uart), b"123");
    }

    /// DDI 0183F does not say what flushing the transmit FIFO does to
    /// `TXRIS`; here the flush is a fall of the fill level like any other.
    #[test]
    fn flushing_the_transmit_fifo_is_a_fall_of_its_level() {
        for (queued, crossed) in [(8, 0), (9, INT_TX)] {
            let mut uart = Pl011::new();
            write(&mut uart, LCR_H, LCR_FEN);
            for i in 0..queued {
                write(&mut uart, DR, i);
            }
            write(&mut uart, LCR_H, 0);
            assert_eq!(
                read(&mut uart, RIS) & INT_TX,
                crossed,
                "{queued} flushed past the half-full level"
            );
        }
        let mut uart = Pl011::new();
        write(&mut uart, DR, u32::from(b'x'));
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(
            read(&mut uart, RIS) & INT_TX,
            INT_TX,
            "the holding register emptied"
        );
    }

    #[test]
    fn with_the_fifo_disabled_the_transmit_holding_register_holds_one_character() {
        let mut uart = Pl011::new();
        write(&mut uart, DR, u32::from(b'1'));
        assert_eq!(
            read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
            FLAG_TXFF | FLAG_BUSY
        );
        write(&mut uart, DR, u32::from(b'2'));
        write(&mut uart, CR, ENABLED);
        assert_eq!(sent(&mut uart), b"1");
        assert_eq!(
            read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
            FLAG_TXFE
        );
    }

    #[test]
    fn changing_fen_flushes_the_transmit_fifo() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, DR, u32::from(b'x'));
        write(&mut uart, DR, u32::from(b'y'));
        write(&mut uart, LCR_H, 0);
        assert_eq!(
            read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
            FLAG_TXFE
        );
        write(&mut uart, CR, ENABLED);
        assert_eq!(sent(&mut uart), b"", "flushed characters never leave");
        write(&mut uart, CR, 0x300);
        write(&mut uart, DR, u32::from(b'w'));
        uart.reset();
        write(&mut uart, CR, ENABLED);
        assert_eq!(sent(&mut uart), b"", "a reset empties the transmit FIFO");
    }

    #[test]
    fn a_loopback_transmitter_that_is_off_sends_its_receiver_nothing() {
        let mut uart = Pl011::new();
        write(&mut uart, CR, CR_UARTEN | CR_RXE | CR_LBE);
        write(&mut uart, DR, u32::from(b'q'));
        assert_eq!(
            read(&mut uart, FR) & FLAG_RXFE,
            FLAG_RXFE,
            "TXE is clear: nothing on the line"
        );
        assert_eq!(sent(&mut uart), b"");
        write(&mut uart, CR, ENABLED | CR_LBE);
        assert_eq!(read(&mut uart, DR), u32::from(b'q'), "sent once TXE is set");
        assert_eq!(sent(&mut uart), b"q");
    }

    #[test]
    fn a_console_set_up_runs_at_the_rate_asked_with_everything_enabled() {
        let mut uart = Pl011::new();
        let baud = |rate| BaudRate::new(rate).expect("a rate");
        uart.set_up_console(baud(115_200), at(0));
        // 24 MHz × 4 / 115200 = 833 64ths: 13 + 1/64.
        assert_eq!(
            [9, 10, LCR_H, CR].map(|n| read(&mut uart, n)),
            [13, 1, 0x70, 0x301]
        );
        let mut slow = Pl011::new();
        slow.set_up_console(baud(1), at(0));
        assert_eq!(
            [9, 10].map(|n| read(&mut slow, n)),
            [0xFFFF, 0],
            "too slow for 16 bits: the slowest baud clock"
        );
        write(&mut uart, DR, u32::from(b'k'));
        assert_eq!(sent(&mut uart), b"k");
        assert!(rx(&mut uart, b'r'));
        assert_eq!(read(&mut uart, DR), u32::from(b'r'));
    }

    #[test]
    fn with_the_fifo_disabled_the_receiver_holds_one_byte() {
        let mut uart = enabled();
        assert!(rx(&mut uart, b'a'));
        assert!(!uart.can_receive());
        assert!(!rx(&mut uart, b'b'), "a second byte has nowhere to go");
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
                assert!(rx(&mut uart, i as u8));
            }
            assert_eq!(
                read(&mut uart, RIS) & INT_RX,
                0,
                "RXIFLSEL {rxiflsel}: below the level"
            );
            assert!(rx(&mut uart, 0xAA));
            assert_eq!(
                read(&mut uart, RIS) & INT_RX,
                INT_RX,
                "RXIFLSEL {rxiflsel}: at the level"
            );
            assert!(uart.irq());
            assert_eq!(read(&mut uart, DR), 0);
            assert_eq!(
                read(&mut uart, RIS) & INT_RX,
                0,
                "RXIFLSEL {rxiflsel}: a read takes it below"
            );
            assert!(!uart.irq());
        }
    }

    #[test]
    fn the_receive_interrupt_marks_a_crossing_of_the_level() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IFLS, 0); // 1/8: two entries
        for byte in [1, 2] {
            assert!(rx(&mut uart, byte));
        }
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX);
        write(&mut uart, ICR, INT_RX);
        assert!(rx(&mut uart, 3));
        assert_eq!(
            read(&mut uart, RIS) & INT_RX,
            0,
            "filling further crosses nothing"
        );
        // Back below the level, then up through it again.
        assert_eq!(read(&mut uart, DR), 1);
        assert_eq!(read(&mut uart, DR), 2);
        assert!(rx(&mut uart, 4));
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX);
    }

    #[test]
    fn with_the_fifo_disabled_each_character_raises_the_receive_interrupt() {
        let mut uart = enabled();
        write(&mut uart, IFLS, 4 << 3);
        assert!(rx(&mut uart, b'a'));
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX, "UARTIFLS has no say");
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(read(&mut uart, RIS) & INT_RX, 0, "a single read clears it");
        assert!(rx(&mut uart, b'b'));
        assert_eq!(read(&mut uart, RIS) & INT_RX, INT_RX);
    }

    /// 115200 baud from 24 MHz, `UARTIBRD` 13 and `UARTFBRD` 1, latched by
    /// a `UARTLCR_H` write enabling the FIFOs.
    fn timed() -> Pl011 {
        let mut uart = enabled();
        write(&mut uart, 9, 13);
        write(&mut uart, 10, 1);
        write(&mut uart, LCR_H, LCR_FEN);
        uart
    }

    /// 32 bit periods at `UARTIBRD` 13, `UARTFBRD` 1: 512 × (13 + 1/64)
    /// cycles of `UARTCLK`, which [`at`] makes ticks.
    const TIMEOUT_13_1: u64 = 512 * 13 + 512 / 64;

    /// A host byte arriving at `ticks`: whether the UART took it.
    fn rx_at(uart: &mut Pl011, byte: u8, ticks: u64) -> bool {
        uart.receive(byte, at(ticks)) == Received::Taken
    }

    fn update(uart: &mut Pl011, ticks: u64) {
        uart.update(at(ticks).machine);
    }

    #[test]
    fn the_receive_timeout_expires_32_bit_periods_after_the_last_arrival() {
        let mut uart = timed();
        write(&mut uart, IMSC, INT_RT);
        assert!(rx_at(&mut uart, b'a', 100));
        assert_eq!(
            uart.next_deadline(),
            Some(VmInstant::from_ticks(100 + TIMEOUT_13_1))
        );
        update(&mut uart, 100 + TIMEOUT_13_1 - 1);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0, "one tick short");
        assert!(rx_at(&mut uart, b'b', 1000));
        assert_eq!(
            uart.next_deadline(),
            Some(VmInstant::from_ticks(1000 + TIMEOUT_13_1)),
            "an arrival restarts it"
        );
        update(&mut uart, 100 + TIMEOUT_13_1);
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            0,
            "the first arrival's deadline is gone"
        );
        update(&mut uart, 1000 + TIMEOUT_13_1);
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            INT_RT,
            "32 bit periods without a character"
        );
        assert_eq!(read(&mut uart, MIS), INT_RT);
        assert!(uart.irq());
        assert_eq!(uart.next_deadline(), None, "it fires once");
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            INT_RT,
            "the FIFO still holds a byte"
        );
        assert_eq!(read(&mut uart, DR), u32::from(b'b'));
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            0,
            "reading it empty clears it"
        );
    }

    #[test]
    fn the_receive_timeout_counts_only_while_rtim_is_set() {
        let mut uart = timed();
        assert!(rx_at(&mut uart, b'a', 0));
        assert_eq!(uart.next_deadline(), None, "RTIM clear: no count");
        update(&mut uart, 100_000);
        assert_eq!(
            read(&mut uart, RIS) & (INT_RT | INT_RX),
            0,
            "RTIM clear: RTRIS cannot be set"
        );
        write_at(&mut uart, IMSC, INT_RT, 5000);
        assert_eq!(
            uart.next_deadline(),
            Some(VmInstant::from_ticks(5000 + TIMEOUT_13_1)),
            "setting RTIM starts it"
        );
        update(&mut uart, 5000 + TIMEOUT_13_1);
        assert_eq!(read(&mut uart, RIS) & INT_RT, INT_RT);
        write(&mut uart, ICR, INT_RT);
        write_at(&mut uart, IMSC, INT_RT, 20_000);
        assert_eq!(
            uart.next_deadline(),
            None,
            "RTIM was set already: nothing starts"
        );
        update(&mut uart, 50_000);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0, "nothing new arrived");
        assert!(rx_at(&mut uart, b'b', 60_000));
        write_at(&mut uart, IMSC, 0, 60_001);
        assert_eq!(uart.next_deadline(), None, "clearing RTIM stops it");
        update(&mut uart, 70_000);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0);
        write_at(&mut uart, IMSC, INT_RT, 70_000);
        assert_eq!(
            uart.next_deadline(),
            Some(VmInstant::from_ticks(70_000 + TIMEOUT_13_1))
        );
        update(&mut uart, 70_000 + TIMEOUT_13_1);
        write(&mut uart, IMSC, 0);
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0, "clearing RTIM clears it");
        write_at(&mut uart, IMSC, INT_RT, 80_000);
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(read(&mut uart, DR), u32::from(b'b'));
        assert_eq!(uart.next_deadline(), None, "emptying the FIFO stops it");
    }

    #[test]
    fn the_receive_timeout_runs_on_while_the_receiver_is_disabled() {
        for off in [CR_UARTEN | CR_TXE, 0x300] {
            let mut uart = timed();
            write(&mut uart, IMSC, INT_RT);
            assert!(rx_at(&mut uart, b'a', 100));
            write_at(&mut uart, CR, off, 200);
            assert_eq!(
                uart.next_deadline(),
                Some(VmInstant::from_ticks(100 + TIMEOUT_13_1)),
                "UARTCR {off:#x}"
            );
            update(&mut uart, 100 + TIMEOUT_13_1);
            assert_eq!(
                read(&mut uart, RIS) & INT_RT,
                INT_RT,
                "UARTCR {off:#x}: the FIFO still holds a character"
            );
        }
    }

    #[test]
    fn the_bit_period_is_the_divisor_the_last_lcr_h_write_latched() {
        let deadline = |uart: &mut Pl011| {
            assert_eq!(
                read(uart, DR) & 0xFF,
                u32::from(b'x'),
                "the FIFO is emptied for the next count"
            );
            assert!(rx_at(uart, b'x', 0));
            uart.next_deadline().map(VmInstant::ticks)
        };
        let mut uart = timed();
        write(&mut uart, IMSC, INT_RT);
        assert!(rx_at(&mut uart, b'x', 0));
        write(&mut uart, 9, 26);
        assert_eq!(
            deadline(&mut uart),
            Some(TIMEOUT_13_1),
            "UARTIBRD alone changes nothing (§3.3.7)"
        );
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(
            deadline(&mut uart),
            Some(512 * 26 + 8),
            "the UARTLCR_H write latches 26 + 1/64"
        );
        write(&mut uart, LCR_H, LCR_FEN | LCR_WLEN_8 | (1 << 3) | (1 << 1));
        assert_eq!(
            deadline(&mut uart),
            Some(512 * 26 + 8),
            "word length, stop bits and parity leave the bit period"
        );
        write(&mut uart, 10, 0);
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(deadline(&mut uart), Some(512 * 26));
        write(&mut uart, 9, 0);
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(deadline(&mut uart), None, "UARTIBRD 0 gives no baud clock");
        write(&mut uart, 9, 0xFFFF);
        write(&mut uart, 10, 1);
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(
            deadline(&mut uart),
            None,
            "0xFFFF with a fraction aborts reception"
        );
        write(&mut uart, 10, 0);
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(deadline(&mut uart), Some(512 * 0xFFFF));
    }

    #[test]
    fn the_receive_timeout_ends_at_the_first_whole_tick_after_32_bit_periods() {
        // At 50 million ticks a second, 6664 cycles of 24 MHz are
        // 13883.3 ticks: the timeout is seen at tick 13884.
        let mut uart = timed();
        write(&mut uart, IMSC, INT_RT);
        let rate = ClockHz::new(50_000_000).expect("non-zero");
        let clocks = UartClocks {
            machine: VmClock::new(VmInstant::from_ticks(7), rate),
            uartclk: UARTCLK,
        };
        assert_eq!(uart.receive(b'a', clocks), Received::Taken);
        assert_eq!(
            uart.next_deadline(),
            Some(VmInstant::from_ticks(7 + 13_884))
        );
        uart.update(VmClock::new(VmInstant::from_ticks(7 + 13_883), rate));
        assert_eq!(read(&mut uart, RIS) & INT_RT, 0);
        uart.update(VmClock::new(VmInstant::from_ticks(7 + 13_884), rate));
        assert_eq!(read(&mut uart, RIS) & INT_RT, INT_RT);
    }

    #[test]
    fn changing_fen_empties_the_receive_fifo() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        assert!(rx(&mut uart, 1));
        assert!(rx(&mut uart, 2));
        write(&mut uart, LCR_H, 0);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
    }

    #[test]
    fn emptying_the_fifo_through_fen_clears_the_receive_interrupts() {
        // FIFO enabled, then disabled: two entries at the 1/8 level.
        let mut uart = timed();
        write(&mut uart, IFLS, 0);
        write(&mut uart, IMSC, INT_RT | INT_RX);
        assert!(rx(&mut uart, 1));
        assert!(rx(&mut uart, 2));
        update(&mut uart, TIMEOUT_13_1);
        assert_eq!(read(&mut uart, RIS) & (INT_RT | INT_RX), INT_RT | INT_RX);
        write(&mut uart, LCR_H, 0);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            0,
            "an empty FIFO has no receive timeout"
        );
        assert_eq!(
            read(&mut uart, RIS) & INT_RX,
            0,
            "an empty FIFO is below the trigger level"
        );
        assert!(!uart.irq());
        // FIFO disabled, then enabled: the holding register.
        assert!(rx_at(&mut uart, 3, 10_000));
        update(&mut uart, 10_000 + TIMEOUT_13_1);
        assert_eq!(read(&mut uart, RIS) & (INT_RT | INT_RX), INT_RT | INT_RX);
        write_at(&mut uart, IMSC, INT_RX, 20_000);
        write_at(&mut uart, IMSC, INT_RT | INT_RX, 20_000);
        assert!(
            uart.next_deadline().is_some(),
            "RTIM set again with the holding register full"
        );
        write(&mut uart, LCR_H, LCR_FEN);
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE);
        assert_eq!(read(&mut uart, RIS) & (INT_RT | INT_RX), 0);
        assert_eq!(uart.next_deadline(), None, "and stops the count");
        assert!(!uart.irq());
    }

    #[test]
    fn an_empty_fifo_reads_the_slot_at_the_read_position() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        assert!(rx(&mut uart, b'p'));
        assert_eq!(read(&mut uart, DR), u32::from(b'p'));
        assert!(rx(&mut uart, b'q'));
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
        // The bits DDI 0183F defines in each read/write register.
        for (n, defined) in [
            (8, 0xFF),    // UARTILPR: 8 bits (Table 3-1), ILPDVSR [7:0] (Table 3-5)
            (9, 0xFFFF),  // UARTIBRD: 16 bits (Table 3-1), BAUD DIVINT [15:0] (Table 3-6)
            (10, 0x3F),   // UARTFBRD: 6 bits (Table 3-1), BAUD DIVFRAC [5:0] (Table 3-7)
            (11, 0xFF),   // UARTLCR_H: [15:8] reserved (Table 3-10)
            (12, 0xFF87), // UARTCR: [6:3] reserved (Table 3-12)
            (13, 0x3F),   // UARTIFLS: [15:6] reserved (Table 3-13)
            (14, 0x7FF),  // UARTIMSC: [15:11] reserved (Table 3-14)
            (18, 0x7),    // UARTDMACR: [15:3] reserved (Table 3-18)
        ] {
            write(&mut uart, n, 0xFFFF_FFFF);
            assert_eq!(read(&mut uart, n), defined, "register {:#x}", n * 4);
        }
        write(&mut uart, ICR, 0xFFFF_FFFF);
        write(&mut uart, LCR_H, 0);
        write(&mut uart, LCR_H, LCR_BRK);
        // A break: data 0 with BE, bit 10 (Table 3-2); UARTRSR.BE is bit 2
        // (Table 3-3), whose bits [7:4], unpredictable when read, read as
        // zero here.
        assert_eq!(read(&mut uart, DR), 0x400);
        assert_eq!(read(&mut uart, RSR), 0x4, "UARTRSR keeps four bits");
        // UARTRIS: [15:11] reserved (Table 3-15).
        assert_eq!(read(&mut uart, RIS) & !0x7FF, 0);
        write(&mut uart, FR, 0);
        // UARTFR: [15:9] reserved (Table 3-4).
        assert_eq!(
            read(&mut uart, FR) & !0x1FF,
            0,
            "the flag register ignores writes"
        );
    }

    #[test]
    fn loopback_returns_transmitted_bytes_and_drops_host_input() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'L'));
        assert_eq!(uart.drain_tx().collect::<std::vec::Vec<u8>>(), b"L", "the byte still leaves");
        assert!(rx(&mut uart, b'h'), "host input is consumed");
        assert_eq!(read(&mut uart, DR), u32::from(b'L'));
        assert_eq!(read(&mut uart, FR) & FLAG_RXFE, FLAG_RXFE, "and dropped");
        write(&mut uart, CR, ENABLED);
        assert!(rx(&mut uart, b'h'));
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
        assert_eq!(
            read(&mut uart, DR),
            DR_OE | 0x41,
            "the next character carries the overrun"
        );
        assert_eq!(
            read(&mut uart, FR) & FLAG_RXFE,
            FLAG_RXFE,
            "the lost character never arrives"
        );
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
        assert_eq!(
            read(&mut uart, DR),
            u32::from(b'1'),
            "the unread byte survived"
        );
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
        assert_eq!(
            read(&mut uart, RIS) & ints,
            ints,
            "entering loopback changed every line"
        );
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
        assert_eq!(
            read(&mut uart, RIS) & INT_BE,
            0,
            "a receiver that is off hears no break"
        );
    }

    /// DDI 0183F §3.4.5: the error interrupts are asserted when the error
    /// occurs in the reception. A break that arrives at a full holding
    /// register is detected, so `BERIS` rises, and its character is lost to
    /// the overrun, so `OERIS` rises and the unread character stays.
    #[test]
    fn a_break_lost_to_an_overrun_raises_beris_and_oeris() {
        let mut uart = enabled();
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, DR, u32::from(b'x'));
        write(&mut uart, IMSC, INT_BE | INT_OE);
        write(&mut uart, LCR_H, LCR_BRK);
        assert_eq!(read(&mut uart, RIS) & (INT_BE | INT_OE), INT_BE | INT_OE);
        assert!(uart.irq());
        assert_eq!(
            read(&mut uart, DR),
            u32::from(b'x'),
            "the unread character stays, with no break"
        );
        assert_eq!(
            read(&mut uart, RSR),
            RSR_OE,
            "UARTRSR has the overrun, and no break error"
        );
        assert_eq!(
            read(&mut uart, FR) & FLAG_RXFE,
            FLAG_RXFE,
            "the break character never arrives"
        );
        write(&mut uart, LCR_H, 0);
        write(&mut uart, DR, u32::from(b'y'));
        assert_eq!(
            read(&mut uart, DR),
            DR_OE | u32::from(b'y'),
            "the next character carries the overrun"
        );
    }

    /// DDI 0183F §3.4.4: with the FIFO disabled, the receive timeout counts
    /// for the character in the holding register, and reading it clears
    /// `RTRIS` ("by reading the holding register").
    #[test]
    fn with_the_fifo_disabled_the_receive_timeout_is_cleared_by_reading_the_holding_register() {
        let mut uart = timed();
        write(&mut uart, LCR_H, 0);
        write(&mut uart, IMSC, INT_RT);
        assert!(rx_at(&mut uart, b'a', 100));
        assert!(
            !uart.can_receive(),
            "one character fills the holding register"
        );
        update(&mut uart, 100 + TIMEOUT_13_1);
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            INT_RT,
            "32 bit periods without a character"
        );
        assert!(uart.irq());
        assert_eq!(read(&mut uart, DR), u32::from(b'a'));
        assert_eq!(
            read(&mut uart, RIS) & INT_RT,
            0,
            "reading the holding register clears it"
        );
        assert!(!uart.irq());
    }

    /// `TXRIS` marks a transition of the transmit FIFO's level through the
    /// trigger level (DDI 0183F §3.4.3, §3.3.9), so a write of
    /// `UARTIFLS.TXIFLSEL` alone, which moves the level and not the FIFO,
    /// changes nothing: not `TXRIS` set, not `TXRIS` clear, not the line.
    #[test]
    fn a_txiflsel_write_alone_changes_nothing() {
        for drained in [false, true] {
            for held in [1, 5, 15] {
                let mut uart = Pl011::new();
                write(&mut uart, LCR_H, LCR_FEN);
                write(&mut uart, IFLS, 0); // 1/8: two entries
                write(&mut uart, IMSC, INT_TX);
                if drained {
                    // Three entries drain through the two-entry level.
                    for byte in *b"abc" {
                        write(&mut uart, DR, u32::from(byte));
                    }
                    write(&mut uart, CR, ENABLED);
                    write(&mut uart, CR, 0x300);
                }
                for i in 0..held {
                    write(&mut uart, DR, u32::from(b'0') + i);
                }
                let ris = read(&mut uart, RIS);
                let irq = uart.irq();
                assert_eq!(
                    ris & INT_TX != 0,
                    drained && held <= 2,
                    "{held} held, drained {drained}"
                );
                for txiflsel in [4, 0, 2, 7, 1, 3] {
                    write(&mut uart, IFLS, txiflsel);
                    assert_eq!(
                        read(&mut uart, RIS),
                        ris,
                        "TXIFLSEL {txiflsel}, {held} held, drained {drained}"
                    );
                    assert_eq!(uart.irq(), irq);
                }
            }
        }
    }

    /// Linux's `pl011_tx_chars` writes a burst through an enabled FIFO,
    /// stopping only when `UARTFR.TXFF` is set: with characters leaving as
    /// they are written, a burst longer than the FIFO goes out whole and
    /// in order, and `TXFF` is never seen.
    #[test]
    fn a_burst_longer_than_the_fifo_goes_out_whole() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_WLEN_8 | LCR_FEN);
        write(&mut uart, IMSC, INT_TX);
        write(&mut uart, CR, ENABLED);
        let burst: std::vec::Vec<u8> = (0..64).map(|i| b'A' + i % 26).collect();
        for &byte in &burst {
            assert_eq!(read(&mut uart, FR) & FLAG_TXFF, 0, "the FIFO is never full");
            write(&mut uart, DR, u32::from(byte));
        }
        assert_eq!(sent(&mut uart), burst);
        assert_eq!(
            read(&mut uart, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
            FLAG_TXFE
        );
    }

    #[test]
    fn an_eight_byte_access_is_two_registers() {
        let mut uart = enabled();
        write(&mut uart, CR, ENABLED | CR_LBE);
        write(&mut uart, LCR_H, LCR_BRK);
        // UARTDR first, which pops the break and sets UARTRSR, then UARTRSR.
        let pair = uart.read(reg(0), AccessWidth::Dword);
        assert_eq!(pair, u64::from(DR_BE) | (u64::from(DR_BE >> 8) << 32));
        uart.write(reg(13), AccessWidth::Dword, 0x0000_0050_0000_0012, at(0));
        assert_eq!(read(&mut uart, 13), 0x12);
        assert_eq!(read(&mut uart, IMSC), 0x50);
    }

    #[test]
    fn a_narrower_access_reaches_the_register_that_holds_it() {
        let mut uart = Pl011::new();
        uart.write(
            RegOffset::new(0x31),
            AccessWidth::Byte,
            u64::from(ENABLED),
            at(0),
        );
        assert_eq!(
            uart.read(RegOffset::new(0x32), AccessWidth::Half),
            u64::from(ENABLED),
            "UARTCR"
        );
        // UARTPeriphID0..3 and UARTPCellID0..3, 0xFE0 to 0xFFC.
        let id: std::vec::Vec<u64> = (0..8)
            .map(|i| uart.read(RegOffset::new(0xFE0 + 4 * i + 3), AccessWidth::Byte))
            .collect();
        assert_eq!(id, [0x11, 0x10, 0x14, 0x00, 0x0D, 0xF0, 0x05, 0xB1]);
        assert_eq!(
            uart.read(RegOffset::new(0xFDC), AccessWidth::Word),
            0,
            "below the identification registers"
        );
    }

    #[test]
    fn a_snapshot_round_trips_registers_and_queued_entries() {
        let mut uart = enabled();
        write(&mut uart, LCR_H, LCR_FEN);
        write(&mut uart, IMSC, 0x50);
        assert!(rx(&mut uart, b'w'));
        assert_eq!(read(&mut uart, DR), u32::from(b'w'));
        assert!(rx(&mut uart, b'x'));
        assert!(rx(&mut uart, b'y'));
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
        assert_eq!(
            read(&mut restored, FR) & !FR_MASK,
            0,
            "the state is not a flag"
        );
        assert_eq!(read(&mut restored, DR), u32::from(b'1'));
        write(&mut restored, DR, u32::from(b'3'));
        assert_eq!(read(&mut restored, DR), DR_OE | u32::from(b'3'));
        let mut quiet = std::vec::Vec::new();
        Pl011::new().save(&mut quiet).expect("save");
        assert_eq!(
            &quiet[..4],
            &(FLAG_RXFE | FLAG_TXFE).to_le_bytes(),
            "no overrun, no extra bit"
        );
    }

    #[test]
    fn a_snapshot_keeps_the_characters_waiting_to_be_transmitted() {
        let mut uart = Pl011::new();
        write(&mut uart, LCR_H, LCR_FEN);
        for byte in *b"abc" {
            write(&mut uart, DR, u32::from(byte));
        }
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let mut restored = Pl011::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(
            source.is_empty(),
            "the restore consumed exactly what the save wrote"
        );
        assert_eq!(
            read(&mut restored, FR) & (FLAG_TXFE | FLAG_TXFF | FLAG_BUSY),
            FLAG_BUSY
        );
        write(&mut restored, CR, ENABLED);
        assert_eq!(sent(&mut restored), b"abc");
    }

    #[test]
    fn a_snapshot_with_an_impossible_transmit_fifo_is_refused() {
        let mut uart = Pl011::new();
        write(&mut uart, DR, u32::from(b'a'));
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        // The registers, the receive FIFO, its read position and count.
        let count = 11 * 4 + RX_FIFO_DEPTH * 4 + 1 + 4;
        assert_eq!(image[count], 1);
        image[count] = 2;
        let mut source: &[u8] = &image;
        assert!(
            Pl011::new().restore(&mut source).is_err(),
            "FEN is clear: one character at most"
        );
        image[count] = 1;
        let cr = 6 * 4;
        image[cr..cr + 4].copy_from_slice(&ENABLED.to_le_bytes());
        let mut source: &[u8] = &image;
        assert!(
            Pl011::new().restore(&mut source).is_err(),
            "transmission enabled would have sent it"
        );
        image[cr..cr + 4].copy_from_slice(&0x300u32.to_le_bytes());
        let mut source: &[u8] = &image;
        assert!(Pl011::new().restore(&mut source).is_ok());
    }

    #[test]
    fn a_snapshot_keeps_the_latched_divisor_and_the_running_receive_timeout() {
        let mut uart = timed();
        write(&mut uart, IMSC, INT_RT);
        assert!(rx_at(&mut uart, b'a', 500));
        write(&mut uart, 9, 99);
        let mut image = std::vec::Vec::new();
        uart.save(&mut image).expect("save");
        let mut restored = Pl011::new();
        let mut source: &[u8] = &image;
        restored.restore(&mut source).expect("restore");
        assert!(
            source.is_empty(),
            "the restore consumed exactly what the save wrote"
        );
        assert_eq!(
            restored.next_deadline(),
            Some(VmInstant::from_ticks(500 + TIMEOUT_13_1))
        );
        assert_eq!(read(&mut restored, DR), u32::from(b'a'));
        assert!(rx_at(&mut restored, b'b', 0));
        assert_eq!(
            restored.next_deadline(),
            Some(VmInstant::from_ticks(TIMEOUT_13_1)),
            "13 + 1/64 still, not 99"
        );
    }

    #[test]
    fn a_snapshot_with_a_receive_timeout_that_cannot_run_is_refused() {
        // The registers, the receive FIFO, its read position and count, the
        // transmit FIFO and its count, then the divisor and the timeout.
        const DIVISOR: usize = 11 * 4 + RX_FIFO_DEPTH * 4 + 1 + 4 + 1 + TX_FIFO_DEPTH;
        let image_of = |uart: &Pl011| {
            let mut image = std::vec::Vec::new();
            uart.save(&mut image).expect("save");
            image
        };
        let refused = |image: &[u8]| {
            let mut source: &[u8] = image;
            Pl011::new().restore(&mut source).is_err()
        };
        let mut uart = timed();
        write(&mut uart, IMSC, INT_RT);
        assert!(rx_at(&mut uart, b'a', 0));
        let running = image_of(&uart);
        assert_eq!(running[DIVISOR..DIVISOR + 4], 833u32.to_le_bytes());
        assert_eq!(running[DIVISOR + 4], 1, "a timeout runs");
        assert!(!refused(&running));
        let mut wide = running.clone();
        wide[DIVISOR + 3] = 1;
        assert!(refused(&wide), "a divisor wider than 22 bits");
        let mut empty = running.clone();
        let count = 11 * 4 + RX_FIFO_DEPTH * 4 + 1;
        empty[count] = 0;
        assert!(refused(&empty), "a count with nothing in the FIFO");
        let mut unmasked = running;
        // UARTIMSC, the ninth register saved.
        unmasked[8 * 4..9 * 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(refused(&unmasked), "a count with RTIM clear");
    }

    /// A count keeps its deadline when a divisor without a baud clock is
    /// latched after it started; the snapshot of that UART restores to the
    /// same UART, which raises `RTRIS` when the unsaved one does.
    #[test]
    fn a_count_running_when_a_divisor_without_a_baud_clock_is_latched_saves_and_restores() {
        for (ibrd, fbrd) in [(0, 0), (0, 1), (0xFFFF, 1)] {
            let mut uart = timed();
            write(&mut uart, IMSC, INT_RT);
            assert!(rx_at(&mut uart, b'a', 100));
            write(&mut uart, 9, ibrd);
            write(&mut uart, 10, fbrd);
            write_at(&mut uart, LCR_H, LCR_FEN, 200);
            assert_eq!(
                uart.timeout_cycles(),
                None,
                "UARTIBRD {ibrd:#x}, UARTFBRD {fbrd}: no baud clock"
            );
            assert_eq!(
                uart.next_deadline(),
                Some(VmInstant::from_ticks(100 + TIMEOUT_13_1)),
                "the count keeps its deadline"
            );
            let mut image = std::vec::Vec::new();
            uart.save(&mut image).expect("save");
            let mut restored = Pl011::new();
            let mut source: &[u8] = &image;
            restored
                .restore(&mut source)
                .expect("a state the UART reaches restores");
            let mut again = std::vec::Vec::new();
            restored.save(&mut again).expect("save");
            assert_eq!(
                again, image,
                "UARTIBRD {ibrd:#x}, UARTFBRD {fbrd}: the same UART"
            );
            for u in [&mut uart, &mut restored] {
                update(u, 100 + TIMEOUT_13_1 - 1);
                assert_eq!(read(u, RIS) & INT_RT, 0);
                update(u, 100 + TIMEOUT_13_1);
                assert_eq!(
                    read(u, RIS) & INT_RT,
                    INT_RT,
                    "UARTIBRD {ibrd:#x}, UARTFBRD {fbrd}: RTRIS on time"
                );
                assert_eq!(read(u, DR), u32::from(b'a'));
                assert!(rx_at(u, b'b', 20_000));
                assert_eq!(
                    u.next_deadline(),
                    None,
                    "the next character starts no count without a baud clock"
                );
            }
            let (mut a, mut b) = (std::vec::Vec::new(), std::vec::Vec::new());
            uart.save(&mut a).expect("save");
            restored.save(&mut b).expect("save");
            assert_eq!(a, b, "both ran to the same state");
        }
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
        assert!(
            uart.restore(&mut source).is_err(),
            "FEN is clear: one entry at most"
        );
        image[count] = 1;
        let mut source: &[u8] = &image;
        assert!(uart.restore(&mut source).is_ok());
    }
}
