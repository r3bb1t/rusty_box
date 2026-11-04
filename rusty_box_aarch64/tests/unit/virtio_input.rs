//! Unit tests of `src/devices/virtio/input.rs`: QEMU's virtio keyboard and
//! tablet, as a driver reads their configuration and receives their events
//! (`hw/input/virtio-input.c`, `hw/input/virtio-input-hid.c`).

use std::vec::Vec;

use rusty_box_core::input::pointer::{PointerButton, PointerPos, WheelSteps};
use rusty_box_core::input::{Key, KeyAction};

use super::{Input, InputKind};
use crate::bus::AccessWidth;
use crate::devices::virtio::test_driver::*;
use crate::devices::virtio::{Device, Vdev};
use crate::identity::{
    BoardIdentity, IdentityNames, InputDeviceId, InputProductId, InputVendorId, InputVersion,
};

/// What Linux accepts of what the device offers.
const LINUX_FEATURES: u64 = F_VERSION_1 | F_INDIRECT_DESC | F_EVENT_IDX;

/// The device of `kind` on a board with the default identity.
fn input(kind: InputKind) -> Input {
    Input::for_board(kind, &BoardIdentity::RUSTY_BOX)
}

fn device(kind: InputKind) -> Driver {
    Driver::new(Device::Input(input(kind)))
}

/// Have the host send through the input device.
fn send<F: FnOnce(&mut Input, &mut Vdev<'_, '_>) -> bool>(d: &mut Driver, f: F) -> bool {
    d.host(|device, vdev| match device {
        Device::Input(input) => f(input, vdev),
        #[cfg(feature = "alloc")]
        Device::Gpu(_) | Device::Block(_) => panic!("not an input device"),
    })
}

/// Choose configuration entry `select`/`subsel` the way Linux's
/// `virtinput_cfg_select` does, a byte at a time, and read its size and
/// payload.
fn entry(d: &mut Driver, select: u8, subsel: u8) -> (u8, Vec<u8>) {
    d.write_width(CONFIG, AccessWidth::Byte, u64::from(select));
    d.write_width(CONFIG + 1, AccessWidth::Byte, u64::from(subsel));
    let size = d.read_width(CONFIG + 2, AccessWidth::Byte) as u8;
    let payload = (0..u64::from(size))
        .map(|i| d.read_width(CONFIG + 8 + i, AccessWidth::Byte) as u8)
        .collect();
    (size, payload)
}

/// Queue `n` empty event buffers; their addresses, by descriptor index.
fn event_buffers(d: &mut Driver, n: usize) -> Vec<u64> {
    (0..n)
        .map(|_| {
            let at = d.alloc(8);
            d.offer(0, &[Buf::writable(at, 8)]);
            at
        })
        .collect()
}

/// The events the device delivered since the last call: type, code, value.
fn events(d: &mut Driver, buffers: &[u64]) -> Vec<(u16, u16, u32)> {
    d.used(0)
        .into_iter()
        .map(|(id, len)| {
            assert_eq!(len, 8, "a whole struct virtio_input_event");
            let b = d.mem(buffers[id as usize], 8);
            (
                u16::from_le_bytes([b[0], b[1]]),
                u16::from_le_bytes([b[2], b[3]]),
                u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            )
        })
        .collect()
}

const SYN: (u16, u16, u32) = (0, 0, 0);

