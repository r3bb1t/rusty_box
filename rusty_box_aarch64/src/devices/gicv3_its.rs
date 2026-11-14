//! The GICv3 Interrupt Translation Service: its control and translation
//! frames, its command queue, and its Device, Interrupt Translation and
//! Collection tables in guest memory, translating an event of a device into
//! a physical LPI at the Redistributor ([`super::gicv3_lpi`]).
//!
//! Provenance (R7): IHI 0069G 5.2 (the ITS and its tables), 5.3 (its
//! commands and their pseudocode), 5.4 (the pseudocode's helpers), 5.6
//! (enabling and disabling it), 12.18 (its register map) and 12.19 (its
//! registers), for a GICv3 ITS of physical LPIs only (`GITS_TYPER.Physical`
//! 1, `Virtual` 0: no GICv4, so no vPE table and no `V*` command). The
//! choices the specification leaves to an implementation:
//! - Sixteen bits of DeviceID (`GITS_TYPER.Devbits` 15), the width of the
//!   PCI Express Requester ID that Arm expects as a DeviceID (5.2.3; BSA
//!   DEN0094C H.2, an identity mapping), and sixteen bits of EventID
//!   (`ID_bits` 15): a PCI Express function's MSI-X table holds up to 2048
//!   vectors (11 bits), and the functions behind a PCI Express to PCI
//!   bridge share the bridge's requester ID, so one DeviceID carries the
//!   vectors of all of them; sixteen bits hold 32 full tables.
//! - Entries of 8 bytes in every table (`GITS_TYPER.ITT_entry_size` 7,
//!   `GITS_BASER<n>.Entry_Size` 7), whose format, IMPLEMENTATION DEFINED
//!   (5.2.1), is the board's, little-endian: a Device table entry's `Valid`
//!   in bit 63, its ITT's address in [51:8] and its `Size` (EventID bits
//!   minus one) in [4:0]; an Interrupt Translation Entry's `Valid` in bit
//!   63, its ICID in [47:32] and its physical INTID in [31:0]; a Collection
//!   table entry's `Valid` in bit 63 and its target, a processor number, in
//!   [35:0].
//! - Two tables, `GITS_BASER0` the Device table's and `GITS_BASER1` the
//!   Collection table's, as 12.19.1 recommends; `GITS_BASER2` to
//!   `GITS_BASER7` unimplemented (RES0). Both may be flat or two-level
//!   (`Indirect` writable, an implementation of flat tables alone being
//!   deprecated where a flat Device table takes many pages), of 4 KiB, 16
//!   KiB or 64 KiB pages; every collection is held in memory
//!   (`GITS_TYPER.HCC` 0, so `CCT` 0) and an ICID has 16 bits (`CIL` 0).
//!   A table's base address is aligned down to its page size (the first
//!   CONSTRAINED UNPREDICTABLE choice of 12.19.1).
//! - Collection targets are processor numbers (`GITS_TYPER.PTA` 0,
//!   `GICR_TYPER.Processor_Number`), the board's one processor being 0. An
//!   LPI whose collection names a processor no Redistributor serves is lost,
//!   as one sent to a Redistributor with `EnableLPIs` 0 is (12.11.2); `SYNC`
//!   of any target completes at once; `MOVI` and `MOVALL` move pending state
//!   from processor 0's Redistributor to a target no Redistributor serves by
//!   clearing it, and move none onto it from one.
//! - Every command executes in full as `GITS_CWRITER` is written (or the ITS
//!   is enabled, or a stall is retried), up to the write pointer, each
//!   atomically (5.2.9); nothing stays outstanding, so `SYNC` has nothing to
//!   wait for and `GITS_CTLR.Quiescent` reads 1 whenever the ITS is
//!   disabled (and 0 while it is enabled, as FEAT_GICv4p1 defines it).
//! - A command error (5.3.2) stalls the queue: `GITS_CREADR` stays at the
//!   command and `GITS_CREADR.Stalled` reads 1 until a write of
//!   `GITS_CWRITER.Retry` runs the command again; there are no System
//!   errors (`GITS_TYPER.SEIS` 0). Errors are those each command's text and
//!   pseudocode name; where the pseudocode goes on (`MOVI` of an event with
//!   no translation, or to a collection that is unmapped) the command
//!   completes and changes nothing. A `MAPTI` or `MAPI` of an INTID that is
//!   not an LPI the GIC has is an error, 1023 among them ("pINTID does not
//!   specify a valid LPI INTID"). A command of an opcode GICv3 has no
//!   command for (the GICv4 ones among them) is an error, and so is a
//!   command the queue holds outside RAM, which reads as zeros.
//! - A write of `GITS_CWRITER` beyond the queue leaves the queue invalid
//!   until one within it (the first choice of 12.19.5); a write of
//!   `GITS_CBASER` or a `GITS_BASER<n>` while the ITS is enabled
//!   (UNPREDICTABLE) is ignored.
//! - `GITS_TRANSLATER` takes a 16-bit and a 32-bit write, and the low word
//!   of a 64-bit one; a write of an EventID wider than sixteen bits is
//!   ignored (the second choice of 12.19.12). The DeviceID a write carries
//!   is its requester's: the board's processor's writes are the only ones
//!   that reach the ITS, the PCIe host having no device on it, and carry
//!   DeviceID 0, the board's own choice ([`PROCESSOR_DEVICE_ID`]).
//! - No unmapped-MSI reporting (`GITS_TYPER.UMSI` 0) and no `GITS_STATUSR`
//!   (optional, RAZ/WI); no MPAM, so `GITS_MPAMIDR`, `GITS_PARTIDR` and
//!   `GITS_MPIDR` are RES0.
//! - `GITS_IIDR` reads the GIC's identification, the distributor's
//!   `GICD_IIDR`, the ITS being part of that GIC; `GITS_PIDR0` to
//!   `GITS_PIDR4` follow the CoreSight scheme as the distributor's do, the
//!   size the ITS's two frames occupy.
//! - Registers decode by aligned word, an 8-byte access as two word
//!   accesses, the low first, which covers the 64-bit registers (their
//!   halves are accessible separately, 12.19); any other access reads as
//!   zero and writes nothing.
//!
//! A table, or the part of one, outside RAM reads as zeros and takes no
//! writes, as the board's devices reach RAM alone.

