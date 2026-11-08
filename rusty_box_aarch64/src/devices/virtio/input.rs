//! virtio-input: QEMU's `virtio-keyboard-device` and `virtio-tablet-device`.
//!
//! Provenance (R7): the virtio 1.2 specification, section 5.8, as QEMU
//! implements it in `hw/input/virtio-input.c` and
//! `hw/input/virtio-input-hid.c`:
//!
//! - `virtio_input_send`: the events up to a `SYN_REPORT` go to the guest
//!   together, a buffer each, with one notification; they are dropped while
//!   the driver is not active, and when it has not queued a buffer for each
//!   (the buffers taken so far are put back);
//! - `virtio_input_handle_sts`: the driver's status events (the keyboard
//!   LEDs) are taken;
//! - `virtio_input_get_config` and `virtio_input_set_config`: the
//!   configuration space shows the entry `select`/`subsel` name; writing it
//!   chooses the entry and notifies a configuration change;
//! - `virtio_input_set_status` and `virtio_input_reset`: the device is
//!   active from `DRIVER_OK` until reset, which the transport's status
//!   holds;
//! - `virtio_keyboard_config` and `virtio_tablet_config`: the keyboard
//!   offers `EV_KEY`, `EV_REP` and `EV_LED`, the tablet `EV_KEY`, `EV_REL`
//!   and `EV_ABS`, each with its bitmap built as
//!   `virtio_input_extend_config` builds one, sized to its highest set
//!   byte;
//! - `virtio_input_handle_event`: a key or button is `EV_KEY` and a pointer
//!   position `EV_ABS`.
//!
//! Event and key codes are Linux's (`include/uapi/linux/input-event-codes.h`),
//! whose evdev events virtio 1.2 carries (5.8, 5.8.6). The host's events
//! arrive as QEMU's display front ends send them (`ui/gtk.c`, `ui/sdl2.c`):
//! a key, then a sync; a position as both axes, then a sync; a button, then
//! a sync.
//!
//! What the devices offer is the board's own, which virtio 1.2 leaves to
//! the device (5.8.4: the codes are "implementation-defined"), each set
//! derived from the board's own vocabulary so that it names exactly what
//! the host can send (README, "Where the references differ"):
//!
//! - the keyboard's keys are the Linux code of each [`Key`], and its LEDs
//!   one for each lock key among them (`LED_NUML`, `LED_CAPSL`,
//!   `LED_SCROLLL`); `EV_REP` with no codes, as the device sends a key's
//!   presses and releases and never a repeat (value 2), so a Linux driver
//!   repeats a held key itself (`Documentation/input/input-programming.rst`,
//!   "Key autorepeat");
//! - the tablet's buttons are the Linux code of each [`PointerButton`]; its
//!   axes `ABS_X` and `ABS_Y`, each from 0 to [`PointerPos::MAX`], the whole
//!   16-bit coordinate a position carries, with no fuzz or flat, as a
//!   position is exact (`input-programming.rst`: zero "means that the thing
//!   is precise"), and resolution 0, as a position is a fraction of a
//!   screen whose size in millimetres the device does not know; its wheel
//!   `REL_WHEEL`, whose value is "the number of detents moved"
//!   (`event-codes.rst`), so a turn of the wheel is one event and its
//!   sync, and no `REL_WHEEL_HI_RES`, as the host reports whole detents
//!   ([`WheelSteps`]);
//! - no input properties: Linux deduces the device's kind from the events
//!   it emits where no property is set (`Documentation/input/event-codes.rst`,
//!   "Device properties"), and its "Guidelines" set `INPUT_PROP_DIRECT`
//!   for touchscreens and pen tablets and `INPUT_PROP_POINTER` for
//!   touchpads and pen tablets, none of which a pointer with mouse buttons
//!   and absolute axes is.
//!
//! Where it follows the specification over QEMU (README, "Where the
//! references differ"): a status event comes back with nothing written
//! (2.7.8.2), where QEMU's gives the length it read; the configuration is
//! `struct virtio_input_config`'s 136 bytes, `select` and `subsel` as the
//! driver wrote them and `size` 0 when they name no entry (5.8.4, 5.8.5),
//! where QEMU's ends after the largest entry and reads all zeros, its
//! header included, when there is none.
//!
//! The host offers its events whole: when the driver has not queued enough
//! buffers for all of an offer, the device takes none of it, so the host can
//! offer it again later, where a QEMU front end sends each event once and
//! the device drops what does not fit. Either way the guest sees each batch
//! whole or not at all.
//!
//! The keyboard's LEDs, which QEMU hands to the host's keyboard
//! (`qemu_input_handler_set_leds_mask`), have no host keyboard to light
//! here: the status queue's reports are taken and acknowledged.