/// The configuration is QEMU's, but for the name, vendor, product and
/// version, which are the board's own.
#[test]
fn the_keyboard_configuration_is_qemus_with_the_boards_name() {
    let mut d = device(InputKind::Keyboard);
    let (size, name) = entry(&mut d, 0x01, 0);
    assert_eq!(size, 25, "the name's length: a string has no NUL");
    assert_eq!(name, b"Rusty Box Virtio Keyboard");
    // BUS_VIRTUAL, vendor "RB" (0x5242), product 1, version 1.
    assert_eq!(
        entry(&mut d, 0x03, 0),
        (8, [6, 0, 0x42, 0x52, 1, 0, 1, 0].to_vec())
    );
    // EV_KEY: every code from KEY_ESC below KEY_REPLY (232).
    let (size, keys) = entry(&mut d, 0x11, 0x01);
    assert_eq!(size, 29);
    assert_eq!(keys[0], 0xFE);
    assert!(keys[1..].iter().all(|&b| b == 0xFF));
    assert_eq!(
        entry(&mut d, 0x11, 0x14),
        (1, [0].to_vec()),
        "EV_REP, no codes"
    );
    assert_eq!(
        entry(&mut d, 0x11, 0x11),
        (1, [0x07].to_vec()),
        "EV_LED: NumLock, CapsLock, ScrollLock"
    );
    assert_eq!(entry(&mut d, 0x11, 0x02), (0, Vec::new()), "no EV_REL");
    // The configuration is the header and the largest entry.
    assert_eq!(d.read_width(CONFIG + 37, AccessWidth::Byte), 0xFF);
}