use rusty_box_core::SnapError;
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::gicv2::GicNames;
use super::gicv3::identification;
use super::gicv3_lpi::{Lpis, read_or_zeros};
use crate::bus::{AccessWidth, RegOffset};
use crate::devices::virtio::queue::GuestRam;

/// The DeviceID a write the board's processor makes to `GITS_TRANSLATER`
/// carries.
pub(crate) const PROCESSOR_DEVICE_ID: u32 = 0;

/// The control frame's registers (Table 12-33).
const GITS_CTLR: u64 = 0x0000;
const GITS_IIDR: u64 = 0x0004;
const GITS_TYPER: u64 = 0x0008;
const GITS_CBASER: u64 = 0x0080;
const GITS_CWRITER: u64 = 0x0088;
const GITS_CREADR: u64 = 0x0090;
const GITS_BASER: u64 = 0x0100;
/// `GITS_BASER7`, the last.
const GITS_BASER_LAST: u64 = 0x0138;
/// The identification registers at the top of the control frame.
const ID_REGISTERS: u64 = 0xFFD0;
/// The translation frame, the control frame's 64 KiB after it, and its one
/// register (Table 12-34).
const TRANSLATION_FRAME: u64 = 0x1_0000;
const GITS_TRANSLATER: u64 = TRANSLATION_FRAME + 0x0040;
/// What the frames occupy, as the identification scheme's `SIZE` counts
/// it: the log2 of their 4 KiB blocks, 128 KiB.
const SIZE_LOG2_BLOCKS: u32 = 5;

/// The bits of DeviceID and of EventID the ITS implements.
const DEVICE_ID_BITS: u32 = 16;
const EVENT_ID_BITS: u32 = 16;
/// The bytes of an entry of every table.
const ENTRY_BYTES: u64 = 8;
/// The bytes of a command (5.2.8).
const COMMAND_BYTES: u64 = 32;
/// `GITS_TYPER`: `Physical`, `ITT_entry_size`, `ID_bits` and `Devbits`;
/// every other field 0.
const TYPER: u64 = 1
    | (ENTRY_BYTES - 1) << 4
    | (EVENT_ID_BITS as u64 - 1) << 8
    | (DEVICE_ID_BITS as u64 - 1) << 13;

/// `GITS_CTLR.Enabled` and `Quiescent`.
const CTLR_ENABLED: u32 = 1 << 0;
const CTLR_QUIESCENT: u32 = 1 << 31;
/// `GITS_CBASER`'s fields: `Valid` [63], `InnerCache` [61:59],
/// `OuterCache` [55:53], `Physical_Address` [51:12], `Shareability`
/// [11:10] and `Size` [7:0].
const CBASER_FIELDS: u64 = 0xB8EF_FFFF_FFFF_FCFF;
const CBASER_VALID: u64 = 1 << 63;
/// The queue's base, 64 KiB aligned (5.2.8): bits [15:12] of the field are
/// taken as zero (the first choice of 12.19.2).
const CBASER_BASE: u64 = 0x000F_FFFF_FFFF_0000;
/// `GITS_CWRITER.Offset` and `GITS_CREADR.Offset`, bits [19:5].
const QUEUE_OFFSET: u64 = 0x000F_FFE0;
/// `GITS_CWRITER.Retry` and `GITS_CREADR.Stalled`.
const CWRITER_RETRY: u64 = 1 << 0;
const CREADR_STALLED: u64 = 1 << 0;
/// `GITS_BASER<n>`'s writable fields: `Valid` [63], `Indirect` [62],
/// `InnerCache` [61:59], `OuterCache` [55:53], `Physical_Address` [47:12],
/// `Shareability` [11:10], `Page_Size` [9:8] and `Size` [7:0].
const BASER_FIELDS: u64 = 0xF8E0_FFFF_FFFF_FFFF;
const BASER_VALID: u64 = 1 << 63;
const BASER_INDIRECT: u64 = 1 << 62;
const BASER_ADDRESS: u64 = 0x0000_FFFF_FFFF_F000;
/// `GITS_BASER<n>.Entry_Size`, read-only: 8 bytes, minus one.
const BASER_ENTRY_SIZE: u64 = (ENTRY_BYTES - 1) << 48;
/// A level 1 table entry's `Valid` and the bits of its level 2 table's
/// address (5.2.1).
const L1_VALID: u64 = 1 << 63;
const L1_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
/// Every entry's `Valid`.
const ENTRY_VALID: u64 = 1 << 63;
/// A Device table entry's ITT address, bits [51:8] (as `MAPD` gives it).
const DTE_ITT: u64 = 0x000F_FFFF_FFFF_FF00;
const DTE_SIZE: u64 = 0x1F;
/// A command's `RDbase`, bits [51:16] of its doubleword: with `PTA` 0, a
/// processor number.
const RDBASE: u64 = 0x000F_FFFF_FFFF_0000;
/// The processor whose Redistributor the board has.
const PROCESSOR_0: u64 = 0;
/// The first LPI and the end of the LPIs the GIC has (`GICD_TYPER.IDbits`).
const LPI_FIRST: u32 = super::gicv3_lpi::LPI_BASE;
const LPI_END: u32 = 1 << super::gicv3_lpi::ID_BITS;

