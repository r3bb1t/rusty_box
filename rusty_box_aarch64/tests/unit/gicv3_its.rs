//! The ITS against IHI 0069G: its registers' reset values, fields and
//! read-only parts, and each command as software sees its effect — the LPI
//! a translation or an `INT` makes pending, acknowledged through the CPU
//! interface, the configuration an `INV` or `INVALL` brings in, the
//! mappings `MAPD`, `MAPC`, `MAPTI`, `MAPI`, `MOVI` and `DISCARD` make and
//! remove, and `GITS_CREADR` stalled at a command error (R9).

use std::vec;
use std::vec::Vec;

use super::*;
use crate::board::RAM_BASE;
use crate::bus::IccRegister;
use crate::devices::gicv2::GicShape;
use crate::devices::gicv3::GicV3;
use crate::identity::{GicDistFields, GicDistIidr, GicIdentity, MsiFrameIidr};

/// The tables, in RAM from `RAM_BASE`: the LPI Configuration table, the
/// LPI Pending table, the command queue (64 KiB aligned, one 4 KiB page:
/// 128 commands), the Device and Collection tables (flat, one 4 KiB page
/// each: 512 entries) and the Interrupt Translation Tables.
const PROP: u64 = RAM_BASE + 0x1_0000;
const PEND: u64 = RAM_BASE + 0x2_0000;
const QUEUE: u64 = RAM_BASE + 0x3_0000;
const DEVICES: u64 = RAM_BASE + 0x4_0000;
const COLLECTIONS: u64 = RAM_BASE + 0x5_0000;
const ITT: u64 = RAM_BASE + 0x6_0000;
const ITT2: u64 = RAM_BASE + 0x6_1000;
/// Level 2 tables of a two-level Device table.
const LEVEL2: u64 = RAM_BASE + 0x7_0000;
const RAM_LEN: usize = 0x8_0000;

/// The registers' offsets.
const CTLR: u64 = 0x0000;
const IIDR: u64 = 0x0004;
const TYPER: u64 = 0x0008;
const CBASER: u64 = 0x0080;
const CWRITER: u64 = 0x0088;
const CREADR: u64 = 0x0090;
const BASER0: u64 = 0x0100;
const BASER1: u64 = 0x0108;
const BASER2: u64 = 0x0110;
const STATUSR: u64 = 0x0040;
const PIDR2: u64 = 0xFFE8;
const PIDR4: u64 = 0xFFD0;
const TRANSLATER: u64 = 0x1_0040;

/// A valid table of one 4 KiB page at `base`, flat.
const fn table(base: u64) -> u64 {
    (1 << 63) | base
}

/// Distributor and Redistributor offsets the bench uses.
const GICD_CTLR: u64 = 0x0000;
const GICR_WAKER: u64 = 0x0014;

/// The commands' encodings (5.3).
fn mapd(device: u32, itt: u64, size: u64, valid: bool) -> [u64; 4] {
    [
        u64::from(device) << 32 | 0x08,
        size,
        u64::from(valid) << 63 | itt,
        0,
    ]
}

fn mapc(icid: u16, target: u64, valid: bool) -> [u64; 4] {
    [
        0x09,
        0,
        u64::from(valid) << 63 | target << 16 | u64::from(icid),
        0,
    ]
}

fn mapti(device: u32, event: u32, intid: u32, icid: u16) -> [u64; 4] {
    [
        u64::from(device) << 32 | 0x0A,
        u64::from(intid) << 32 | u64::from(event),
        u64::from(icid),
        0,
    ]
}

fn mapi(device: u32, event: u32, icid: u16) -> [u64; 4] {
    [
        u64::from(device) << 32 | 0x0B,
        u64::from(event),
        u64::from(icid),
        0,
    ]
}

/// A command of a device's event alone: `INT`, `CLEAR`, `INV`, `DISCARD`.
fn event_command(opcode: u64, device: u32, event: u32) -> [u64; 4] {
    [u64::from(device) << 32 | opcode, u64::from(event), 0, 0]
}

