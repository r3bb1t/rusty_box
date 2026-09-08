//! Portable host-input abstraction shared by the native (egui desktop) and
//! wasm (browser) frontends.
//!
//! The two targets deliver input by very different routes — native pushes into
//! an `Arc<Mutex<SharedDisplay>>` drained by the emulator thread, while wasm is
//! single-threaded and drives the `Emulator` directly — but both translate the
//! same egui events into [`HostInputEvent`]s and push them into a
//! [`HostInputSink`]. This keeps one translation path for keyboard and mouse
//! across targets. Deliberately alloc-friendly and NOT `Send + Sync`: a sink is
//! only ever touched from the frontend context.

use alloc::vec::Vec;

#[cfg(feature = "gui-egui")]
use super::keymap::char_to_bx_key_sequence;
#[cfg(feature = "gui-egui")]
use crate::iodev::scancodes::BxKey;

/// A relative PS/2 mouse update: signed movement deltas plus a button bitmask.
///
/// Buttons follow the PS/2 layout: bit 0 = left, bit 1 = right, bit 2 = middle.
/// `dz` is the scroll-wheel movement (IntelliMouse); it is ignored unless the
/// guest has negotiated IMPS2 wheel mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostMouseEvent {
    pub dx: i32,
    pub dy: i32,
    pub dz: i32,
    pub buttons: u8,
}

/// A single portable host input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostInputEvent {
    /// A PS/2 set-1 scancode byte (make/break; `0xE0`/`0xF0` prefixes inline).
    ///
    /// Prefer [`HostInputEvent::Key`]: raw bytes bypass the guest's selected
    /// scancode set. This variant remains for callers that genuinely have bytes
    /// (host bridges, the WASM console, tests).
    Scancode(u8),
    /// A guest key press (`true`) or release (`false`).
    ///
    /// This is the Bochs-shaped path: the front end reports *which key* and the
    /// keyboard controller renders it through `scancodes[key][set]`, so a guest
    /// that selects scancode set 1 or 3 gets the right bytes.
    Key(crate::iodev::scancodes::BxKey, bool),
    /// A relative mouse movement / button / wheel update.
    Mouse(HostMouseEvent),
}

/// A destination for host input events.
///
/// Implemented by `SharedDisplay` (native: queues events for the emulator
/// thread) and by `Emulator` (wasm: applies them immediately). One egui
/// translator feeds either.
pub trait HostInputSink {
    /// Returns whether the event was taken. A sink that queues for another
    /// thread reports whether its queue had room; a sink that applies events
    /// straight to a machine reports whether the guest's own 16-byte keyboard
    /// ring did. Either way a `false` means this event did NOT reach the guest,
    /// and a caller feeding a batch should stop rather than push the rest in
    /// after a hole.
    #[must_use]
    fn push(&mut self, event: HostInputEvent) -> bool;
}

/// Translate this frame's egui pointer state into at most one [`HostMouseEvent`]
/// and push it into `sink`. Returns the current button bitmask so the caller can
/// track it across frames (a button release with no motion still needs to be
/// reported once).
///
/// PS/2 mice are relative, so egui's per-frame pointer delta maps directly to
/// `dx`/`dy`. Screen Y grows downward while the PS/2 protocol treats up as
/// positive, so `dy` is negated. `prev_buttons` is the bitmask returned by the
/// previous call; an event is emitted only when something actually changed.
#[cfg(feature = "gui-egui")]
pub fn translate_egui_mouse(
    input: &egui::InputState,
    prev_buttons: u8,
    sink: &mut impl HostInputSink,
) -> u8 {
    let delta = input.pointer.delta();
    let dx = delta.x.round() as i32;
    // Screen down is positive; PS/2 up is positive.
    let dy = -(delta.y.round() as i32);
    // One wheel notch per frame in the scrolled direction; PS/2 wheel-up is
    // negative (Bochs create_mouse_packet negates delayed_dz).
    let dz = -(input.smooth_scroll_delta.y.signum() as i32);

    let mut buttons = 0u8;
    if input.pointer.button_down(egui::PointerButton::Primary) {
        buttons |= 0x01;
    }
    if input.pointer.button_down(egui::PointerButton::Secondary) {
        buttons |= 0x02;
    }
    if input.pointer.button_down(egui::PointerButton::Middle) {
        buttons |= 0x04;
    }

    if dx != 0 || dy != 0 || dz != 0 || buttons != prev_buttons {
        if !sink.push(HostInputEvent::Mouse(HostMouseEvent {
            dx,
            dy,
            dz,
            buttons,
        })) {
            // A dropped motion is self-correcting: PS/2 deltas are relative, so
            // the next frame's delta is measured from the same pointer position
            // and the guest simply sees one coarser movement. A dropped BUTTON
            // edge is not, so it is worth saying when it happens.
            tracing::debug!("host mouse event refused by the sink; motion coalesces into the next frame");
        }
    }
    buttons
}

