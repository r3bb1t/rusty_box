//! Unit tests of `src/devices/virtio/input.rs`: the virtio keyboard and
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

/// The codes whose bits a configuration bitmap sets.
fn codes(bitmap: &[u8]) -> Vec<u16> {
    (0..bitmap.len() as u16 * 8)
        .filter(|&c| bitmap[usize::from(c / 8)] & (1 << (c % 8)) != 0)
        .collect()
}

/// The `EV_KEY` codes the keyboard sends, one key after another, for every
/// key the host can press.
fn codes_the_keyboard_sends() -> Vec<u16> {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let mut sent = Vec::new();
    // A key is two events; each round fills the event queue afresh, so a
    // buffer's descriptor index is its place in the round.
    let queue = usize::from(super::QUEUE_SIZE);
    for keys in Key::ALL.chunks(queue / 2) {
        let buffers = event_buffers(&mut d, queue);
        let presses: Vec<(Key, KeyAction)> =
            keys.iter().map(|&key| (key, KeyAction::Press)).collect();
        assert!(send(&mut d, |input, vdev| input.keys(vdev, &presses)));
        for (kind, code, value) in events(&mut d, &buffers) {
            if (kind, code, value) != SYN {
                assert_eq!((kind, value), (1, 1), "an EV_KEY press");
                sent.push(code);
            }
        }
    }
    sent.sort_unstable();
    sent.dedup();
    sent
}

/// The keyboard offers exactly the keys the host can send it, each of
/// which it sends as its Linux code, so a key the guest is told of can be
/// pressed and a key pressed was one it was told of. The configuration is
/// the board's own throughout: the name, vendor, product and version its
/// identity's.
#[test]
fn the_keyboard_configuration_is_the_boards_own() {
    let mut d = device(InputKind::Keyboard);
    let (size, name) = entry(&mut d, 0x01, 0);
    assert_eq!(size, 25, "the name's length: a string has no NUL");
    assert_eq!(name, b"Rusty Box Virtio Keyboard");
    // BUS_VIRTUAL, vendor "RB" (0x5242), product 1, version 1.
    assert_eq!(
        entry(&mut d, 0x03, 0),
        (8, [6, 0, 0x42, 0x52, 1, 0, 1, 0].to_vec())
    );
    let (size, keys) = entry(&mut d, 0x11, 0x01);
    assert_eq!(
        size, 28,
        "up to KEY_SEARCH (217), the highest code a key has"
    );
    let offered = codes(&keys);
    assert_eq!(offered, codes_the_keyboard_sends());
    assert_eq!(
        offered.len(),
        Key::COUNT - 2,
        "Print and Alt+SysRq are KEY_SYSRQ, Pause and Ctrl+Break KEY_PAUSE"
    );
    // udev's `input_id` calls a device with every code from KEY_ESC to
    // KEY_S (1 to 31) a keyboard (ID_INPUT_KEYBOARD).
    assert!((1..=31).all(|c| offered.contains(&c)));
    assert_eq!(
        entry(&mut d, 0x11, 0x14),
        (1, [0].to_vec()),
        "EV_REP, no codes: the guest repeats a held key"
    );
    assert_eq!(
        entry(&mut d, 0x11, 0x11),
        (1, [0x07].to_vec()),
        "EV_LED: NumLock, CapsLock, ScrollLock, one for each lock key"
    );
    assert_eq!(entry(&mut d, 0x11, 0x02), (0, Vec::new()), "no EV_REL");
    assert_eq!(entry(&mut d, 0x11, 0x03), (0, Vec::new()), "no EV_ABS");
    assert_eq!(entry(&mut d, 0x10, 0), (0, Vec::new()), "no properties");
    assert_eq!(entry(&mut d, 0x02, 0), (0, Vec::new()), "no serial number");
    // The configuration is `struct virtio_input_config`, the union's bytes
    // past the entry zero.
    entry(&mut d, 0x11, 0x01);
    assert_eq!(d.read_width(CONFIG + 36, AccessWidth::Byte), 0);
    assert_eq!(d.read_width(CONFIG + 135, AccessWidth::Byte), 0);
    assert_eq!(d.read_width(CONFIG + 136, AccessWidth::Byte), 0xFF);
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

/// The tablet offers exactly the buttons the host's pointer has, its two
/// axes over the whole 16-bit range a position carries, and its wheel; the
/// name, vendor, product and version are its identity's.
#[test]
fn the_tablet_configuration_is_the_boards_own() {
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
    // EV_KEY: BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE and BTN_EXTRA, each
    // the code of a button the host presses.
    let (size, buttons) = entry(&mut d, 0x11, 0x01);
    assert_eq!(size, 35, "up to BTN_EXTRA (0x114)");
    assert_eq!(codes(&buttons), [0x110, 0x111, 0x112, 0x113, 0x114]);
    let mut sent = Vec::new();
    let mut tablet = device(InputKind::Tablet);
    tablet.start(LINUX_FEATURES);
    let buffers = event_buffers(&mut tablet, 2 * PointerButton::ALL.len());
    for button in PointerButton::ALL {
        assert!(send(&mut tablet, |input, vdev| input.button(
            vdev,
            button,
            KeyAction::Press
        )));
    }
    for (kind, code, _) in events(&mut tablet, &buffers) {
        if kind == 1 {
            sent.push(code);
        }
    }
    sent.sort_unstable();
    assert_eq!(codes(&buttons), sent, "the buttons offered are those sent");
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
            [0, 0, 0, 0, 0xFF, 0xFF, 0, 0],
            "min 0, max 0xFFFF: a position's whole 16 bits"
        );
        assert!(
            info[8..].iter().all(|&b| b == 0),
            "fuzz, flat and resolution 0"
        );
    }
    assert_eq!(entry(&mut d, 0x12, 2), (0, Vec::new()), "no ABS_Z");
    assert_eq!(entry(&mut d, 0x11, 0x11), (0, Vec::new()), "no EV_LED");
    assert_eq!(entry(&mut d, 0x10, 0), (0, Vec::new()), "no properties");
    entry(&mut d, 0x12, 0);
    assert_eq!(d.read_width(CONFIG + 28, AccessWidth::Byte), 0);
    assert_eq!(d.read_width(CONFIG + 136, AccessWidth::Byte), 0xFF);
}

