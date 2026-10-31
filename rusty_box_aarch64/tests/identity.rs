//! What a guest reads of who made its machine (R9: each test asserts what
//! the guest sees): the device tree's `/model` and root `compatible`, the
//! virtio-mmio `VendorID`, the GICv2m frame's `MSI_IIDR`, and the input
//! devices' names, vendor, products and versions are the board's
//! identity's, the default [`BoardIdentity::RUSTY_BOX`] or one a host gives;
//! nothing names QEMU, and the tree has no `fw-cfg` node.
//! `tests/device_regions.rs` checks that `fw_cfg`'s window is unassigned.
//!
//! Guest code is hand-assembled (GNU as for aarch64); each snippet's source
//! is beside its words.

use rusty_box_aarch64::board::{GIC_V2M_BASE, RAM_BASE, VIRTIO_MMIO_BASE};
use rusty_box_aarch64::boot::fdt;
use rusty_box_aarch64::{
    BoardIdentity, EdidDate, EdidWeek, EdidWeekNumber, EdidYear, IdentityError, IdentityField,
    IdentityNames, IdentityProblem, InputDeviceId, InputProductId, InputVendorId, InputVersion,
    LentStorage, MachineBuilder, MemoryLoan, MsiFrameField, MsiFrameFields, MsiFrameIidr, NoEl2,
    NotAnEdidWeek, NotAnEdidYear, NotMsiFrameFields, NotPnpId, PnpId, RamSize, Reg, RunBudget,
    StopReason, VirtConfig, VirtMachine, VirtioVendorId,
};

const RAM_LEN: usize = 4 << 20;

/// `ldr w2, [x1]; b .`
const LOAD_WORD: [u32; 2] = [0xB940_0022, 0x1400_0000];

/// `MSI_IIDR`'s offset in the GICv2m frame.
const MSI_IIDR: u64 = 0xFCC;
/// `VendorID`'s offset in a virtio-mmio transport.
const VENDOR_ID: u64 = 0x00C;
/// The highest transport, which the first device plugged in takes.
const LAST_TRANSPORT: u64 = VIRTIO_MMIO_BASE + 31 * 0x200;