/// The Shift, Ctrl and Alt keys the guest holds, carried across frames so that
/// each edge is forwarded once and a refused edge is retried.
///
/// egui reports these three through [`egui::Modifiers`] on every platform,
/// while their own `Key` events arrive only from native windows, so the edges
/// are read from `Modifiers` and those `Key` events are dropped (see
/// [`egui_key_to_bx_key`]). `Modifiers` does not say which side was pressed,
/// so a modifier is forwarded as its left-hand key.
#[cfg(feature = "gui-egui")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeldModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[cfg(feature = "gui-egui")]
impl HeldModifiers {
    /// The three forwarded modifiers out of egui's five: `command` is an
    /// alias of `ctrl` off macOS, and `mac_cmd` has no PS/2 key.
    fn from_egui(modifiers: egui::Modifiers) -> Self {
        Self {
            shift: modifiers.shift,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
        }
    }
}

/// One frame's delivery verdict. Once the sink refuses a key nothing more is
/// pushed that frame, per [`HostInputSink::push`]: a sequence with a hole in
/// it is worse for the guest than a short one.
#[cfg(feature = "gui-egui")]
#[derive(Default)]
struct KeyDelivery {
    refused: bool,
}

#[cfg(feature = "gui-egui")]
impl KeyDelivery {
    fn push(&mut self, sink: &mut impl HostInputSink, key: BxKey, pressed: bool) {
        if self.refused {
            return;
        }
        if !sink.push(HostInputEvent::Key(key, pressed)) {
            self.refused = true;
        }
    }

    fn tap(&mut self, sink: &mut impl HostInputSink, key: BxKey) {
        self.push(sink, key, true);
        self.push(sink, key, false);
    }

    /// Bring the modifiers the guest holds level with `target`, pressing and
    /// releasing what differs. `guest` records only what the sink took, so a
    /// refused edge is retried by the next reconciliation.
    fn reconcile(
        &mut self,
        sink: &mut impl HostInputSink,
        guest: &mut HeldModifiers,
        target: HeldModifiers,
    ) {
        self.edge(sink, &mut guest.shift, target.shift, BxKey::ShiftL);
        self.edge(sink, &mut guest.ctrl, target.ctrl, BxKey::CtrlL);
        self.edge(sink, &mut guest.alt, target.alt, BxKey::AltL);
    }

    fn edge(&mut self, sink: &mut impl HostInputSink, held: &mut bool, want: bool, key: BxKey) {
        if *held == want {
            return;
        }
        self.push(sink, key, want);
        if !self.refused {
            *held = want;
        }
    }
}