/// A selector naming no entry reads `size` 0 and no data (virtio 1.2
/// 5.8.5.2), and `select` and `subsel` as written.
#[test]
fn a_selector_with_no_entry_reads_size_zero() {
    let mut d = device(InputKind::Keyboard);
    entry(&mut d, 0x11, 0x04);
    assert_eq!(d.read_width(CONFIG, AccessWidth::Byte), 0x11, "select");
    assert_eq!(d.read_width(CONFIG + 1, AccessWidth::Byte), 0x04, "subsel");
    for off in 2..136 {
        assert_eq!(d.read_width(CONFIG + off, AccessWidth::Byte), 0, "byte {off}");
    }
}

/// The driver may write `select` and `subsel` in either order (5.8.5.1),
/// reading between them: the first written stays, though what it and the
/// old `subsel` name has no entry.
#[test]
fn select_and_subsel_may_be_written_in_either_order() {
    let mut d = device(InputKind::Keyboard);
    entry(&mut d, 0x01, 0);
    d.write_width(CONFIG + 1, AccessWidth::Byte, 0x01);
    assert_eq!(d.read_width(CONFIG + 2, AccessWidth::Byte), 0, "(0x01, 0x01): none");
    d.write_width(CONFIG, AccessWidth::Byte, 0x11);
    assert_eq!(
        d.read_width(CONFIG + 2, AccessWidth::Byte),
        28,
        "(0x11, 0x01): EV_KEY's bitmap"
    );
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
    assert!(send(&mut d, |input, vdev| input
        .move_to(vdev, PointerPos::new(PointerPos::MAX, 0))));
    assert_eq!(
        events(&mut d, &buffers),
        [(3, 0, 0xFFFF), (3, 1, 0), SYN],
        "the right edge is the axis's maximum"
    );
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
    // A turn of the wheel is one REL_WHEEL event of its detents, signed.
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(1))));
    assert_eq!(events(&mut d, &buffers), [(2, 8, 1), SYN]);
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(-1))));
    assert_eq!(events(&mut d, &buffers), [(2, 8, u32::MAX), SYN]);
}