/// The tables the ITS has, each its `GITS_BASER<n>`.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
    /// `GITS_BASER0`, `Type` 0b001.
    Device,
    /// `GITS_BASER1`, `Type` 0b100.
    Collection,
}

impl Table {
    const ALL: [Self; 2] = [Self::Device, Self::Collection];

    const fn index(self) -> usize {
        match self {
            Self::Device => 0,
            Self::Collection => 1,
        }
    }

    /// Its register's read-only `Type` and `Entry_Size`.
    const fn read_only(self) -> u64 {
        BASER_ENTRY_SIZE
            | match self {
                Self::Device => 0b001 << 56,
                Self::Collection => 0b100 << 56,
            }
    }
}

/// Whether the ITS is enabled (`GITS_CTLR.Enabled`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItsState {
    /// Translations are ignored and no command is processed; quiescent.
    Disabled,
    Enabled,
}

/// Whether the command queue runs (`GITS_CREADR.Stalled`).
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queue {
    Running,
    /// A command error stopped it at `GITS_CREADR`.
    Stalled,
}

/// Where an entry of a table is.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The table cannot hold it: no table, or an ID past its end.
    OutOfRange,
    /// A two-level table's level 1 entry for it is not valid: it reads as
    /// unmapped and a write to it is discarded.
    Unbacked,
    At(u64),
}

/// What a table holds for an ID.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup<T> {
    OutOfRange,
    Unmapped,
    Mapped(T),
}

/// A Device table entry: the device's Interrupt Translation Table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceEntry {
    itt: u64,
    /// The bits of EventID it translates: `MAPD`'s `Size` plus one.
    event_bits: u32,
}

/// An Interrupt Translation Entry of a physical LPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranslationEntry {
    intid: u32,
    collection: u16,
}

/// A Collection table entry: the processor number its LPIs go to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CollectionEntry {
    target: u64,
}

/// How a command ended.
///
/// Closed (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Executed, or skipped where its pseudocode goes on: the read pointer
    /// moves on.
    Done,
    /// A command error (5.3.2), which stalls the queue.
    Error,
}

/// A command, its four doublewords (5.3).
#[derive(Debug, Clone, Copy)]
struct Command([u64; 4]);

impl Command {
    const CLEAR: u8 = 0x04;
    const DISCARD: u8 = 0x0F;
    const INT: u8 = 0x03;
    const INV: u8 = 0x0C;
    const INVALL: u8 = 0x0D;
    const MAPC: u8 = 0x09;
    const MAPD: u8 = 0x08;
    const MAPI: u8 = 0x0B;
    const MAPTI: u8 = 0x0A;
    const MOVALL: u8 = 0x0E;
    /// The opcodes of the commands GICv3 has (Table 5-6).
    const MOVI: u8 = 0x01;
    const SYNC: u8 = 0x05;

    fn opcode(self) -> u8 {
        self.0[0] as u8
    }

    fn device(self) -> u32 {
        (self.0[0] >> 32) as u32
    }

    fn event(self) -> u32 {
        self.0[1] as u32
    }

    /// `MAPTI`'s pINTID.
    fn physical_intid(self) -> u32 {
        (self.0[1] >> 32) as u32
    }

    fn collection(self) -> u16 {
        self.0[2] as u16
    }

    /// `MAPD`'s and `MAPC`'s `V`.
    fn valid(self) -> bool {
        self.0[2] & ENTRY_VALID != 0
    }

    /// The `RDbase` in doubleword `dw`: a processor number.
    fn target(self, dw: usize) -> u64 {
        (self.0[dw] & RDBASE) >> 16
    }
}