fn movi(device: u32, event: u32, icid: u16) -> [u64; 4] {
    [
        u64::from(device) << 32 | 0x01,
        u64::from(event),
        u64::from(icid),
        0,
    ]
}

fn invall(icid: u16) -> [u64; 4] {
    [0x0D, 0, u64::from(icid), 0]
}

fn sync(target: u64) -> [u64; 4] {
    [0x05, 0, target << 16, 0]
}

fn movall(from: u64, to: u64) -> [u64; 4] {
    [0x0E, 0, from << 16, to << 16]
}

const INT: u64 = 0x03;
const CLEAR: u64 = 0x04;
const INV: u64 = 0x0C;
const DISCARD: u64 = 0x0F;

/// A GIC with its Redistributor awake, Group 1 enabled, nothing masked and
/// LPIs enabled on the bench's tables; its ITS's tables and command queue
/// given, and the ITS enabled.
struct Bench {
    gic: GicV3,
    ram: Vec<u8>,
    /// Where the next command goes, from the queue's start.
    cwriter: u64,
}

impl Bench {
    fn new() -> Self {
        let mut bench = Self::quiet();
        bench.enable_lpis();
        bench.its64(BASER0, table(DEVICES));
        bench.its64(BASER1, table(COLLECTIONS));
        bench.its64(CBASER, table(QUEUE));
        bench.its64(CTLR, 1);
        bench
    }

    /// The GIC with nothing of the ITS or the LPIs set up.
    fn quiet() -> Self {
        let mut gic = GicV3::new(GicShape::DEFAULT, GicNames::NONE);
        let mut ram = vec![0u8; RAM_LEN];
        let mut guest = GuestRam::new(&mut ram);
        gic.redist_write(&mut guest, RegOffset::new(GICR_WAKER), AccessWidth::Word, 0);
        gic.dist_write(RegOffset::new(GICD_CTLR), AccessWidth::Word, 0b10);
        gic.icc_write(IccRegister::Igrpen1, 1);
        gic.icc_write(IccRegister::Pmr, 0xFF);
        Self {
            gic,
            ram,
            cwriter: 0,
        }
    }

    fn enable_lpis(&mut self) {
        self.redist64(0x0070, PROP | 13);
        self.redist64(0x0078, PEND);
        self.redist64(0x0000, 1);
    }

    fn redist64(&mut self, off: u64, value: u64) {
        let mut guest = GuestRam::new(&mut self.ram);
        self.gic
            .redist_write(&mut guest, RegOffset::new(off), AccessWidth::Dword, value);
    }

    fn its64(&mut self, off: u64, value: u64) {
        self.its_write(off, AccessWidth::Dword, value);
    }

    fn its_write(&mut self, off: u64, width: AccessWidth, value: u64) {
        let mut guest = GuestRam::new(&mut self.ram);
        self.gic
            .its_write(&mut guest, RegOffset::new(off), width, value);
    }

    fn its(&self, off: u64) -> u64 {
        self.gic.its_read(RegOffset::new(off), AccessWidth::Dword)
    }

    fn its32(&self, off: u64) -> u32 {
        self.gic.its_read(RegOffset::new(off), AccessWidth::Word) as u32
    }

    /// Put `command` in the queue at the write pointer and move the pointer
    /// past it.
    fn queue(&mut self, command: [u64; 4]) {
        let at = (QUEUE - RAM_BASE + self.cwriter) as usize;
        for (i, dw) in command.iter().enumerate() {
            self.ram[at + 8 * i..at + 8 * i + 8].copy_from_slice(&dw.to_le_bytes());
        }
        self.cwriter = (self.cwriter + 32) % 0x1000;
    }

    /// Queue `commands` and write `GITS_CWRITER`.
    fn run(&mut self, commands: &[[u64; 4]]) {
        for &command in commands {
            self.queue(command);
        }
        self.its64(CWRITER, self.cwriter);
    }