fn acme() -> BoardIdentity {
    BoardIdentity::new(IdentityNames {
        model: "Acme Board",
        compatible: &["acme,board-1", "acme,board"],
        virtio_vendor: VirtioVendorId::new(0x1234_5678),
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"))
}

/// A bare machine named `identity`, with the virtio devices `devices`.
fn bare(
    ram: &mut [u8],
    identity: BoardIdentity,
    devices: Devices,
) -> VirtMachine<LentStorage<'_>, NoEl2> {
    let builder = MachineBuilder::new(VirtConfig::DEFAULT)
        .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
        .identity(identity);
    let builder = match devices {
        Devices::None => builder,
        Devices::Keyboard => builder.virtio_keyboard(),
        Devices::KeyboardAndTablet => builder.virtio_keyboard().virtio_tablet(),
    };
    builder
        .bare()
        .build_in(MemoryLoan { ram, vars: &mut [] })
        .expect("a bare machine builds")
}

/// What is plugged into a test's board.
#[derive(Debug, Clone, Copy)]
enum Devices {
    /// Nothing: every transport empty, of the legacy layout.
    None,
    /// A virtio keyboard in the last transport, every transport of the
    /// non-legacy layout.
    Keyboard,
    /// A virtio keyboard in the last transport and a tablet in the one
    /// before it.
    KeyboardAndTablet,
}

/// `strb w2, [x1, #0x100]; ldrb w3, [x1, #0x102]; ldr x4, [x1, #0x108];
/// ldr x5, [x1, #0x110]; ldr x6, [x1, #0x118]; ldr x7, [x1, #0x120]; b .`:
/// select a virtio input configuration entry, then read its size and its
/// first 32 bytes.
const READ_INPUT_ENTRY: [u32; 7] = [
    0x3904_0022,
    0x3944_0823,
    0xF940_8424,
    0xF940_8825,
    0xF940_8C26,
    0xF940_9027,
    0x1400_0000,
];
/// `VIRTIO_INPUT_CFG_ID_NAME` and `VIRTIO_INPUT_CFG_ID_DEVIDS`.
const CFG_ID_NAME: u64 = 0x01;
const CFG_ID_DEVIDS: u64 = 0x03;

/// The entry `select` of the virtio input device in `transport`, as the
/// guest reads it: `size` bytes of its payload.
fn input_entry(
    m: &mut VirtMachine<LentStorage<'_>, NoEl2>,
    transport: u64,
    select: u64,
) -> Vec<u8> {
    for (i, word) in READ_INPUT_ENTRY.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    m.reg_write(Reg::Pc, RAM_BASE);
    m.reg_write(Reg::X1, transport);
    m.reg_write(Reg::X2, select);
    let out = m.step(RunBudget::Instructions(6));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    let size = m.reg_read(Reg::X3) as usize;
    let payload: Vec<u8> = [Reg::X4, Reg::X5, Reg::X6, Reg::X7]
        .into_iter()
        .flat_map(|r| m.reg_read(r).to_le_bytes())
        .collect();
    payload[..size.min(payload.len())].to_vec()
}

/// The keyboard and the tablet the board plugs in name themselves and
/// their vendor as its identity says, each its own name.
#[test]
fn the_input_devices_name_themselves_by_the_identity() {
    let id = |product, version| InputDeviceId {
        product: InputProductId::new(product),
        version: InputVersion::new(version),
    };
    let host = BoardIdentity::new(IdentityNames {
        keyboard: "Acme Keys",
        keyboard_id: id(0x0A01, 0x0B02),
        tablet: "Acme Pad",
        tablet_id: id(0x0C03, 0x0D04),
        input_vendor: InputVendorId::new(0xACE5),
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"));
    // Each device's `VIRTIO_INPUT_CFG_ID_DEVIDS`: BUS_VIRTUAL (6), the
    // vendor, the product and the version, little-endian.
    let cases = [
        (
            BoardIdentity::RUSTY_BOX,
            &b"Rusty Box Virtio Keyboard"[..],
            [6, 0, 0x42, 0x52, 1, 0, 1, 0],
            &b"Rusty Box Virtio Tablet"[..],
            [6, 0, 0x42, 0x52, 2, 0, 1, 0],
        ),
        (
            host,
            &b"Acme Keys"[..],
            [6, 0, 0xE5, 0xAC, 0x01, 0x0A, 0x02, 0x0B],
            &b"Acme Pad"[..],
            [6, 0, 0xE5, 0xAC, 0x03, 0x0C, 0x04, 0x0D],
        ),
    ];
    for (identity, keyboard, keyboard_ids, tablet, tablet_ids) in cases {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = bare(&mut ram, identity, Devices::KeyboardAndTablet);
        let tablet_transport = LAST_TRANSPORT - 0x200;
        assert_eq!(input_entry(&mut m, LAST_TRANSPORT, CFG_ID_NAME), keyboard);
        assert_eq!(
            input_entry(&mut m, LAST_TRANSPORT, CFG_ID_DEVIDS),
            keyboard_ids
        );
        assert_eq!(input_entry(&mut m, tablet_transport, CFG_ID_NAME), tablet);
        assert_eq!(
            input_entry(&mut m, tablet_transport, CFG_ID_DEVIDS),
            tablet_ids
        );
    }
}

/// What the guest's `ldr w2, [x1]` reads at `addr` on `m`.
fn guest_load(m: &mut VirtMachine<LentStorage<'_>, NoEl2>, addr: u64) -> u64 {
    for (i, word) in LOAD_WORD.iter().enumerate() {
        m.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())
            .expect("code is in RAM");
    }
    m.reg_write(Reg::Pc, RAM_BASE);
    m.reg_write(Reg::X1, addr);
    m.reg_write(Reg::X2, 0x5555);
    let out = m.step(RunBudget::Instructions(1));
    assert_eq!(out.stop, StopReason::BudgetExhausted);
    m.reg_read(Reg::X2)
}

/// Every transport's `VendorID` is the identity's, empty (legacy layout)
/// or with a device behind it (non-legacy).
#[test]
fn a_virtio_transport_reads_the_identitys_vendor() {
    let rbox = VirtioVendorId::from_ascii(*b"RBOX");
    let acme_vendor = VirtioVendorId::new(0x1234_5678);
    for (identity, vendor) in [(BoardIdentity::RUSTY_BOX, rbox), (acme(), acme_vendor)] {
        for devices in [Devices::None, Devices::Keyboard] {
            let mut ram = vec![0u8; RAM_LEN];
            let mut m = bare(&mut ram, identity, devices);
            let register = u64::from(vendor.get());
            assert_eq!(
                guest_load(&mut m, VIRTIO_MMIO_BASE + VENDOR_ID),
                register,
                "an empty transport, {devices:?}"
            );
            assert_eq!(
                guest_load(&mut m, LAST_TRANSPORT + VENDOR_ID),
                register,
                "the last transport, {devices:?}"
            );
        }
    }
}

/// The GICv2m frame's `MSI_IIDR` is the identity's (BSA 1.0C, I.9.3): by
/// default 0, naming no product and no implementer, where QEMU's reads its
/// product ID, 'Q' (`PRODUCT_ID_QEMU`); a host's product, revision and
/// JEP106 code where it gives them, the architecture version 0 between.
#[test]
fn the_msi_frame_reads_the_identitys_iidr() {
    let fields = MsiFrameFields {
        product: 0x123,
        revision: 0x5,
        jep106_continuation: 0x6,
        jep106_identity: 0x78,
    };
    let iidr = MsiFrameIidr::new(fields).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(iidr.fields(), fields);
    let hosts = BoardIdentity::new(IdentityNames {
        msi_frame_iidr: iidr,
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"));
    for (identity, iidr) in [(BoardIdentity::RUSTY_BOX, 0), (hosts, 0x1230_5678)] {
        let mut ram = vec![0u8; RAM_LEN];
        let mut m = bare(&mut ram, identity, Devices::None);
        assert_eq!(guest_load(&mut m, GIC_V2M_BASE + MSI_IIDR), iidr);
    }
}

/// The tree the firmware is handed names the board by its identity, and
/// holds no string of QEMU's: no `qemu,` compatible, no `fw-cfg` node and
/// not QEMU's `linux,dummy-virt`.
#[test]
fn the_device_tree_a_firmware_reads_names_the_board() {
    /// `b .`: firmware that waits.
    const FIRMWARE: [u8; 4] = 0x1400_0000u32.to_le_bytes();
    for (identity, model, compatible) in [
        (
            BoardIdentity::RUSTY_BOX,
            &b"Rusty Box virt\0"[..],
            &b"rusty-box,virt\0"[..],
        ),
        (
            acme(),
            &b"Acme Board\0"[..],
            &b"acme,board-1\0acme,board\0"[..],
        ),
    ] {
        let mut ram = vec![0u8; RAM_LEN];
        let m = MachineBuilder::new(VirtConfig::DEFAULT)
            .ram(RamSize::bytes(RAM_LEN as u64).expect("a valid size"))
            .identity(identity)
            .firmware(&FIRMWARE)
            .build_in(MemoryLoan {
                ram: &mut ram,
                vars: &mut [],
            })
            .expect("a firmware machine builds");
        let tree = m.device_tree();
        assert_eq!(fdt::property(tree, "", "model"), Ok(Some(model)));
        assert_eq!(fdt::property(tree, "", "compatible"), Ok(Some(compatible)));
        let lower = tree.to_ascii_lowercase();
        for qemus in [&b"qemu"[..], b"fw-cfg", b"dummy-virt"] {
            assert!(
                !lower.windows(qemus.len()).any(|w| w == qemus),
                "{}",
                String::from_utf8_lossy(qemus)
            );
        }
    }
}

/// The identity is part of a machine's shape, as its RAM and devices are:
/// a snapshot restores into a machine of the identity it was taken under,
/// and is refused by one whose identity differs in any name or number, the
/// display's date among them: a later year, or the same year as a model
/// year.
#[test]
fn a_snapshot_restores_only_under_its_identity() {
    let mut ram = vec![0u8; RAM_LEN];
    let m = bare(&mut ram, BoardIdentity::RUSTY_BOX, Devices::None);
    let mut image = Vec::new();
    m.save_snapshot(&mut image).expect("a snapshot saves");
    let year = EdidYear::new(2026).unwrap_or_else(|e| panic!("{e}"));
    let differing = |date| {
        BoardIdentity::new(IdentityNames {
            display_date: date,
            ..IdentityNames::RUSTY_BOX
        })
        .unwrap_or_else(|e| panic!("{e}"))
    };
    let later = differing(EdidDate::Manufactured {
        week: EdidWeek::Unstated,
        year: EdidYear::new(2027).unwrap_or_else(|e| panic!("{e}")),
    });
    let model_year = differing(EdidDate::ModelYear(year));
    for (identity, restores) in [
        (BoardIdentity::RUSTY_BOX, true),
        (acme(), false),
        (later, false),
        (model_year, false),
    ] {
        let mut ram = vec![0u8; RAM_LEN];
        let mut other = bare(&mut ram, identity, Devices::None);
        assert_eq!(
            other.restore_snapshot(&mut image.as_slice()).is_ok(),
            restores,
            "{identity:?}"
        );
    }
}

/// Why [`BoardIdentity::new`] refuses the default names once `change` has
/// changed them, if it does.
fn refused<'a>(change: impl FnOnce(&mut IdentityNames<'a>)) -> Option<IdentityError> {
    let mut names = IdentityNames::RUSTY_BOX;
    change(&mut names);
    BoardIdentity::new(names).err()
}

/// A name the board cannot give is refused, naming the field and why.
#[test]
fn an_identity_the_board_cannot_give_is_refused() {
    use IdentityField as F;
    use IdentityProblem as P;
    let error = |field, problem| Some(IdentityError { field, problem });
    assert_eq!(refused(|n| n.model = ""), error(F::Model, P::Empty));
    assert_eq!(
        refused(|n| n.model = "tab\there"),
        error(F::Model, P::NotPrintableAscii)
    );
    let not_of_the_pattern: [&[&str]; 4] = [
        &["rusty box,virt"],
        &[",virt"],
        &["a"],
        &["acme,board-1", "acme board"],
    ];
    for compatible in not_of_the_pattern {
        assert_eq!(
            refused(|n| n.compatible = compatible),
            error(F::Compatible, P::NotCompatibleString),
            "{compatible:?}"
        );
    }
    assert_eq!(
        refused(|n| n.compatible = &[]),
        error(F::Compatible, P::Empty)
    );
    assert_eq!(
        refused(|n| n.compatible = &["acme,board", ""]),
        error(F::Compatible, P::Empty)
    );
    // Eight strings of 15 characters and their NULs fill the 128 bytes; a
    // ninth does not fit.
    let fifteen = ["acme,board-0001"; 9];
    assert_eq!(refused(|n| n.compatible = &fifteen[..8]), None);
    assert_eq!(
        refused(|n| n.compatible = &fifteen),
        error(F::Compatible, P::TooLong { max: 128 })
    );
    assert_eq!(
        refused(|n| n.display_name = "Fourteen chars"),
        error(F::DisplayName, P::TooLong { max: 13 })
    );
    let long = "k".repeat(129);
    assert_eq!(
        refused(|n| n.keyboard = &long),
        error(F::Keyboard, P::TooLong { max: 128 })
    );
    assert_eq!(refused(|n| n.tablet = ""), error(F::Tablet, P::Empty));
    assert_eq!(
        refused(|n| n.compatible = &["acme,board+2.0_x/y-z", "acme,board"]),
        None
    );
}

/// An `MSI_IIDR` is made only of fields within their bits (BSA 1.0C,
/// I.9.3), and a field wider than its bits is refused, named.
#[test]
fn an_msi_iidr_field_wider_than_its_bits_is_refused() {
    let widest = MsiFrameFields {
        product: 0xFFF,
        revision: 0xF,
        jep106_continuation: 0xF,
        jep106_identity: 0x7F,
    };
    // Every field at its widest: the architecture version, bits [19:16],
    // and bit 7 stay 0.
    assert_eq!(
        MsiFrameIidr::new(widest).map(MsiFrameIidr::get),
        Ok(0xFFF0_FF7F)
    );
    for (wide, field) in [
        (
            MsiFrameFields {
                product: 0x1000,
                ..widest
            },
            MsiFrameField::Product,
        ),
        (
            MsiFrameFields {
                revision: 0x10,
                ..widest
            },
            MsiFrameField::Revision,
        ),
        (
            MsiFrameFields {
                jep106_continuation: 0x10,
                ..widest
            },
            MsiFrameField::Jep106Continuation,
        ),
        (
            MsiFrameFields {
                jep106_identity: 0x80,
                ..widest
            },
            MsiFrameField::Jep106Identity,
        ),
    ] {
        assert_eq!(
            MsiFrameIidr::new(wide),
            Err(NotMsiFrameFields { field }),
            "{field:?}"
        );
    }
}

/// The EDID's maker, week and year are values EDID can give, or are not
/// made at all.
#[test]
fn an_edid_value_edid_cannot_give_is_not_made() {
    for id in ["RBx", "RB", "RBXX", "R1X"] {
        assert_eq!(PnpId::new(id), Err(NotPnpId), "{id}");
    }
    assert_eq!(PnpId::new("ACM").map(PnpId::letters), Ok(*b"ACM"));
    // EDID 1.4 reserves the year bytes 00h to 0Fh, 1990 to 2005.
    for year in [0, 1989, 1990, 2005, 2246, u16::MAX] {
        assert_eq!(EdidYear::new(year), Err(NotAnEdidYear { year }), "{year}");
    }
    for year in [2006, 2026, 2245] {
        assert_eq!(EdidYear::new(year).map(EdidYear::get), Ok(year), "{year}");
    }
    assert_eq!(EdidYear::FIRST.get(), 2006);
    assert_eq!(EdidYear::LAST.get(), 2245);
    for week in [0, 55, u8::MAX] {
        assert_eq!(
            EdidWeekNumber::new(week),
            Err(NotAnEdidWeek { week }),
            "{week}"
        );
    }
    for week in [1, 42, 54] {
        let number = EdidWeekNumber::new(week).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(EdidWeek::Of(number).byte(), week);
    }
    assert_eq!(EdidWeek::Unstated.byte(), 0);
}