/// The Interrupt Translation Service.
#[derive(Clone)]
pub(crate) struct Its {
    state: ItsState,
    /// `GITS_CBASER`, its fields as written.
    cbaser: u64,
    /// `GITS_CWRITER.Offset`.
    cwriter: u64,
    /// `GITS_CREADR.Offset`.
    creadr: u64,
    queue: Queue,
    /// `GITS_BASER0` and `GITS_BASER1`, their writable fields as written.
    baser: [u64; 2],
}

impl Its {
    /// Out of reset (12.19): disabled and quiescent, no command queue or
    /// table valid, the queue's pointers at its start, and every field whose
    /// reset value is UNKNOWN zero.
    pub(crate) const fn new() -> Self {
        Self {
            state: ItsState::Disabled,
            cbaser: 0,
            cwriter: 0,
            creadr: 0,
            queue: Queue::Running,
            baser: [0; 2],
        }
    }

    // ----- registers -----

    /// A read of the ITS's frames, the GIC named `names`.
    pub(crate) fn read(&self, names: GicNames, offset: RegOffset, width: AccessWidth) -> u64 {
        let off = offset.get();
        match width {
            AccessWidth::Byte | AccessWidth::Half => 0,
            AccessWidth::Word => u64::from(self.word(names, off)),
            AccessWidth::Dword => {
                u64::from(self.word(names, off)) | u64::from(self.word(names, off + 4)) << 32
            }
        }
    }

    /// The word at `off`: a 32-bit register, or the half of a 64-bit one
    /// (its doubleword at `off & !7`) that `off` names.
    fn word(&self, names: GicNames, off: u64) -> u32 {
        if !off.is_multiple_of(4) {
            return 0;
        }
        let half = |value: u64| (value >> (32 * ((off >> 2) & 1))) as u32;
        match (off, off & !7) {
            (GITS_CTLR, _) => match self.state {
                ItsState::Enabled => CTLR_ENABLED,
                ItsState::Disabled => CTLR_QUIESCENT,
            },
            (GITS_IIDR, _) => names.gic.distributor.get(),
            (_, GITS_TYPER) => half(TYPER),
            (_, GITS_CBASER) => half(self.cbaser),
            (_, GITS_CWRITER) => half(self.cwriter),
            (_, GITS_CREADR) => half(self.creadr_value()),
            (_, GITS_BASER..=GITS_BASER_LAST) => {
                half(self.baser_value(((off - GITS_BASER) / 8) as usize))
            }
            (ID_REGISTERS..=0xFFFC, _) => {
                identification(names, off - ID_REGISTERS, SIZE_LOG2_BLOCKS)
            }
            // The reserved and unimplemented registers, `GITS_STATUSR`, and
            // the translation frame's, whose one register is write-only.
            _ => 0,
        }
    }

    /// `GITS_CREADR`: the offset and `Stalled`.
    fn creadr_value(&self) -> u64 {
        self.creadr
            | match self.queue {
                Queue::Stalled => CREADR_STALLED,
                Queue::Running => 0,
            }
    }

    /// `GITS_BASER<n>`: a table's fields as written with its type and entry
    /// size, or 0 for one the ITS does not have.
    fn baser_value(&self, n: usize) -> u64 {
        match Table::ALL.get(n) {
            Some(table) => self.baser[table.index()] | table.read_only(),
            None => 0,
        }
    }

    /// A write of the ITS's frames, the commands it starts executed and a
    /// translation made at once, against the Redistributor's `lpis` and
    /// guest `ram`.
    pub(crate) fn write(
        &mut self,
        lpis: &mut Lpis,
        ram: &mut GuestRam<'_>,
        offset: RegOffset,
        width: AccessWidth,
        value: u64,
    ) {
        let off = offset.get();
        if off == GITS_TRANSLATER {
            let event = match width {
                AccessWidth::Half => value as u32 & 0xFFFF,
                AccessWidth::Word | AccessWidth::Dword => value as u32,
                AccessWidth::Byte => return,
            };
            self.translate(lpis, ram, PROCESSOR_DEVICE_ID, event);
            return;
        }
        match width {
            AccessWidth::Byte | AccessWidth::Half => {}
            AccessWidth::Word => self.write_word(lpis, ram, off, value as u32),
            AccessWidth::Dword => {
                self.write_word(lpis, ram, off, value as u32);
                self.write_word(lpis, ram, off + 4, (value >> 32) as u32);
            }
        }
    }