    /// LPI `intid`'s LPI Configuration table entry.
    fn config(&mut self, intid: u32, entry: u8) {
        self.ram[(PROP - RAM_BASE) as usize + (intid as usize - 8192)] = entry;
    }

    /// A write of `event` to `GITS_TRANSLATER`, from the processor.
    fn translate(&mut self, event: u32) {
        self.its_write(TRANSLATER, AccessWidth::Word, u64::from(event));
    }

    fn hppir(&mut self) -> u64 {
        self.gic.icc_read(IccRegister::Hppir1)
    }

    fn iar(&mut self) -> u64 {
        self.gic.icc_read(IccRegister::Iar1)
    }

    fn eoi(&mut self, intid: u64) {
        self.gic.icc_write(IccRegister::Eoir1, intid);
    }

    /// Whether the queue ran to the write pointer, unstalled.
    fn drained(&self) -> bool {
        self.its(CREADR) == self.cwriter
    }

    /// Device 0's events 0 to 31 (`MAPD` `Size` 4) with an ITT at `ITT`,
    /// collection 0 on processor 0, and event `event` translated to LPI
    /// `intid`, enabled at priority 0xA0.
    fn map(&mut self, event: u32, intid: u32) {
        self.config(intid, 0xA0 | 0b11);
        self.run(&[
            mapd(0, ITT, 4, true),
            mapc(0, 0, true),
            mapti(0, event, intid, 0),
            sync(0),
        ]);
        // The LPI's configuration as the Redistributor caches it: read when
        // LPIs were enabled, before the entry was written.
        self.run(&[event_command(INV, 0, event)]);
        assert!(self.drained(), "the mapping commands complete");
    }
}

/// The reset values: disabled and quiescent; `GITS_TYPER` of physical LPIs,
/// 8-byte entries, sixteen bits of EventID and of DeviceID, collections in
/// memory and targets as processor numbers; `GITS_BASER0` the Device
/// table's and `GITS_BASER1` the Collection table's, 8-byte entries, the
/// rest unimplemented; `GITS_PIDR2.ArchRev` 3 and the frames' size in
/// `GITS_PIDR4`.
#[test]
fn registers_reset_to_their_values() {
    let bench = Bench::quiet();
    assert_eq!(bench.its32(CTLR), 1 << 31, "Quiescent");
    assert_eq!(bench.its(TYPER), 0x1_EF71);
    assert_eq!(bench.its(BASER0), 0x0107_0000_0000_0000);
    assert_eq!(bench.its(BASER1), 0x0407_0000_0000_0000);
    for n in 2..8 {
        assert_eq!(bench.its(BASER0 + 8 * n), 0, "GITS_BASER{n}");
    }
    assert_eq!(bench.its(CBASER), 0);
    assert_eq!(bench.its(CREADR), 0);
    assert_eq!(bench.its32(PIDR2) >> 4, 3, "ArchRev: GICv3");
    assert_eq!(bench.its32(PIDR4) >> 4, 5, "128 KiB");
    assert_eq!(bench.its32(IIDR), 0);
}

/// `GITS_IIDR` reads the GIC's identification, as `GICD_IIDR` does.
#[test]
fn gits_iidr_is_the_gics() {
    let iidr = GicDistIidr::new(GicDistFields {
        product: 0x42,
        variant: 1,
        revision: 2,
        jep106_continuation: 0x9,
        jep106_identity: 0x2B,
    })
    .expect("fields that fit");
    let names = GicNames {
        gic: GicIdentity {
            distributor: iidr,
            cpu_interface: GicIdentity::NONE.cpu_interface,
        },
        msi_frame: MsiFrameIidr::NONE,
    };
    let gic = GicV3::new(GicShape::DEFAULT, names);
    assert_eq!(
        gic.its_read(RegOffset::new(IIDR), AccessWidth::Word),
        u64::from(iidr.get())
    );
    assert_eq!(
        gic.its_read(RegOffset::new(0xFFE0), AccessWidth::Word),
        0x42,
        "GITS_PIDR0: the product"
    );
}