/// Translate this frame's egui keyboard events into guest key presses and
/// releases, push them into `sink`, and consume them so egui does not also
/// spend them on its own widgets (Tab moves focus; Space and Enter click).
/// Returns the modifiers the guest holds afterwards, for the next call.
///
/// A guest is sent KEYS, not characters, because its own keyboard layout
/// decides what a key means and the host's must not: a `Key` event is
/// forwarded as the physical key it names — the position on the keyboard, the
/// same under every host layout — which is what Bochs's GUIs forward (win32.cc
/// and sdl2.cc map host keycodes to `BX_KEY_*`). Where the platform reports no
/// physical key (the web) the logical key stands in.
///
/// Shift, Ctrl and Alt come from [`egui::Modifiers`]: before each `Key` event
/// the guest's held set is brought level with the modifiers that keystroke was
/// made under, and at the end of the frame with the frame's own, so a chord is
/// a chord in the guest even when its modifier changed within the frame.
///
/// egui folds Ctrl+C, Ctrl+X and Ctrl+V into `Copy`, `Cut` and `Paste` before
/// any widget sees them. A guest console is not a text editor, so each is
/// unfolded back into a tap of the letter that produced it, under the frame's
/// modifiers, which are the ones the chord was made under.
///
/// `Text` speaks only for a frame with no `Key` events at all: synthesized
/// input (an inspection harness, an IME commit) has no key behind it and is
/// spelled in ASCII, which [`char_to_bx_key_sequence`] renders with its own
/// Shift. In a frame that has keys, `Text` is the same keystroke seen again
/// through the host's layout, and is dropped.
///
/// Delivery stops at the sink's first refusal for the rest of the frame; the
/// frame's remaining keyboard events are still consumed, since they are this
/// frame's keystrokes whichever way they went.
#[cfg(feature = "gui-egui")]
pub fn translate_egui_keyboard(
    input: &mut egui::InputState,
    held: HeldModifiers,
    sink: &mut impl HostInputSink,
) -> HeldModifiers {
    let frame_end = HeldModifiers::from_egui(input.modifiers);
    let frame_has_keys = input
        .events
        .iter()
        .any(|event| matches!(event, egui::Event::Key { .. }));
    let mut delivery = KeyDelivery::default();
    let mut guest = held;

    input.events.retain(|event| match event {
        egui::Event::Key {
            key,
            physical_key,
            pressed,
            modifiers,
            ..
        } => {
            delivery.reconcile(sink, &mut guest, HeldModifiers::from_egui(*modifiers));
            if let Some(guest_key) = egui_key_to_bx_key(physical_key.unwrap_or(*key)) {
                delivery.push(sink, guest_key, *pressed);
            }
            false
        }
        egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
            if !frame_has_keys {
                for ch in text.chars() {
                    for (key, pressed) in char_to_bx_key_sequence(ch) {
                        delivery.push(sink, key, pressed);
                    }
                }
            }
            false
        }
        egui::Event::Copy => {
            delivery.reconcile(sink, &mut guest, frame_end);
            delivery.tap(sink, BxKey::C);
            false
        }
        egui::Event::Cut => {
            delivery.reconcile(sink, &mut guest, frame_end);
            delivery.tap(sink, BxKey::X);
            false
        }
        egui::Event::Paste(_) => {
            delivery.reconcile(sink, &mut guest, frame_end);
            delivery.tap(sink, BxKey::V);
            false
        }
        _ => true,
    });
    delivery.reconcile(sink, &mut guest, frame_end);
    if delivery.refused {
        tracing::debug!("host key event refused by the sink; the rest of the frame was not delivered");
    }
    guest
}