use rusty_box_core::input::pointer::{PointerButton, PointerPos, WheelSteps};
use rusty_box_core::input::{Key, KeyAction};
use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use super::queue::{self, Chain};
use super::{Core, Vdev};
use crate::identity::{BoardIdentity, InputDeviceId, InputVendorId, Text, INPUT_NAME_CAPACITY};

/// Which input device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputKind {
    Keyboard,
    Tablet,
}

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const EV_LED: u16 = 0x11;
const EV_REP: u16 = 0x14;
const SYN_REPORT: u16 = 0;
const REL_WHEEL: u16 = 0x08;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const LED_NUML: u8 = 0;
const LED_CAPSL: u8 = 1;
const LED_SCROLLL: u8 = 2;
const KEY_CAPSLOCK: u16 = 58;
const KEY_NUMLOCK: u16 = 69;
const KEY_SCROLLLOCK: u16 = 70;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const BTN_SIDE: u16 = 0x113;
const BTN_EXTRA: u16 = 0x114;
const BUS_VIRTUAL: u16 = 0x06;

const CFG_ID_NAME: u8 = 0x01;
const CFG_ID_DEVIDS: u8 = 0x03;
const CFG_EV_BITS: u8 = 0x11;
const CFG_ABS_INFO: u8 = 0x12;

/// The event queue and the status queue (virtio 1.2 5.8.2), each of the
/// board's queue size ([`queue::QUEUE_SIZE`]): events wait in the event
/// queue's buffers until the guest takes them, so its size is how many the
/// guest can fall behind by before the host has to hold its offers. A
/// guest may keep fewer buffers there than the queue holds (Linux's driver
/// keeps 64, `struct virtio_input`'s `evts`).
pub const EVENT_QUEUE: usize = 0;
pub const STATUS_QUEUE: usize = 1;
pub const QUEUE_SIZE: u16 = queue::QUEUE_SIZE;

/// The configuration's header: `select`, `subsel`, `size` and padding.
const CONFIG_HEADER_LEN: usize = 8;
/// The configuration's union `u` (5.8.4): its largest member, `bitmap`,
/// of 128 bytes.
const CONFIG_UNION_LEN: usize = 128;
/// Bytes of configuration space: the whole `struct virtio_input_config`
/// (5.8.4), its union's bytes past the selected entry zero. QEMU's
/// `virtio_input_device_realize` ends it after the largest entry, past
/// which the transport reads all ones.
pub const CONFIG_LEN: usize = CONFIG_HEADER_LEN + CONFIG_UNION_LEN;

/// The most events one batch has: both axes of a position and the sync.
const MAX_BATCH: usize = 3;

/// One `struct virtio_input_event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Event {
    kind: u16,
    code: u16,
    value: u32,
}

impl Event {
    const fn new(kind: u16, code: u16, value: u32) -> Self {
        Self { kind, code, value }
    }

    const SYN: Self = Self::new(EV_SYN, SYN_REPORT, 0);

    fn bytes(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..2].copy_from_slice(&self.kind.to_le_bytes());
        b[2..4].copy_from_slice(&self.code.to_le_bytes());
        b[4..].copy_from_slice(&self.value.to_le_bytes());
        b
    }
}

/// Events ending in a `SYN_REPORT`, which `virtio_input_send` hands to the
/// guest together.
#[derive(Debug, Clone, Copy)]
struct Batch {
    events: [Event; MAX_BATCH],
    len: usize,
}