/// The registers keep their fields: `GITS_BASER<n>`'s `Type` and
/// `Entry_Size` are read-only and the unimplemented ones RAZ/WI,
/// `GITS_CBASER` drops its RES0 bits, `GITS_TYPER`, `GITS_CREADR` and
/// `GITS_STATUSR` take no write, `Retry` reads 0; while the ITS is enabled
/// the table and queue registers keep their values.
#[test]
fn registers_keep_their_fields() {
    let mut bench = Bench::quiet();
    bench.its64(BASER0, u64::MAX);
    assert_eq!(
        bench.its(BASER0),
        0xF8E0_FFFF_FFFF_FFFF | 0x0107_0000_0000_0000
    );
    bench.its64(BASER2, u64::MAX);
    assert_eq!(bench.its(BASER2), 0);
    bench.its64(CBASER, u64::MAX);
    assert_eq!(bench.its(CBASER), 0xB8EF_FFFF_FFFF_FCFF);
    bench.its64(TYPER, 0);
    assert_eq!(bench.its(TYPER), 0x1_EF71);
    bench.its64(CREADR, 0x40);
    assert_eq!(bench.its(CREADR), 0);
    bench.its64(STATUSR, u64::MAX);
    assert_eq!(bench.its32(STATUSR), 0, "no GITS_STATUSR");
    bench.its64(CWRITER, 0x41);
    assert_eq!(bench.its(CWRITER), 0x40, "Retry reads 0");
    bench.its64(BASER0, table(DEVICES));
    bench.its64(CBASER, table(QUEUE));
    bench.its64(CTLR, u64::MAX);
    assert_eq!(bench.its32(CTLR), 1, "Enabled; RES0 bits and Quiescent 0");
    bench.its64(BASER0, 0);
    bench.its64(CBASER, 0);
    assert_eq!(bench.its(BASER0) & (1 << 63), 1 << 63, "kept while enabled");
    assert_eq!(bench.its(CBASER), table(QUEUE));
    bench.its64(CTLR, 0);
    assert_eq!(bench.its32(CTLR), 1 << 31, "quiescent at once");
}

/// `MAPD`, `MAPC` and `MAPTI` map device 0's event to an LPI: a write of
/// the event to `GITS_TRANSLATER` makes it pending and the CPU interface
/// signals and acknowledges it; an event out of the device's range, one
/// unmapped, a 16-bit write and an EventID wider than the ITS's are as the
/// specification says.
#[test]
fn mapped_events_translate_to_lpis() {
    let mut bench = Bench::new();
    bench.translate(3);
    assert_eq!(bench.hppir(), 1023, "nothing mapped yet");
    bench.map(3, 8200);
    assert_eq!(bench.hppir(), 1023);
    bench.translate(3);
    assert!(bench.gic.irq_out());
    assert_eq!(bench.iar(), 8200);
    assert_eq!(bench.gic.icc_read(IccRegister::Rpr), 0xA0);
    bench.eoi(8200);
    assert_eq!(bench.gic.icc_read(IccRegister::Rpr), 0xFF);
    for event in [4, 32, 1 << 16 | 3] {
        bench.translate(event);
        assert_eq!(bench.hppir(), 1023, "event {event:#x}");
    }
    bench.its_write(TRANSLATER, AccessWidth::Half, 0xFFFF_0003);
    assert_eq!(bench.iar(), 8200, "a 16-bit write's EventID");
}

/// `MAPI` maps an event to the LPI of its own number.
#[test]
fn mapi_maps_an_event_to_its_own_intid() {
    let mut bench = Bench::new();
    bench.config(8195, 0x80 | 0b11);
    bench.run(&[
        mapd(0, ITT, 13, true),
        mapc(0, 0, true),
        mapi(0, 8195, 0),
        invall(0),
    ]);
    assert!(bench.drained());
    bench.translate(8195);
    assert_eq!(bench.iar(), 8195);
}