    fn write_word(&mut self, lpis: &mut Lpis, ram: &mut GuestRam<'_>, off: u64, v: u32) {
        if !off.is_multiple_of(4) {
            return;
        }
        let shift = 32 * ((off >> 2) & 1);
        let with_half =
            |register: u64| (register & !(0xFFFF_FFFF << shift)) | u64::from(v) << shift;
        match (off, off & !7) {
            (GITS_CTLR, _) => {
                self.state = if v & CTLR_ENABLED != 0 {
                    ItsState::Enabled
                } else {
                    ItsState::Disabled
                };
                self.process(lpis, ram);
            }
            (GITS_CWRITER, _) => {
                self.cwriter = u64::from(v) & QUEUE_OFFSET;
                if u64::from(v) & CWRITER_RETRY != 0 {
                    self.queue = Queue::Running;
                }
                self.process(lpis, ram);
            }
            (_, GITS_CBASER) if self.state == ItsState::Disabled => {
                self.cbaser = with_half(self.cbaser) & CBASER_FIELDS;
                self.creadr = 0;
                self.queue = Queue::Running;
            }
            (_, GITS_BASER..=GITS_BASER_LAST) if self.state == ItsState::Disabled => {
                if let Some(table) = Table::ALL.get(((off - GITS_BASER) / 8) as usize) {
                    let i = table.index();
                    self.baser[i] = with_half(self.baser[i]) & BASER_FIELDS;
                }
            }
            // `GITS_CBASER` and `GITS_BASER<n>` while the ITS is enabled;
            // `GITS_IIDR`, `GITS_TYPER` and `GITS_CREADR`, read-only;
            // `GITS_CWRITER`'s high word, RES0; the rest, reserved,
            // unimplemented or RAZ/WI.
            _ => {}
        }
    }

    // ----- the command queue -----

    /// The queue's size in bytes: `GITS_CBASER.Size` plus one 4 KiB pages.
    fn queue_bytes(&self) -> u64 {
        ((self.cbaser & 0xFF) + 1) * 0x1000
    }

    /// Execute the commands from `GITS_CREADR` to `GITS_CWRITER`, while the
    /// ITS is enabled, its queue valid and not stalled, and the write
    /// pointer within the queue.
    fn process(&mut self, lpis: &mut Lpis, ram: &mut GuestRam<'_>) {
        if self.state == ItsState::Disabled
            || self.cbaser & CBASER_VALID == 0
            || self.queue == Queue::Stalled
        {
            return;
        }
        let size = self.queue_bytes();
        if self.cwriter >= size || self.creadr >= size {
            return;
        }
        let base = self.cbaser & CBASER_BASE;
        while self.creadr != self.cwriter {
            let mut bytes = [0u8; COMMAND_BYTES as usize];
            read_or_zeros(ram, base + self.creadr, &mut bytes);
            let mut dw = [0u64; 4];
            for (d, chunk) in dw.iter_mut().zip(bytes.chunks_exact(8)) {
                let mut word = [0u8; 8];
                word.copy_from_slice(chunk);
                *d = u64::from_le_bytes(word);
            }
            match self.execute(lpis, ram, Command(dw)) {
                Outcome::Done => self.creadr = (self.creadr + COMMAND_BYTES) % size,
                Outcome::Error => {
                    self.queue = Queue::Stalled;
                    return;
                }
            }
        }
    }

    /// Execute one command, as its pseudocode does (5.3).
    fn execute(&self, lpis: &mut Lpis, ram: &mut GuestRam<'_>, cmd: Command) -> Outcome {
        match cmd.opcode() {
            Command::MAPD => self.mapd(ram, cmd),
            Command::MAPC => self.mapc(ram, cmd),
            Command::MAPTI => self.map_translation(ram, cmd, cmd.physical_intid()),
            Command::MAPI => self.map_translation(ram, cmd, cmd.event()),
            Command::INT => {
                self.on_translation(ram, cmd, |its, ram, ite| its.set_pending(lpis, ram, ite))
            }
            Command::CLEAR => {
                self.on_translation(ram, cmd, |its, ram, ite| its.clear_pending(lpis, ram, ite))
            }
            Command::INV => self.on_translation(ram, cmd, |its, ram, ite| {
                match its.collection(ram, ite.collection) {
                    Lookup::Mapped(cte) => {
                        if cte.target == PROCESSOR_0 {
                            lpis.invalidate(ram, ite.intid);
                        }
                        Outcome::Done
                    }
                    Lookup::OutOfRange | Lookup::Unmapped => Outcome::Error,
                }
            }),
            Command::DISCARD => self.discard(lpis, ram, cmd),
            Command::INVALL => match self.collection(ram, cmd.collection()) {
                Lookup::Mapped(cte) => {
                    if cte.target == PROCESSOR_0 {
                        lpis.invalidate_all(ram);
                    }
                    Outcome::Done
                }
                Lookup::OutOfRange | Lookup::Unmapped => Outcome::Error,
            },
            Command::MOVI => self.movi(lpis, ram, cmd),
            Command::MOVALL => {
                move_all_pending(lpis, cmd.target(2), cmd.target(3));
                Outcome::Done
            }
            Command::SYNC => Outcome::Done,
            _ => Outcome::Error,
        }
    }

    /// `MAPD DeviceID, ITT_addr, Size, V`.
    fn mapd(&self, ram: &mut GuestRam<'_>, cmd: Command) -> Outcome {
        let entry = if cmd.valid() {
            let size = cmd.0[1] & DTE_SIZE;
            if size + 1 > u64::from(EVENT_ID_BITS) {
                // MAPD_ITTSIZE_OOR.
                return Outcome::Error;
            }
            ENTRY_VALID | (cmd.0[2] & DTE_ITT) | size
        } else {
            0
        };
        match self.device_slot(ram, cmd.device()) {
            Slot::At(address) => {
                store_entry(ram, address, entry);
                Outcome::Done
            }
            Slot::Unbacked => Outcome::Done,
            // MAPD_DEVICE_OOR.
            Slot::OutOfRange => Outcome::Error,
        }
    }

