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
    Scancode(u8),
    /// A relative mouse movement / button / wheel update.
    Mouse(HostMouseEvent),
}

/// A destination for host input events.
///
/// Implemented by `SharedDisplay` (native: queues events for the emulator
/// thread) and by `Emulator` (wasm: applies them immediately). One egui
/// translator feeds either.
pub trait HostInputSink {
    fn push(&mut self, event: HostInputEvent);
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
        sink.push(HostInputEvent::Mouse(HostMouseEvent {
            dx,
            dy,
            dz,
            buttons,
        }));
    }
    buttons
}

/// Push a batch of scancodes into a sink.
pub fn push_scancodes(sink: &mut impl HostInputSink, scancodes: &[u8]) {
    for &sc in scancodes {
        sink.push(HostInputEvent::Scancode(sc));
    }
}

impl HostInputSink for super::shared_display::SharedDisplay {
    fn push(&mut self, event: HostInputEvent) {
        match event {
            HostInputEvent::Scancode(sc) => self.pending_scancodes.push(sc),
            HostInputEvent::Mouse(mouse) => self.pending_mouse.push(mouse),
        }
    }
}

impl<'a, I: crate::cpu::BxCpuIdTrait, T: crate::cpu::instrumentation::Instrumentation> HostInputSink
    for crate::emulator::Emulator<'a, I, T>
{
    fn push(&mut self, event: HostInputEvent) {
        match event {
            HostInputEvent::Scancode(sc) => self.send_scancode(sc),
            HostInputEvent::Mouse(mouse) => {
                self.send_mouse_event(mouse.dx, mouse.dy, mouse.dz, mouse.buttons);
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
    fn push(&mut self, event: HostInputEvent) {
        self.events.push(event);
    }
}