/// `INT` makes the event's LPI pending, `CLEAR` removes the pending state,
/// and `DISCARD` removes it and the translation: a later write of the
/// event does nothing and an `INT` of it is a command error.
#[test]
fn int_clear_and_discard() {
    let mut bench = Bench::new();
    bench.map(1, 8300);
    bench.run(&[event_command(INT, 0, 1)]);
    assert_eq!(bench.hppir(), 8300);
    bench.run(&[event_command(CLEAR, 0, 1)]);
    assert_eq!(bench.hppir(), 1023);
    bench.run(&[event_command(INT, 0, 1), event_command(DISCARD, 0, 1)]);
    assert!(bench.drained());
    assert_eq!(bench.hppir(), 1023, "DISCARD cleared it");
    bench.translate(1);
    assert_eq!(bench.hppir(), 1023, "and unmapped it");
    bench.run(&[event_command(INT, 0, 1)]);
    assert_eq!(bench.its(CREADR) & 1, 1, "INT_UNMAPPED_INTERRUPT stalls");
}

/// The Redistributor caches the LPI Configuration table: a change in
/// memory is not seen until `INV` of an event mapped to the LPI, or
/// `INVALL` of the collection.
#[test]
fn inv_and_invall_bring_in_a_configuration_change() {
    let mut bench = Bench::new();
    bench.map(2, 8400);
    bench.map(5, 8401);
    bench.config(8400, 0xA0 | 0b10);
    bench.run(&[event_command(INT, 0, 2)]);
    assert_eq!(bench.iar(), 8400, "still enabled in the cache");
    bench.eoi(8400);
    bench.run(&[event_command(INV, 0, 2), event_command(INT, 0, 2)]);
    assert_eq!(bench.hppir(), 1023, "disabled once invalidated");
    bench.config(8400, 0x40 | 0b11);
    bench.config(8401, 0x30 | 0b11);
    bench.run(&[event_command(INT, 0, 5)]);
    assert_eq!(bench.hppir(), 8401, "8400 still disabled in the cache");
    bench.run(&[invall(0)]);
    assert_eq!(bench.hppir(), 8401, "0x30 before 0x40");
    assert_eq!(bench.iar(), 8401);
    assert_eq!(bench.gic.icc_read(IccRegister::Rpr), 0x30);
    bench.eoi(8401);
    assert_eq!(bench.iar(), 8400, "enabled by INVALL, pending all along");
}

/// `MOVI` moves an event to another collection: unmapping the old one then
/// changes nothing of it, unmapping the new one makes an `INT` of it a
/// command error. Moving it to a collection that targets another processor,
/// which has no Redistributor, loses its pending state.
#[test]
fn movi_moves_an_event_to_another_collection() {
    let mut bench = Bench::new();
    bench.map(0, 8500);
    bench.run(&[mapc(1, 0, true), movi(0, 0, 1), mapc(0, 0, false)]);
    bench.run(&[event_command(INT, 0, 0)]);
    assert!(bench.drained(), "in collection 1 now");
    assert_eq!(bench.iar(), 8500);
    bench.eoi(8500);
    bench.run(&[mapc(2, 1, true), event_command(INT, 0, 0)]);
    assert_eq!(bench.hppir(), 8500);
    bench.run(&[movi(0, 0, 2)]);
    assert_eq!(bench.hppir(), 1023, "moved to processor 1, lost");
    bench.run(&[movi(0, 0, 1), mapc(1, 0, false), event_command(INT, 0, 0)]);
    let creadr = bench.its(CREADR);
    assert_eq!(creadr & 1, 1, "INT of an unmapped collection stalls");
    assert_eq!(creadr & !1, (bench.cwriter + 0x1000 - 32) % 0x1000);
}