/// A host's identity names its input devices: the driver reads the
/// keyboard's and tablet's names, each sized to its length with no NUL
/// (virtio 1.2, 5.8.4), and their vendor.
#[test]
fn a_hosts_identity_names_the_input_devices() {
    let id = |product, version| InputDeviceId {
        product: InputProductId::new(product),
        version: InputVersion::new(version),
    };
    let identity = BoardIdentity::new(IdentityNames {
        keyboard: "Acme Keys",
        keyboard_id: id(0x0101, 0x0203),
        tablet: "Acme Pad",
        tablet_id: id(0x0405, 0x0607),
        input_vendor: InputVendorId::new(0xACE5),
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"));
    for (kind, name, product_and_version) in [
        (
            InputKind::Keyboard,
            &b"Acme Keys"[..],
            [0x01, 0x01, 0x03, 0x02],
        ),
        (
            InputKind::Tablet,
            &b"Acme Pad"[..],
            [0x05, 0x04, 0x07, 0x06],
        ),
    ] {
        let mut d = Driver::new(Device::Input(Input::for_board(kind, &identity)));
        assert_eq!(entry(&mut d, 0x01, 0), (name.len() as u8, name.to_vec()));
        let (_, ids) = entry(&mut d, 0x03, 0);
        assert_eq!(ids[2..4], [0xE5, 0xAC], "the vendor, little-endian");
        assert_eq!(
            ids[4..],
            product_and_version,
            "the product and version, little-endian"
        );
    }
    // The longest name an identity holds fills the whole 128-byte string.
    let longest = "k".repeat(crate::identity::INPUT_NAME_CAPACITY);
    let identity = BoardIdentity::new(IdentityNames {
        keyboard: &longest,
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let mut d = Driver::new(Device::Input(Input::for_board(
        InputKind::Keyboard,
        &identity,
    )));
    let (size, name) = entry(&mut d, 0x01, 0);
    assert_eq!(size, 128, "the whole string field");
    assert_eq!(name, longest.as_bytes());
}

#[test]
fn the_tablet_configuration_is_qemus_with_the_boards_name() {
    let mut d = device(InputKind::Tablet);
    assert_eq!(
        entry(&mut d, 0x01, 0),
        (23, b"Rusty Box Virtio Tablet".to_vec())
    );
    assert_eq!(
        entry(&mut d, 0x03, 0),
        (8, [6, 0, 0x42, 0x52, 2, 0, 1, 0].to_vec()),
        "BUS_VIRTUAL, vendor \"RB\", product 2, version 1"
    );
    // EV_KEY: keymap_button's codes, up to BTN_GEAR_UP (0x151).
    let (size, buttons) = entry(&mut d, 0x11, 0x01);
    assert_eq!(size, 43);
    let set: Vec<u16> = (0..43 * 8)
        .filter(|&c| buttons[usize::from(c / 8)] & (1 << (c % 8)) != 0)
        .collect();
    assert_eq!(
        set,
        [0x110, 0x111, 0x112, 0x113, 0x114, 0x14A, 0x150, 0x151]
    );
    assert_eq!(
        entry(&mut d, 0x11, 0x02),
        (2, [0, 0x01].to_vec()),
        "EV_REL: REL_WHEEL"
    );
    assert_eq!(
        entry(&mut d, 0x11, 0x03),
        (1, [0x03].to_vec()),
        "EV_ABS: ABS_X, ABS_Y"
    );
    for axis in [0, 1] {
        let (size, info) = entry(&mut d, 0x12, axis);
        assert_eq!(size, 20);
        assert_eq!(
            info[..8],
            [0, 0, 0, 0, 0xFF, 0x7F, 0, 0],
            "min 0, max INPUT_EVENT_ABS_MAX"
        );
        assert!(info[8..].iter().all(|&b| b == 0));
    }
    assert_eq!(d.read_width(CONFIG + 50, AccessWidth::Byte), 0);
    assert_eq!(d.read_width(CONFIG + 51, AccessWidth::Byte), 0xFF);
}

#[test]
fn a_selector_with_no_entry_reads_all_zeros() {
    let mut d = device(InputKind::Keyboard);
    entry(&mut d, 0x11, 0x04);
    for off in 0..37 {
        assert_eq!(
            d.read_width(CONFIG + off, AccessWidth::Byte),
            0,
            "byte {off}, select and subsel included"
        );
    }
}

#[test]
fn the_selection_survives_a_device_reset() {
    let mut d = device(InputKind::Keyboard);
    entry(&mut d, 0x01, 0);
    d.start(LINUX_FEATURES);
    d.write(STATUS, 0);
    assert_eq!(
        d.read_width(CONFIG + 2, AccessWidth::Byte),
        25,
        "QEMU's reset leaves cfg_select alone: the name's size"
    );
}

#[test]
fn keys_reach_the_guest_each_with_its_sync() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let buffers = event_buffers(&mut d, 8);
    let stroke = [
        (Key::ShiftL, KeyAction::Press),
        (Key::A, KeyAction::Press),
        (Key::A, KeyAction::Release),
        (Key::ShiftL, KeyAction::Release),
    ];
    assert!(send(&mut d, |input, vdev| input.keys(vdev, &stroke)));
    assert_eq!(
        events(&mut d, &buffers),
        [
            (1, 42, 1),
            SYN,
            (1, 30, 1),
            SYN,
            (1, 30, 0),
            SYN,
            (1, 42, 0),
            SYN
        ]
    );
    assert_eq!(d.read(INTERRUPT_STATUS), 1);
    assert!(d.irq());
}

#[test]
fn nothing_is_delivered_before_driver_ok() {
    let mut d = device(InputKind::Keyboard);
    d.negotiate(LINUX_FEATURES);
    d.setup_queue(0);
    d.setup_queue(1);
    let buffers = event_buffers(&mut d, 2);
    assert!(!send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    assert!(events(&mut d, &buffers).is_empty());
}

#[test]
fn an_offer_without_a_buffer_for_every_event_is_refused_whole() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let mut buffers = event_buffers(&mut d, 1);
    assert!(!send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    assert!(events(&mut d, &buffers).is_empty());
    assert_eq!(d.avail_event(0), 0, "the device took no buffer");
    assert!(!d.irq());

    buffers.extend(event_buffers(&mut d, 1));
    assert!(send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    assert_eq!(events(&mut d, &buffers), [(1, 30, 1), SYN]);
}

#[test]
fn the_tablet_reports_positions_buttons_and_the_wheel() {
    let mut d = device(InputKind::Tablet);
    d.start(LINUX_FEATURES);
    let buffers = event_buffers(&mut d, 16);
    assert!(send(&mut d, |input, vdev| input
        .move_to(vdev, PointerPos::new(100, 200))));
    assert_eq!(events(&mut d, &buffers), [(3, 0, 100), (3, 1, 200), SYN]);
    assert!(send(&mut d, |input, vdev| input.button(
        vdev,
        PointerButton::Left,
        KeyAction::Press
    )));
    assert!(send(&mut d, |input, vdev| input.button(
        vdev,
        PointerButton::Left,
        KeyAction::Release
    )));
    assert_eq!(
        events(&mut d, &buffers),
        [(1, 0x110, 1), SYN, (1, 0x110, 0), SYN]
    );
    // A detent is the wheel button pressed (a REL_WHEEL step) and released
    // (its BTN_GEAR key going up).
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(1))));
    assert_eq!(
        events(&mut d, &buffers),
        [(2, 8, 1), SYN, (1, 0x151, 0), SYN]
    );
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(-1))));
    assert_eq!(
        events(&mut d, &buffers),
        [(2, 8, u32::MAX), SYN, (1, 0x150, 0), SYN]
    );
}