    /// `MAPC ICID, RDbase, V`.
    fn mapc(&self, ram: &mut GuestRam<'_>, cmd: Command) -> Outcome {
        let entry = if cmd.valid() {
            ENTRY_VALID | cmd.target(2)
        } else {
            0
        };
        match self.slot(ram, Table::Collection, u64::from(cmd.collection())) {
            Slot::At(address) => {
                store_entry(ram, address, entry);
                Outcome::Done
            }
            Slot::Unbacked => Outcome::Done,
            // MAPC_COLLECTION_OOR.
            Slot::OutOfRange => Outcome::Error,
        }
    }

    /// `MAPTI DeviceID, EventID, pINTID, ICID`, or `MAPI DeviceID, EventID,
    /// ICID` with `intid` the EventID.
    fn map_translation(&self, ram: &mut GuestRam<'_>, cmd: Command, intid: u32) -> Outcome {
        if self.slot(ram, Table::Collection, u64::from(cmd.collection())) == Slot::OutOfRange {
            // MAPTI_COLLECTION_OOR, MAPI_COLLECTION_OOR.
            return Outcome::Error;
        }
        let Lookup::Mapped(dte) = self.device(ram, cmd.device()) else {
            // _DEVICE_OOR, _UNMAPPED_DEVICE.
            return Outcome::Error;
        };
        if !event_in_range(cmd.event(), dte) || !(LPI_FIRST..LPI_END).contains(&intid) {
            // _ID_OOR, MAPTI_PHYSICALID_OOR.
            return Outcome::Error;
        }
        let entry = ENTRY_VALID | u64::from(cmd.collection()) << 32 | u64::from(intid);
        store_entry(ram, translation_address(dte, cmd.event()), entry);
        Outcome::Done
    }