/// `MOVI` of an event with no translation, or to an unmapped collection,
/// completes and changes nothing.
#[test]
fn movi_of_nothing_completes() {
    let mut bench = Bench::new();
    bench.map(0, 8600);
    bench.run(&[movi(0, 7, 0), movi(0, 0, 9)]);
    assert!(bench.drained());
    bench.translate(0);
    assert_eq!(bench.iar(), 8600, "still in collection 0");
}

/// `MOVALL` from processor 0 to itself keeps every pending LPI, and to
/// another processor, which has no Redistributor, loses them; `SYNC` of any
/// target completes.
#[test]
fn movall_and_sync() {
    let mut bench = Bench::new();
    bench.map(0, 8700);
    bench.run(&[event_command(INT, 0, 0), movall(0, 0), sync(5)]);
    assert!(bench.drained());
    assert_eq!(bench.hppir(), 8700);
    bench.run(&[movall(0, 1)]);
    assert_eq!(bench.hppir(), 1023);
}

/// A command error stalls the queue at the command, `GITS_CREADR.Stalled`
/// set and no later command run, until `GITS_CWRITER.Retry` runs it again;
/// an opcode GICv3 has no command for is an error too.
#[test]
fn a_command_error_stalls_the_queue_until_retried() {
    let mut bench = Bench::new();
    bench.config(8800, 0x60 | 0b11);
    bench.run(&[
        mapd(0, ITT, 4, true),
        mapc(0, 0, true),
        mapti(0, 0, 100, 0),
        mapti(0, 1, 8800, 0),
    ]);
    let stalled_at = 2 * 32;
    assert_eq!(bench.its(CREADR), stalled_at | 1, "MAPTI_PHYSICALID_OOR");
    bench.translate(1);
    assert_eq!(bench.hppir(), 1023, "the next MAPTI did not run");
    bench.its64(CWRITER, bench.cwriter);
    assert_eq!(bench.its(CREADR), stalled_at | 1, "still stalled");
    let fixed = mapti(0, 0, 8800, 0);
    let at = (QUEUE - RAM_BASE + stalled_at) as usize;
    bench.ram[at + 8..at + 16].copy_from_slice(&fixed[1].to_le_bytes());
    bench.its64(CWRITER, bench.cwriter | 1);
    assert!(bench.drained(), "retried and run on");
    bench.run(&[invall(0)]);
    bench.translate(1);
    assert_eq!(bench.iar(), 8800);
    bench.run(&[[0x2B, 0, 0, 0]]);
    assert_eq!(bench.its(CREADR) & 1, 1, "VMAPI: no GICv4");
}

/// The commands' other errors: a device past the Device table, a DeviceID
/// wider than the ITS's, a `MAPD` `Size` wider than its EventIDs, a
/// collection past the Collection table, an unmapped device.
#[test]
fn out_of_range_arguments_are_command_errors() {
    for command in [
        mapd(512, ITT, 4, true),
        mapd(1 << 16, ITT, 4, true),
        mapd(0, ITT, 16, true),
        mapc(512, 0, true),
        mapti(1, 0, 8192, 0),
        invall(3),
    ] {
        let mut bench = Bench::new();
        bench.run(&[command]);
        assert_eq!(bench.its(CREADR), 1, "{command:x?}");
    }
}