/// The guest key an egui key names, or `None` for a key the guest has no
/// equivalent for.
///
/// egui's shifted names (`Colon`, `Pipe`, `OpenCurlyBracket`, …) come from the
/// logical key, which the web reports; each maps to the unshifted key at that
/// position, the guest's Shift being sent separately. `Plus` is the numpad add
/// key, its one physical origin. egui folds the other numpad keys into the
/// main-block key of the same meaning before this table sees them (its
/// `key_from_key_code`), so the guest's `Kp*` keys other than `KpAdd` are
/// unreachable from egui. The Shift, Ctrl and Alt keys are `None` because
/// their edges arrive through [`egui::Modifiers`] on every platform (see
/// [`HeldModifiers`]); the Super keys are not in `Modifiers` off macOS, so
/// they are forwarded from their own events. `Copy`, `Cut` and `Paste` here
/// are dedicated keyboard keys, not the folded chords, and F13 upward — none
/// has a `BX_KEY_*`.
#[cfg(feature = "gui-egui")]
pub fn egui_key_to_bx_key(key: egui::Key) -> Option<BxKey> {
    use egui::Key;
    Some(match key {
        Key::ArrowDown => BxKey::Down,
        Key::ArrowLeft => BxKey::Left,
        Key::ArrowRight => BxKey::Right,
        Key::ArrowUp => BxKey::Up,
        Key::Escape => BxKey::Esc,
        Key::Tab => BxKey::Tab,
        Key::Backspace => BxKey::Backspace,
        Key::Enter => BxKey::Enter,
        Key::Space => BxKey::Space,
        Key::Insert => BxKey::Insert,
        Key::Delete => BxKey::Delete,
        Key::Home => BxKey::Home,
        Key::End => BxKey::End,
        Key::PageUp => BxKey::PageUp,
        Key::PageDown => BxKey::PageDown,
        Key::Colon | Key::Semicolon => BxKey::Semicolon,
        Key::Comma => BxKey::Comma,
        Key::Backslash | Key::Pipe => BxKey::Backslash,
        Key::Slash | Key::Questionmark => BxKey::Slash,
        Key::Exclamationmark => BxKey::K1,
        Key::OpenBracket | Key::OpenCurlyBracket => BxKey::LeftBracket,
        Key::CloseBracket | Key::CloseCurlyBracket => BxKey::RightBracket,
        Key::Backtick => BxKey::Grave,
        Key::Minus => BxKey::Minus,
        Key::Period => BxKey::Period,
        Key::Plus => BxKey::KpAdd,
        Key::Equals => BxKey::Equals,
        Key::Quote => BxKey::SingleQuote,
        Key::Num0 => BxKey::K0,
        Key::Num1 => BxKey::K1,
        Key::Num2 => BxKey::K2,
        Key::Num3 => BxKey::K3,
        Key::Num4 => BxKey::K4,
        Key::Num5 => BxKey::K5,
        Key::Num6 => BxKey::K6,
        Key::Num7 => BxKey::K7,
        Key::Num8 => BxKey::K8,
        Key::Num9 => BxKey::K9,
        Key::A => BxKey::A,
        Key::B => BxKey::B,
        Key::C => BxKey::C,
        Key::D => BxKey::D,
        Key::E => BxKey::E,
        Key::F => BxKey::F,
        Key::G => BxKey::G,
        Key::H => BxKey::H,
        Key::I => BxKey::I,
        Key::J => BxKey::J,
        Key::K => BxKey::K,
        Key::L => BxKey::L,
        Key::M => BxKey::M,
        Key::N => BxKey::N,
        Key::O => BxKey::O,
        Key::P => BxKey::P,
        Key::Q => BxKey::Q,
        Key::R => BxKey::R,
        Key::S => BxKey::S,
        Key::T => BxKey::T,
        Key::U => BxKey::U,
        Key::V => BxKey::V,
        Key::W => BxKey::W,
        Key::X => BxKey::X,
        Key::Y => BxKey::Y,
        Key::Z => BxKey::Z,
        Key::F1 => BxKey::F1,
        Key::F2 => BxKey::F2,
        Key::F3 => BxKey::F3,
        Key::F4 => BxKey::F4,
        Key::F5 => BxKey::F5,
        Key::F6 => BxKey::F6,
        Key::F7 => BxKey::F7,
        Key::F8 => BxKey::F8,
        Key::F9 => BxKey::F9,
        Key::F10 => BxKey::F10,
        Key::F11 => BxKey::F11,
        Key::F12 => BxKey::F12,
        Key::BrowserBack => BxKey::IntBack,
        Key::SuperLeft => BxKey::WinL,
        Key::SuperRight => BxKey::WinR,
        Key::IntlBackslash => BxKey::LeftBackslash,
        Key::Copy
        | Key::Cut
        | Key::Paste
        | Key::F13
        | Key::F14
        | Key::F15
        | Key::F16
        | Key::F17
        | Key::F18
        | Key::F19
        | Key::F20
        | Key::F21
        | Key::F22
        | Key::F23
        | Key::F24
        | Key::F25
        | Key::F26
        | Key::F27
        | Key::F28
        | Key::F29
        | Key::F30
        | Key::F31
        | Key::F32
        | Key::F33
        | Key::F34
        | Key::F35
        | Key::ShiftLeft
        | Key::ShiftRight
        | Key::ControlLeft
        | Key::ControlRight
        | Key::AltLeft
        | Key::AltRight => return None,
    })
}