impl Batch {
    /// `events` and the sync after them.
    fn of(events: &[Event]) -> Self {
        let mut batch = Self {
            events: [Event::SYN; MAX_BATCH],
            len: events.len() + 1,
        };
        batch.events[..events.len()].copy_from_slice(events);
        batch
    }

    fn events(&self) -> &[Event] {
        &self.events[..self.len]
    }
}

/// `1` for a press, `0` for a release.
fn key_value(action: KeyAction) -> u32 {
    match action {
        KeyAction::Press => 1,
        KeyAction::Release => 0,
    }
}

/// The Linux key code (`input-event-codes.h`) of the key `key` is, as QEMU's
/// `qemu_input_map_qcode_to_linux` gives it for the qcode of each Bochs key.
pub const fn linux_key_code(key: Key) -> u16 {
    match key {
        Key::CtrlL => 29,
        Key::ShiftL => 42,
        Key::F1 => 59,
        Key::F2 => 60,
        Key::F3 => 61,
        Key::F4 => 62,
        Key::F5 => 63,
        Key::F6 => 64,
        Key::F7 => 65,
        Key::F8 => 66,
        Key::F9 => 67,
        Key::F10 => 68,
        Key::F11 => 87,
        Key::F12 => 88,
        Key::CtrlR => 97,
        Key::ShiftR => 54,
        Key::CapsLock => 58,
        Key::NumLock => 69,
        Key::AltL => 56,
        Key::AltR => 100,
        Key::A => 30,
        Key::B => 48,
        Key::C => 46,
        Key::D => 32,
        Key::E => 18,
        Key::F => 33,
        Key::G => 34,
        Key::H => 35,
        Key::I => 23,
        Key::J => 36,
        Key::K => 37,
        Key::L => 38,
        Key::M => 50,
        Key::N => 49,
        Key::O => 24,
        Key::P => 25,
        Key::Q => 16,
        Key::R => 19,
        Key::S => 31,
        Key::T => 20,
        Key::U => 22,
        Key::V => 47,
        Key::W => 17,
        Key::X => 45,
        Key::Y => 21,
        Key::Z => 44,
        Key::K0 => 11,
        Key::K1 => 2,
        Key::K2 => 3,
        Key::K3 => 4,
        Key::K4 => 5,
        Key::K5 => 6,
        Key::K6 => 7,
        Key::K7 => 8,
        Key::K8 => 9,
        Key::K9 => 10,
        Key::Esc => 1,
        Key::Space => 57,
        Key::SingleQuote => 40,
        Key::Comma => 51,
        Key::Period => 52,
        Key::Slash => 53,
        Key::Semicolon => 39,
        Key::Equals => 13,
        Key::LeftBracket => 26,
        Key::Backslash => 43,
        Key::RightBracket => 27,
        Key::Minus => 12,
        Key::Grave => 41,
        Key::Backspace => 14,
        Key::Enter => 28,
        Key::Tab => 15,
        // KEY_102ND, the key left of Z on an ISO keyboard.
        Key::LeftBackslash => 86,
        // KEY_SYSRQ: Print Screen and Alt+SysRq are one key.
        Key::Print | Key::AltSysreq => 99,
        Key::ScrlLock => 70,
        // KEY_PAUSE: Ctrl+Break is Ctrl and the Pause key.
        Key::Pause | Key::CtrlBreak => 119,
        Key::Insert => 110,
        Key::Delete => 111,
        Key::Home => 102,
        Key::End => 107,
        Key::PageUp => 104,
        Key::PageDown => 109,
        Key::KpAdd => 78,
        Key::KpSubtract => 74,
        Key::KpEnd => 79,
        Key::KpDown => 80,
        Key::KpPageDown => 81,
        Key::KpLeft => 75,
        Key::KpRight => 77,
        Key::KpHome => 71,
        Key::KpUp => 72,
        Key::KpPageUp => 73,
        Key::KpInsert => 82,
        Key::KpDelete => 83,
        Key::Kp5 => 76,
        Key::Up => 103,
        Key::Down => 108,
        Key::Left => 105,
        Key::Right => 106,
        Key::KpEnter => 96,
        Key::KpMultiply => 55,
        Key::KpDivide => 98,
        Key::WinL => 125,
        Key::WinR => 126,
        // KEY_COMPOSE: QEMU's qcode "menu".
        Key::Menu => 127,
        Key::IntBack => 158,
        Key::IntForward => 159,
        Key::IntStop => 128,
        Key::IntMail => 155,
        Key::IntSearch => 217,
        Key::IntFav => 156,
        Key::IntHome => 172,
        Key::PowerMycomp => 157,
        Key::PowerCalc => 140,
        Key::PowerSleep => 142,
        Key::PowerPower => 116,
        Key::PowerWake => 143,
    }
}