/// A two-level Device table: a device whose level 1 entry is valid maps in
/// the level 2 page it names; one whose level 1 entry is not has its `MAPD`
/// discarded, which completes, and its events are not translated.
#[test]
fn a_two_level_device_table() {
    let mut bench = Bench::quiet();
    bench.enable_lpis();
    bench.its64(BASER0, table(DEVICES) | 1 << 62);
    bench.its64(BASER1, table(COLLECTIONS));
    bench.its64(CBASER, table(QUEUE));
    bench.its64(CTLR, 1);
    assert_eq!(bench.its(BASER0) >> 62, 0b11, "Valid, Indirect");
    // Level 1 entry 1 (devices 512 to 1023) valid, entry 0 not.
    let l1 = (DEVICES - RAM_BASE) as usize + 8;
    bench.ram[l1..l1 + 8].copy_from_slice(&((1u64 << 63) | LEVEL2).to_le_bytes());
    bench.config(8900, 0xA3);
    bench.config(8901, 0xA3);
    bench.run(&[
        mapd(512 + 7, ITT, 4, true),
        mapd(7, ITT2, 4, true),
        mapc(0, 0, true),
        mapti(512 + 7, 0, 8900, 0),
        invall(0),
    ]);
    assert!(bench.drained());
    bench.run(&[event_command(INT, 512 + 7, 0)]);
    assert_eq!(bench.iar(), 8900);
    bench.eoi(8900);
    bench.run(&[mapti(7, 0, 8901, 0)]);
    assert_eq!(
        bench.its(CREADR) & 1,
        1,
        "device 7's MAPD was discarded: MAPTI_UNMAPPED_DEVICE"
    );
}

/// With the ITS disabled, translations are ignored and commands wait until
/// it is enabled; with the Redistributor's LPIs disabled, commands
/// complete and their LPIs are lost.
#[test]
fn a_disabled_its_or_redistributor() {
    let mut bench = Bench::new();
    bench.map(0, 9000);
    bench.its64(CTLR, 0);
    bench.translate(0);
    assert_eq!(bench.hppir(), 1023);
    bench.run(&[event_command(INT, 0, 0)]);
    assert_eq!(bench.hppir(), 1023, "the ITS is disabled");
    assert!(!bench.drained());
    bench.its64(CTLR, 1);
    assert!(bench.drained());
    assert_eq!(bench.iar(), 9000, "run as the ITS was enabled");
    bench.eoi(9000);
    bench.redist64(0x0000, 0);
    bench.run(&[event_command(INT, 0, 0)]);
    assert!(bench.drained(), "completes");
    bench.translate(0);
    bench.redist64(0x0000, 1);
    assert_eq!(bench.hppir(), 1023, "lost while LPIs were disabled");
}

/// A `GITS_CWRITER` beyond the queue leaves the queue invalid until one
/// within it.
#[test]
fn a_write_pointer_beyond_the_queue() {
    let mut bench = Bench::new();
    bench.map(0, 9100);
    bench.queue(event_command(INT, 0, 0));
    bench.its64(CWRITER, 0x2000);
    assert_eq!(bench.hppir(), 1023);
    bench.its64(CWRITER, bench.cwriter);
    assert_eq!(bench.hppir(), 9100);
}

/// A snapshot holds the ITS's registers: restored, a stalled queue is still
/// stalled at its command, and a retry runs it.
#[test]
fn snapshots_hold_the_its() {
    let mut bench = Bench::new();
    bench.map(0, 9200);
    bench.run(&[[0xFF, 0, 0, 0], event_command(INT, 0, 0)]);
    let stalled = bench.its(CREADR);
    assert_eq!(stalled & 1, 1);
    let mut image = std::vec::Vec::new();
    bench.gic.save(&mut image).expect("save");
    let mut restored = Bench::quiet();
    restored.ram = bench.ram.clone();
    restored
        .gic
        .restore(&mut image.as_slice())
        .expect("restore");
    restored.cwriter = bench.cwriter;
    assert_eq!(restored.its(CREADR), stalled);
    assert_eq!(restored.its(BASER0), bench.its(BASER0));
    let at = (QUEUE - RAM_BASE + (stalled & !1)) as usize;
    restored.ram[at..at + 8].copy_from_slice(&0x05u64.to_le_bytes());
    restored.its64(CWRITER, restored.cwriter | 1);
    assert!(restored.drained());
    assert_eq!(restored.iar(), 9200);
    // A read pointer outside the queue is refused.
    let mut bad = image.clone();
    let creadr = image.len() - 8 * 2 - 8;
    bad[creadr..creadr + 8].copy_from_slice(&0x1000u64.to_le_bytes());
    assert!(Bench::quiet().gic.restore(&mut bad.as_slice()).is_err());
}