/// However far the wheel turns, the turn is one event and its sync, so it
/// fits two buffers; a turn of no detents sends nothing.
#[test]
fn a_wheel_turn_is_one_event_however_long() {
    let mut d = device(InputKind::Tablet);
    d.start(LINUX_FEATURES);
    let buffers = event_buffers(&mut d, 4);
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(17))));
    assert!(send(&mut d, |input, vdev| input
        .wheel(vdev, WheelSteps(i32::MIN))));
    assert_eq!(
        events(&mut d, &buffers),
        [(2, 8, 17), SYN, (2, 8, 0x8000_0000), SYN]
    );
    assert!(send(&mut d, |input, vdev| input.wheel(vdev, WheelSteps(0))));
    assert!(events(&mut d, &buffers).is_empty());
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

/// An event queue whose `QueueReady` is 0 takes no host event (virtio 1.2
/// 4.2.2.1): its buffers, which the driver may have freed, stay untouched,
/// and the host may offer the event again once it is ready.
#[test]
fn a_host_event_waits_while_the_event_queue_is_not_ready() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let bufs = event_buffers(&mut d, 4);
    d.write(QUEUE_SEL, 0);
    d.write(QUEUE_READY, 0);
    assert!(!send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    assert!(d.used(0).is_empty());
    assert_eq!(d.mem(bufs[0], 8), [0; 8], "untouched");
    d.write(QUEUE_READY, 1);
    assert!(send(&mut d, |input, vdev| input
        .keys(vdev, &[(Key::A, KeyAction::Press)])));
    assert_eq!(d.used(0).len(), 2, "the key and its sync");
}

/// A status report is device-readable, so it comes back with nothing
/// written (virtio 1.2 2.7.8.2), and the driver is interrupted for it; a
/// notification that finds no report interrupts nothing.
#[test]
fn status_reports_are_taken_with_nothing_written() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let led = d.alloc_with(&[0x11, 0, 1, 0, 1, 0, 0, 0]);
    let head = d.offer(1, &[Buf::readable(led, 8)]);
    d.notify(1);
    assert_eq!(d.used(1), [(u32::from(head), 0)]);
    assert_eq!(d.take_interrupt(), 1);
    let short = d.alloc_with(&[0x11, 0, 1, 0]);
    let head = d.offer(1, &[Buf::readable(short, 4)]);
    d.notify(1);
    assert_eq!(d.used(1), [(u32::from(head), 0)]);
    d.take_interrupt();
    d.notify(1);
    assert_eq!(d.read(INTERRUPT_STATUS), 0, "nothing taken, no interrupt");
}

/// A report given back before a later one in the same notification breaks
/// the device is still notified: `InterruptStatus` shows the used buffer
/// and the configuration change, each an event that took place (virtio 1.2
/// 4.2.2.1).
#[test]
fn a_report_given_back_before_the_device_breaks_is_notified() {
    let mut d = device(InputKind::Keyboard);
    d.start(LINUX_FEATURES);
    let led = d.alloc_with(&[0x11, 0, 1, 0, 1, 0, 0, 0]);
    let head = d.offer(1, &[Buf::readable(led, 8)]);
    d.offer(1, &[Buf::readable(UNASSIGNED, 8)]);
    d.notify(1);
    assert_eq!(d.used(1), [(u32::from(head), 0)]);
    assert_eq!(d.read(STATUS) & NEEDS_RESET, NEEDS_RESET);
    assert_eq!(d.read(INTERRUPT_STATUS), 0b11);
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