/// The LED of the lock key whose Linux key code is `code`, if it is one: a
/// keyboard has an LED for each of its lock keys (Linux
/// `drivers/tty/vt/keyboard.c` lights `LED_NUML`, `LED_CAPSL` and
/// `LED_SCROLLL` for the Num, Caps and Scroll Lock states).
const fn lock_led(code: u16) -> Option<u8> {
    match code {
        KEY_NUMLOCK => Some(LED_NUML),
        KEY_CAPSLOCK => Some(LED_CAPSL),
        KEY_SCROLLLOCK => Some(LED_SCROLLL),
        _ => None,
    }
}

/// The Linux code of `button`, from `BTN_LEFT` to `BTN_EXTRA`.
const fn linux_button_code(button: PointerButton) -> u16 {
    match button {
        PointerButton::Left => BTN_LEFT,
        PointerButton::Right => BTN_RIGHT,
        PointerButton::Middle => BTN_MIDDLE,
        PointerButton::Side => BTN_SIDE,
        PointerButton::Extra => BTN_EXTRA,
    }
}

/// One entry of the configuration (`struct virtio_input_config` past its
/// header): its size and bytes.
struct ConfigEntry {
    size: u8,
    data: [u8; CONFIG_UNION_LEN],
}

impl ConfigEntry {
    fn from_bytes(bytes: &[u8]) -> Self {
        let mut entry = Self {
            size: bytes.len() as u8,
            data: [0; CONFIG_UNION_LEN],
        };
        entry.data[..bytes.len()].copy_from_slice(bytes);
        entry
    }

    /// `name`, its size the string's length: a string here has no NUL
    /// terminator (virtio 1.2, 5.8.4).
    fn name(name: &Text<INPUT_NAME_CAPACITY>) -> Self {
        Self::from_bytes(name.as_bytes())
    }

    /// A bitmap with bit `code` set for every code, sized to its highest
    /// set byte (QEMU `virtio_input_extend_config`).
    fn bitmap<I: Iterator<Item = u16>>(codes: I) -> Self {
        let mut entry = Self {
            size: 0,
            data: [0; CONFIG_UNION_LEN],
        };
        for code in codes {
            let byte = usize::from(code / 8);
            entry.data[byte] |= 1 << (code % 8);
            entry.size = entry.size.max(byte as u8 + 1);
        }
        entry
    }
}

/// A virtio-input device.
pub struct Input {
    kind: InputKind,
    /// What `VIRTIO_INPUT_CFG_ID_NAME` reads.
    name: Text<INPUT_NAME_CAPACITY>,
    /// The vendor `VIRTIO_INPUT_CFG_ID_DEVIDS` reads.
    vendor: InputVendorId,
    /// The product and version `VIRTIO_INPUT_CFG_ID_DEVIDS` reads.
    id: InputDeviceId,
    select: u8,
    subsel: u8,
}

impl Input {
    /// The device of `kind` a board named `identity` plugs in: of the name,
    /// vendor, product and version the identity gives it.
    pub(crate) const fn for_board(kind: InputKind, identity: &BoardIdentity) -> Self {
        Self {
            kind,
            name: identity.input_name(kind),
            vendor: identity.input_vendor(),
            id: identity.input_id(kind),
            select: 0,
            subsel: 0,
        }
    }

    pub fn kind(&self) -> InputKind {
        self.kind
    }