/// Push a batch of scancodes into a sink, returning how many were taken.
///
/// Stops at the first refusal, so the count is the resume point:
/// `scancodes[accepted..]` is what still has to go. Pushing past a refusal
/// would leave the guest a sequence with a hole in the middle, which is worse
/// than a short one.
pub fn push_scancodes(sink: &mut impl HostInputSink, scancodes: &[u8]) -> usize {
    let mut accepted = 0;
    for &sc in scancodes {
        if !sink.push(HostInputEvent::Scancode(sc)) {
            break;
        }
        accepted += 1;
    }
    accepted
}

impl HostInputSink for super::shared_display::SharedDisplay {
    /// These queues grow on the host side and hand off to the emulator thread,
    /// so they cannot refuse an event; the guest's ring is what may fill, and
    /// that is reported when the queue is drained into the machine.
    fn push(&mut self, event: HostInputEvent) -> bool {
        match event {
            HostInputEvent::Scancode(sc) => self.pending_scancodes.push(sc),
            HostInputEvent::Key(key, pressed) => self.pending_keys.push((key, pressed)),
            HostInputEvent::Mouse(mouse) => self.pending_mouse.push(mouse),
        }
        true
    }
}

impl<'a, T: crate::cpu::instrumentation::Instrumentation> HostInputSink
    for crate::emulator::Emulator<T>
{
    fn push(&mut self, event: HostInputEvent) -> bool {
        match event {
            HostInputEvent::Scancode(sc) => self.keyboard().scancodes(&[sc]) == 1,
            HostInputEvent::Key(key, pressed) => self.keyboard().key(key, pressed),
            HostInputEvent::Mouse(mouse) => {
                self.mouse().motion(mouse.dx, mouse.dy, mouse.dz, mouse.buttons)
            }
        }
    }
}

/// A trivial [`HostInputSink`] that records events, for tests.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct RecordingSink {
    pub events: Vec<HostInputEvent>,
}

impl HostInputSink for RecordingSink {
    fn push(&mut self, event: HostInputEvent) -> bool {
        self.events.push(event);
        true
    }
}

#[cfg(all(test, feature = "gui-egui"))]
mod egui_keyboard_tests {
    use super::{translate_egui_keyboard, HeldModifiers, HostInputEvent, HostInputSink, RecordingSink};
    use crate::iodev::scancodes::BxKey;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;
    use egui::{Event, Key, Modifiers};

    fn frame(events: Vec<Event>, modifiers: Modifiers) -> egui::InputState {
        let mut input = egui::InputState::default();
        input.events = events;
        input.modifiers = modifiers;
        input
    }