    /// A command of a mapped event (`INT`, `CLEAR`, `INV`): `act` on its
    /// translation, or a command error where the device or the event has
    /// none.
    fn on_translation<F>(&self, ram: &mut GuestRam<'_>, cmd: Command, act: F) -> Outcome
    where
        F: FnOnce(&Self, &mut GuestRam<'_>, TranslationEntry) -> Outcome,
    {
        match self.event_translation(ram, cmd) {
            Some(ite) => act(self, ram, ite),
            // _DEVICE_OOR, _UNMAPPED_DEVICE, _ID_OOR, _UNMAPPED_INTERRUPT.
            None => Outcome::Error,
        }
    }

    /// The translation of a command's event: `None` where the device is out
    /// of range or unmapped, the event out of the device's range, or
    /// unmapped.
    fn event_translation(&self, ram: &GuestRam<'_>, cmd: Command) -> Option<TranslationEntry> {
        let Lookup::Mapped(dte) = self.device(ram, cmd.device()) else {
            return None;
        };
        if !event_in_range(cmd.event(), dte) {
            return None;
        }
        translation(ram, dte, cmd.event())
    }

    /// `DISCARD DeviceID, EventID`.
    fn discard(&self, lpis: &mut Lpis, ram: &mut GuestRam<'_>, cmd: Command) -> Outcome {
        let Lookup::Mapped(dte) = self.device(ram, cmd.device()) else {
            // DISCARD_DEVICE_OOR, DISCARD_UNMAPPED_DEVICE.
            return Outcome::Error;
        };
        if !event_in_range(cmd.event(), dte) {
            // DISCARD_ID_OOR.
            return Outcome::Error;
        }
        let Some(ite) = translation(ram, dte, cmd.event()) else {
            // DISCARD_UNMAPPED_INTERRUPT.
            return Outcome::Error;
        };
        match self.clear_pending(lpis, ram, ite) {
            Outcome::Done => {
                store_entry(ram, translation_address(dte, cmd.event()), 0);
                Outcome::Done
            }
            // DISCARD_ITE_INVALID.
            Outcome::Error => Outcome::Error,
        }
    }

    /// `MOVI DeviceID, EventID, ICID`.
    fn movi(&self, lpis: &mut Lpis, ram: &mut GuestRam<'_>, cmd: Command) -> Outcome {
        if self.slot(ram, Table::Collection, u64::from(cmd.collection())) == Slot::OutOfRange {
            // MOVI_COLLECTION_OOR.
            return Outcome::Error;
        }
        let Lookup::Mapped(dte) = self.device(ram, cmd.device()) else {
            // MOVI_DEVICE_OOR, MOVI_UNMAPPED_DEVICE.
            return Outcome::Error;
        };
        if !event_in_range(cmd.event(), dte) {
            // MOVI_ID_OOR.
            return Outcome::Error;
        }
        let Some(ite) = translation(ram, dte, cmd.event()) else {
            // MOVI_UNMAPPED_INTERRUPT: the pseudocode goes on.
            return Outcome::Done;
        };
        let Lookup::Mapped(from) = self.collection(ram, ite.collection) else {
            // MOVI_UNMAPPED_COLLECTION, of the event's collection.
            return Outcome::Error;
        };
        let Lookup::Mapped(to) = self.collection(ram, cmd.collection()) else {
            // MOVI_UNMAPPED_COLLECTION, of the command's: the pseudocode
            // goes on.
            return Outcome::Done;
        };
        if from.target != to.target && from.target == PROCESSOR_0 && lpis.is_pending(ite.intid) {
            lpis.clear_pending(ite.intid);
        }
        let entry = ENTRY_VALID | u64::from(cmd.collection()) << 32 | u64::from(ite.intid);
        store_entry(ram, translation_address(dte, cmd.event()), entry);
        Outcome::Done
    }

    /// `SetPendingState(ite)`: the LPI made pending at its collection's
    /// Redistributor; an error where the collection is unmapped.
    fn set_pending(&self, lpis: &mut Lpis, ram: &GuestRam<'_>, ite: TranslationEntry) -> Outcome {
        match self.collection(ram, ite.collection) {
            Lookup::Mapped(cte) => {
                if cte.target == PROCESSOR_0 {
                    lpis.set_pending(ite.intid);
                }
                Outcome::Done
            }
            Lookup::OutOfRange | Lookup::Unmapped => Outcome::Error,
        }
    }

    /// `ClearPendingState(ite)`, as [`Its::set_pending`].
    fn clear_pending(&self, lpis: &mut Lpis, ram: &GuestRam<'_>, ite: TranslationEntry) -> Outcome {
        match self.collection(ram, ite.collection) {
            Lookup::Mapped(cte) => {
                if cte.target == PROCESSOR_0 {
                    lpis.clear_pending(ite.intid);
                }
                Outcome::Done
            }
            Lookup::OutOfRange | Lookup::Unmapped => Outcome::Error,
        }
    }

    // ----- translation -----

    /// A write of `event` to `GITS_TRANSLATER` from device `device`: the
    /// LPI its translation names made pending, if the ITS is enabled and
    /// the device, the event and its collection are all mapped; otherwise
    /// the write is ignored (12.19.12).
    pub(crate) fn translate(&self, lpis: &mut Lpis, ram: &GuestRam<'_>, device: u32, event: u32) {
        if self.state == ItsState::Disabled || event >> EVENT_ID_BITS != 0 {
            return;
        }
        let Lookup::Mapped(dte) = self.device(ram, device) else {
            return;
        };
        if !event_in_range(event, dte) {
            return;
        }
        let Some(ite) = translation(ram, dte, event) else {
            return;
        };
        match self.set_pending(lpis, ram, ite) {
            Outcome::Done => {}
            // An event of an unmapped collection is ignored.
            Outcome::Error => {}
        }
    }

    // ----- the tables -----

    /// Where table `table`'s entry `id` is (5.2.1): in a flat table the
    /// `id`th entry; in a two-level table, the entry in the level 2 page
    /// the level 1 entry for `id` names.
    fn slot(&self, ram: &GuestRam<'_>, table: Table, id: u64) -> Slot {
        let baser = self.baser[table.index()];
        if baser & BASER_VALID == 0 {
            return Slot::OutOfRange;
        }
        let page = page_bytes(baser);
        let base = table_base(baser, page);
        let span = ((baser & 0xFF) + 1) * page;
        if baser & BASER_INDIRECT == 0 {
            let offset = id * ENTRY_BYTES;
            return if offset < span {
                Slot::At(base + offset)
            } else {
                Slot::OutOfRange
            };
        }
        let per_page = page / ENTRY_BYTES;
        let l1_offset = (id / per_page) * 8;
        if l1_offset >= span {
            return Slot::OutOfRange;
        }
        let l1 = load_entry(ram, base + l1_offset);
        if l1 & L1_VALID == 0 {
            return Slot::Unbacked;
        }
        Slot::At((l1 & L1_ADDRESS & !(page - 1)) + (id % per_page) * ENTRY_BYTES)
    }

    /// Where device `device`'s Device table entry is: out of range past the
    /// DeviceID bits the ITS implements.
    fn device_slot(&self, ram: &GuestRam<'_>, device: u32) -> Slot {
        if device >> DEVICE_ID_BITS != 0 {
            return Slot::OutOfRange;
        }
        self.slot(ram, Table::Device, u64::from(device))
    }

    /// Device `device`'s Device table entry.
    fn device(&self, ram: &GuestRam<'_>, device: u32) -> Lookup<DeviceEntry> {
        match self.device_slot(ram, device) {
            Slot::OutOfRange => Lookup::OutOfRange,
            Slot::Unbacked => Lookup::Unmapped,
            Slot::At(address) => {
                let entry = load_entry(ram, address);
                if entry & ENTRY_VALID == 0 {
                    Lookup::Unmapped
                } else {
                    Lookup::Mapped(DeviceEntry {
                        itt: entry & DTE_ITT,
                        event_bits: (entry & DTE_SIZE) as u32 + 1,
                    })
                }
            }
        }
    }

    /// Collection `icid`'s Collection table entry.
    fn collection(&self, ram: &GuestRam<'_>, icid: u16) -> Lookup<CollectionEntry> {
        match self.slot(ram, Table::Collection, u64::from(icid)) {
            Slot::OutOfRange => Lookup::OutOfRange,
            Slot::Unbacked => Lookup::Unmapped,
            Slot::At(address) => {
                let entry = load_entry(ram, address);
                if entry & ENTRY_VALID == 0 {
                    Lookup::Unmapped
                } else {
                    Lookup::Mapped(CollectionEntry {
                        target: entry & (RDBASE >> 16),
                    })
                }
            }
        }
    }

    // ----- snapshots -----

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bool(self.state == ItsState::Enabled)?;
        w.write_bool(self.queue == Queue::Stalled)?;
        w.write_u64(self.cbaser)?;
        w.write_u64(self.cwriter)?;
        w.write_u64(self.creadr)?;
        for baser in self.baser {
            w.write_u64(baser)?;
        }
        Ok(())
    }

    /// The registers an image holds, refused where one has a bit no field
    /// holds or the read pointer lies outside its queue.
    pub(crate) fn restore<R: SnapRead>(r: &mut R) -> SnapResult<Self> {
        let state = if r.read_bool()? {
            ItsState::Enabled
        } else {
            ItsState::Disabled
        };
        let queue = if r.read_bool()? {
            Queue::Stalled
        } else {
            Queue::Running
        };
        let cbaser = r.read_u64()?;
        let cwriter = r.read_u64()?;
        let creadr = r.read_u64()?;
        let mut baser = [0u64; 2];
        for entry in baser.iter_mut() {
            *entry = r.read_u64()?;
        }
        let its = Self {
            state,
            cbaser,
            cwriter,
            creadr,
            queue,
            baser,
        };
        if cbaser & !CBASER_FIELDS != 0
            || cwriter & !QUEUE_OFFSET != 0
            || creadr & !QUEUE_OFFSET != 0
            || baser.iter().any(|b| b & !BASER_FIELDS != 0)
        {
            return Err(SnapError::Invalid(
                "GITS register with a bit no field holds",
            ));
        }
        if creadr >= its.queue_bytes() {
            return Err(SnapError::Invalid(
                "GITS_CREADR outside the ITS command queue",
            ));
        }
        Ok(its)
    }
}

/// The page size a `GITS_BASER<n>` gives (`Page_Size`: 4 KiB, 16 KiB, and
/// 64 KiB for 0b10 and the reserved 0b11).
const fn page_bytes(baser: u64) -> u64 {
    match (baser >> 8) & 0b11 {
        0b00 => 0x1000,
        0b01 => 0x4000,
        _ => 0x1_0000,
    }
}

/// A table's base address: `Physical_Address`, which with 64 KiB pages
/// gives address bits [51:48] in register bits [15:12], aligned down to
/// the page size.
const fn table_base(baser: u64, page: u64) -> u64 {
    let address = baser & BASER_ADDRESS;
    let address = if page == 0x1_0000 {
        (address & !0xF000) | ((address >> 12) & 0xF) << 48
    } else {
        address
    };
    address & !(page - 1)
}

/// Whether `event` is one device table entry `dte` translates.
const fn event_in_range(event: u32, dte: DeviceEntry) -> bool {
    (event as u64) >> dte.event_bits == 0
}

/// Where event `event`'s Interrupt Translation Entry is in `dte`'s table.
const fn translation_address(dte: DeviceEntry, event: u32) -> u64 {
    dte.itt + event as u64 * ENTRY_BYTES
}

/// Event `event`'s Interrupt Translation Entry in `dte`'s table, if valid.
fn translation(ram: &GuestRam<'_>, dte: DeviceEntry, event: u32) -> Option<TranslationEntry> {
    let entry = load_entry(ram, translation_address(dte, event));
    (entry & ENTRY_VALID != 0).then_some(TranslationEntry {
        intid: entry as u32,
        collection: (entry >> 32) as u16,
    })
}

/// `MoveAllPendingState(rd1, rd2)`: with the board's one Redistributor,
/// processor 0's, every LPI pending there is cleared when the move is to
/// another target, which no Redistributor serves; one from another target
/// brings nothing.
fn move_all_pending(lpis: &mut Lpis, from: u64, to: u64) {
    if from != to && from == PROCESSOR_0 {
        lpis.clear_all_pending();
    }
}

/// The 8-byte entry at `address`, zero outside RAM.
fn load_entry(ram: &GuestRam<'_>, address: u64) -> u64 {
    let mut bytes = [0u8; 8];
    read_or_zeros(ram, address, &mut bytes);
    u64::from_le_bytes(bytes)
}

/// Store the 8-byte entry `entry` at `address`; nothing outside RAM.
fn store_entry(ram: &mut GuestRam<'_>, address: u64, entry: u64) {
    ram.store(address, &entry.to_le_bytes());
}

#[cfg(test)]
#[path = "../../tests/unit/gicv3_its.rs"]
mod tests;