    /// The configuration entry `select`/`subsel` names, if the device has
    /// one.
    fn entry(&self, select: u8, subsel: u8) -> Option<ConfigEntry> {
        let subsel = u16::from(subsel);
        Some(match (self.kind, select) {
            (_, CFG_ID_NAME) if subsel == 0 => ConfigEntry::name(&self.name),
            (_, CFG_ID_DEVIDS) if subsel == 0 => {
                let mut ids = [0u8; 8];
                ids[..2].copy_from_slice(&BUS_VIRTUAL.to_le_bytes());
                ids[2..4].copy_from_slice(&self.vendor.get().to_le_bytes());
                ids[4..6].copy_from_slice(&self.id.product.get().to_le_bytes());
                ids[6..].copy_from_slice(&self.id.version.get().to_le_bytes());
                ConfigEntry::from_bytes(&ids)
            }
            // Every key the board's keyboard has.
            (InputKind::Keyboard, CFG_EV_BITS) if subsel == EV_KEY => {
                ConfigEntry::bitmap(Key::ALL.into_iter().map(linux_key_code))
            }
            // Supported, with no codes: the device never sends a repeat, so
            // the driver repeats a held key.
            (InputKind::Keyboard, CFG_EV_BITS) if subsel == EV_REP => ConfigEntry::from_bytes(&[0]),
            // An LED for each lock key among them.
            (InputKind::Keyboard, CFG_EV_BITS) if subsel == EV_LED => ConfigEntry::bitmap(
                Key::ALL
                    .into_iter()
                    .filter_map(|key| lock_led(linux_key_code(key)))
                    .map(u16::from),
            ),
            // Every button the board's pointer has.
            (InputKind::Tablet, CFG_EV_BITS) if subsel == EV_KEY => {
                ConfigEntry::bitmap(PointerButton::ALL.into_iter().map(linux_button_code))
            }
            (InputKind::Tablet, CFG_EV_BITS) if subsel == EV_REL => {
                ConfigEntry::bitmap(core::iter::once(REL_WHEEL))
            }
            (InputKind::Tablet, CFG_EV_BITS) if subsel == EV_ABS => {
                ConfigEntry::bitmap([ABS_X, ABS_Y].into_iter())
            }
            (InputKind::Tablet, CFG_ABS_INFO) if subsel == ABS_X || subsel == ABS_Y => {
                // min 0 and max `PointerPos::MAX`, a position's whole range;
                // fuzz, flat and res 0: a position is exact, and a fraction
                // of a screen of no known physical size.
                let mut info = [0u8; 20];
                info[4..8].copy_from_slice(&u32::from(PointerPos::MAX).to_le_bytes());
                ConfigEntry::from_bytes(&info)
            }
            // No serial number, no input properties, and nothing for any
            // other selector.
            _ => return None,
        })
    }

    /// QEMU `virtio_input_get_config`: `select` and `subsel` as the driver
    /// wrote them, which it may do in either order (5.8.5.1), and the entry
    /// they name, `size` 0 and no data where there is none (5.8.5.2). QEMU
    /// zeroes `select` and `subsel` too then, so a driver that writes one,
    /// reads, then writes the other finds the first gone.
    pub fn get_config(&self, config: &mut [u8]) {
        config.fill(0);
        config[0] = self.select;
        config[1] = self.subsel;
        if let Some(entry) = self.entry(self.select, self.subsel) {
            config[2] = entry.size;
            let payload = config.len() - CONFIG_HEADER_LEN;
            config[CONFIG_HEADER_LEN..].copy_from_slice(&entry.data[..payload]);
        }
    }

    /// QEMU `virtio_input_set_config`.
    pub fn set_config(&mut self, config: &[u8], core: &mut Core) {
        self.select = config[0];
        self.subsel = config[1];
        core.notify_config();
    }

    /// The driver notified a queue: the status queue's reports are taken
    /// (QEMU `virtio_input_handle_sts`), each given back with no byte
    /// written (2.7.8.2: the used length counts what the device wrote, and
    /// a report is device-readable), and the driver interrupted for each
    /// as it is given back, so a later report that breaks the device does
    /// not hide those before it; the event queue has nothing to do until
    /// the host sends an event. QEMU gives a report back with the bytes it
    /// read, and interrupts once whether or not it took one.
    pub fn handle_output(&mut self, vdev: &mut Vdev<'_, '_>, q: usize) {
        if q != STATUS_QUEUE {
            return;
        }
        while let Some(chain) = vdev.pop(STATUS_QUEUE) {
            vdev.push(STATUS_QUEUE, &chain, 0);
            vdev.notify(STATUS_QUEUE);
        }
    }