    fn key(key: Key, physical_key: Option<Key>, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    fn keys(sink: &RecordingSink) -> Vec<(BxKey, bool)> {
        sink.events
            .iter()
            .filter_map(|event| match event {
                HostInputEvent::Key(key, pressed) => Some((*key, *pressed)),
                HostInputEvent::Scancode(_) | HostInputEvent::Mouse(_) => None,
            })
            .collect()
    }

    // A host whose layout is Cyrillic reports the R key as `Key::R` from its
    // position, alongside `Text("к")`. The guest, whose own layout decides
    // what R means, sees the key once and nothing of the character.
    #[test]
    fn a_letter_under_a_non_latin_host_layout_reaches_the_guest_as_its_key() {
        let mut sink = RecordingSink::default();
        let mut input = frame(
            vec![
                key(Key::R, Some(Key::R), true, Modifiers::NONE),
                Event::Text("к".to_string()),
            ],
            Modifiers::NONE,
        );

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(keys(&sink), [(BxKey::R, true)]);
    }

    // Space arrives as both a `Key` and a `Text(" ")` in one frame. It is one
    // keystroke, so the guest gets one press.
    #[test]
    fn a_key_with_its_text_in_the_same_frame_is_delivered_once() {
        let mut sink = RecordingSink::default();
        let mut input = frame(
            vec![
                key(Key::Space, Some(Key::Space), true, Modifiers::NONE),
                Event::Text(" ".to_string()),
            ],
            Modifiers::NONE,
        );

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(keys(&sink), [(BxKey::Space, true)]);
    }

    // An AZERTY host reports the top-left letter key as logical A at physical
    // Q. The guest is sent the position, as Bochs's GUIs send host keycodes.
    #[test]
    fn the_physical_key_is_forwarded_over_the_logical_one() {
        let mut sink = RecordingSink::default();
        let mut input = frame(
            vec![key(Key::A, Some(Key::Q), true, Modifiers::NONE)],
            Modifiers::NONE,
        );

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(keys(&sink), [(BxKey::Q, true)]);
    }

    // The web reports no physical key; the logical one stands in.
    #[test]
    fn without_a_physical_key_the_logical_key_is_forwarded() {
        let mut sink = RecordingSink::default();
        let mut input = frame(vec![key(Key::Enter, None, true, Modifiers::NONE)], Modifiers::NONE);

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(keys(&sink), [(BxKey::Enter, true)]);
    }

    // Text with no key behind it is spelled through the ASCII keymap, Shift
    // included, so synthesized input still types.
    #[test]
    fn text_alone_is_spelled_through_the_ascii_keymap() {
        let mut sink = RecordingSink::default();
        let mut input = frame(vec![Event::Text("Ab".to_string())], Modifiers::NONE);

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(
            keys(&sink),
            [
                (BxKey::ShiftL, true),
                (BxKey::A, true),
                (BxKey::A, false),
                (BxKey::ShiftL, false),
                (BxKey::B, true),
                (BxKey::B, false),
            ]
        );
    }

    // Shift is read from `Modifiers`: its press leads the key made under it,
    // and its release, seen at the end of a later frame, trails that key's
    // release, so a chord is a chord in the guest too.
    #[test]
    fn modifier_edges_bracket_the_frames_keys() {
        let mut sink = RecordingSink::default();
        let mut down = frame(
            vec![key(Key::R, Some(Key::R), true, Modifiers::SHIFT)],
            Modifiers::SHIFT,
        );
        let held = translate_egui_keyboard(&mut down, HeldModifiers::default(), &mut sink);
        assert!(held.shift);

        let mut up = frame(
            vec![key(Key::R, Some(Key::R), false, Modifiers::SHIFT)],
            Modifiers::NONE,
        );
        let held = translate_egui_keyboard(&mut up, held, &mut sink);
        assert!(!held.shift);

        assert_eq!(
            keys(&sink),
            [
                (BxKey::ShiftL, true),
                (BxKey::R, true),
                (BxKey::R, false),
                (BxKey::ShiftL, false),
            ]
        );
    }

    // A modifier tapped around a key inside one frame is gone from the frame's
    // own `Modifiers` by the end; the key event still records it, and the
    // guest still gets the chord.
    #[test]
    fn a_modifier_tapped_around_a_key_within_one_frame_still_brackets_it() {
        let mut sink = RecordingSink::default();
        let mut input = frame(
            vec![
                key(Key::R, Some(Key::R), true, Modifiers::SHIFT),
                key(Key::R, Some(Key::R), false, Modifiers::SHIFT),
            ],
            Modifiers::NONE,
        );

        let held = translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert!(!held.shift);
        assert_eq!(
            keys(&sink),
            [
                (BxKey::ShiftL, true),
                (BxKey::R, true),
                (BxKey::R, false),
                (BxKey::ShiftL, false),
            ]
        );
    }

    // The returned state is what the guest holds, not what the host does, so
    // an edge the sink refused is offered again next frame.
    #[test]
    fn a_refused_modifier_edge_is_retried_next_frame() {
        let mut full = FullAfter {
            room: 0,
            taken: Vec::new(),
        };
        let mut first = frame(Vec::new(), Modifiers::SHIFT);
        let held = translate_egui_keyboard(&mut first, HeldModifiers::default(), &mut full);
        assert!(!held.shift);
        assert!(full.taken.is_empty());

        let mut sink = RecordingSink::default();
        let mut second = frame(Vec::new(), Modifiers::SHIFT);
        let held = translate_egui_keyboard(&mut second, held, &mut sink);

        assert!(held.shift);
        assert_eq!(keys(&sink), [(BxKey::ShiftL, true)]);
    }

    // A native `Key::ShiftLeft` event and the `Modifiers` edge describe the
    // same press; only the edge is forwarded.
    #[test]
    fn a_modifiers_own_key_event_is_not_forwarded_twice() {
        let mut sink = RecordingSink::default();
        let mut input = frame(
            vec![key(Key::ShiftLeft, Some(Key::ShiftLeft), true, Modifiers::SHIFT)],
            Modifiers::SHIFT,
        );

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(keys(&sink), [(BxKey::ShiftL, true)]);
    }

    // egui folds Ctrl+C into `Copy` before any widget sees a key; the guest
    // gets the chord back as Ctrl held around a tap of C.
    #[test]
    fn a_folded_copy_chord_is_unfolded_into_its_letter() {
        let mut sink = RecordingSink::default();
        let mut input = frame(vec![Event::Copy], Modifiers::CTRL);

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(
            keys(&sink),
            [(BxKey::CtrlL, true), (BxKey::C, true), (BxKey::C, false)]
        );
    }

    // Keyboard events are spent on the guest; pointer events stay for egui.
    #[test]
    fn keyboard_events_are_consumed_and_the_rest_are_kept() {
        let mut sink = RecordingSink::default();
        let mut input = frame(
            vec![
                key(Key::Tab, Some(Key::Tab), true, Modifiers::NONE),
                Event::Text("\t".to_string()),
                Event::PointerGone,
            ],
            Modifiers::NONE,
        );

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(input.events, [Event::PointerGone]);
    }

    /// A sink with room for a fixed number of events.
    struct FullAfter {
        room: usize,
        taken: Vec<HostInputEvent>,
    }

    impl HostInputSink for FullAfter {
        fn push(&mut self, event: HostInputEvent) -> bool {
            if self.taken.len() == self.room {
                return false;
            }
            self.taken.push(event);
            true
        }
    }

    // After a refusal the rest of the frame is withheld rather than pushed in
    // after the hole, and is still consumed.
    #[test]
    fn delivery_stops_at_the_sinks_first_refusal() {
        let mut sink = FullAfter {
            room: 1,
            taken: Vec::new(),
        };
        let mut input = frame(
            vec![
                key(Key::A, Some(Key::A), true, Modifiers::NONE),
                key(Key::A, Some(Key::A), false, Modifiers::NONE),
                key(Key::B, Some(Key::B), true, Modifiers::NONE),
            ],
            Modifiers::NONE,
        );

        translate_egui_keyboard(&mut input, HeldModifiers::default(), &mut sink);

        assert_eq!(sink.taken, [HostInputEvent::Key(BxKey::A, true)]);
        assert!(input.events.is_empty());
    }
}