/// A detent is four events, so a full event queue of 64 buffers takes a
/// turn of 16 detents whole, and a longer turn — however long — is refused
/// with nothing delivered.
#[test]
fn a_wheel_turn_longer_than_the_queue_is_refused_whole() {
    let mut d = device(InputKind::Tablet);
    d.start(LINUX_FEATURES);
    let buffers = event_buffers(&mut d, 64);
    assert!(!send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(17))));
    assert!(!send(&mut d, |input, vdev| input
        .wheel(vdev, WheelSteps(i32::MIN))));
    assert!(events(&mut d, &buffers).is_empty());
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(-16))));
    assert_eq!(events(&mut d, &buffers).len(), 64);
}

#[test]
fn each_device_takes_only_its_own_events() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    event_buffers(&mut d, 8);
    assert!(!send(&mut d, |input, vdev| input
        .move_to(vdev, PointerPos::new(1, 1))));
    assert!(!send(&mut d, |input, vdev| input.button(
        vdev,
        PointerButton::Left,
        KeyAction::Press
    )));
    assert!(!send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(1))));

    let mut d = device(InputKind::Tablet);
    d.start(LINUX_FEATURES);
    event_buffers(&mut d, 8);
    assert!(!send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
}

#[test]
fn status_reports_are_taken_with_their_length() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let led = d.alloc_with(&[0x11, 0, 1, 0, 1, 0, 0, 0]);
    let head = d.offer(1, &[Buf::readable(led, 8)]);
    d.notify(1);
    assert_eq!(d.used(1), [(u32::from(head), 8)]);
    let short = d.alloc_with(&[0x11, 0, 1, 0]);
    let head = d.offer(1, &[Buf::readable(short, 4)]);
    d.notify(1);
    assert_eq!(d.used(1), [(u32::from(head), 4)]);
}

#[test]
fn a_reset_makes_the_device_inactive() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    d.write(STATUS, 0);
    d.negotiate(LINUX_FEATURES);
    d.setup_queue(0);
    d.setup_queue(1);
    let buffers = event_buffers(&mut d, 2);
    assert!(!send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    d.write(STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
    assert!(send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    assert_eq!(events(&mut d, &buffers), [(1, 30, 1), SYN]);
}

#[test]
fn a_snapshot_keeps_the_device_active_and_its_selection() {
    let mut d = device(InputKind::Tablet);
    entry(&mut d, 0x12, 1);
    d.start(LINUX_FEATURES);
    let buffers = event_buffers(&mut d, 3);
    let saved = d.save();
    let mut resumed = d.restored(Device::Input(input(InputKind::Tablet)), &saved);
    assert_eq!(resumed.read_width(CONFIG + 2, AccessWidth::Byte), 20);
    assert!(send(&mut resumed, |input, vdev| input
        .move_to(vdev, PointerPos::new(7, 9))));
    assert_eq!(events(&mut resumed, &buffers), [(3, 0, 7), (3, 1, 9), SYN]);
}