    /// Hand `batches` to the guest, each as `virtio_input_send` does once
    /// it sees the batch's sync. `false`, with nothing delivered, while the
    /// device is not active or the driver has not queued a buffer for every
    /// event; `false` too if the driver's buffers break a rule part of the
    /// way through, which leaves the device broken.
    fn deliver<I: Iterator<Item = Batch> + Clone>(
        &mut self,
        vdev: &mut Vdev<'_, '_>,
        batches: I,
    ) -> bool {
        if !vdev.driver_ok() {
            return false;
        }
        let needed: usize = batches.clone().map(|b| b.len).sum();
        if usize::from(vdev.available(EVENT_QUEUE)) < needed {
            return false;
        }
        for batch in batches {
            let mut chains: [Option<Chain>; MAX_BATCH] = [None; MAX_BATCH];
            for (taken, slot) in chains[..batch.len].iter_mut().enumerate() {
                match vdev.pop(EVENT_QUEUE) {
                    Some(chain) => *slot = Some(chain),
                    None => {
                        for _ in 0..taken {
                            vdev.unpop(EVENT_QUEUE);
                        }
                        return false;
                    }
                }
            }
            for (event, chain) in batch.events().iter().zip(chains.iter().flatten()) {
                let len = vdev.write(chain, 0, &event.bytes());
                vdev.push(EVENT_QUEUE, chain, len as u32);
            }
            vdev.notify(EVENT_QUEUE);
        }
        true
    }

    /// Keys, for the keyboard, each with its sync. A tablet takes none.
    pub fn keys(&mut self, vdev: &mut Vdev<'_, '_>, keys: &[(Key, KeyAction)]) -> bool {
        match self.kind {
            InputKind::Keyboard => self.deliver(
                vdev,
                keys.iter().map(|&(key, action)| {
                    Batch::of(&[Event::new(EV_KEY, linux_key_code(key), key_value(action))])
                }),
            ),
            InputKind::Tablet => false,
        }
    }

    /// An absolute position, for the tablet.
    pub fn move_to(&mut self, vdev: &mut Vdev<'_, '_>, pos: PointerPos) -> bool {
        match self.kind {
            InputKind::Tablet => {
                let batch = Batch::of(&[
                    Event::new(EV_ABS, ABS_X, u32::from(pos.x())),
                    Event::new(EV_ABS, ABS_Y, u32::from(pos.y())),
                ]);
                self.deliver(vdev, core::iter::once(batch))
            }
            InputKind::Keyboard => false,
        }
    }

    /// A button, for the tablet.
    pub fn button(
        &mut self,
        vdev: &mut Vdev<'_, '_>,
        button: PointerButton,
        action: KeyAction,
    ) -> bool {
        match self.kind {
            InputKind::Tablet => {
                let batch = Batch::of(&[Event::new(
                    EV_KEY,
                    linux_button_code(button),
                    key_value(action),
                )]);
                self.deliver(vdev, core::iter::once(batch))
            }
            InputKind::Keyboard => false,
        }
    }

    /// A turn of the wheel, for the tablet: one `REL_WHEEL` event of its
    /// detents, positive away from the user, and its sync. A turn of no
    /// detents moves nothing and sends nothing.
    pub fn wheel(&mut self, vdev: &mut Vdev<'_, '_>, steps: WheelSteps) -> bool {
        match self.kind {
            InputKind::Tablet => {
                // `struct virtio_input_event`'s `value` is evdev's signed
                // value in its 32 bits.
                let turn = (steps.0 != 0)
                    .then(|| Batch::of(&[Event::new(EV_REL, REL_WHEEL, steps.0 as u32)]));
                self.deliver(vdev, turn.into_iter())
            }
            InputKind::Keyboard => false,
        }
    }

    pub fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_u8(self.select)?;
        w.write_u8(self.subsel)
    }

    pub fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        self.select = r.read_u8()?;
        self.subsel = r.read_u8()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/virtio_input.rs"]
mod tests;
