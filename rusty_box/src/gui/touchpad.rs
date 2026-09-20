//! Touch as a trackpad for the guest's relative PS/2 mouse.
//!
//! egui reports a finger as the primary pointer button held down, so a finger
//! that slides to move the cursor would drag with the left button held, and a
//! touch screen would have no right button at all. [`Touchpad`] reads the raw
//! touches instead and turns them into what a laptop touchpad sends.

use super::host_input::{HostInputEvent, HostInputSink, HostMouseEvent};
use alloc::vec::Vec;

/// Where a touch began, which decides what the touch does for its whole life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchZone {
    /// The guest's image: the trackpad itself.
    Pad,
    /// The on-screen left button.
    LeftButton,
    /// The on-screen right button.
    RightButton,
    /// Anything else, such as a menu drawn over the image: not the guest's.
    Elsewhere,
}

/// One touch event as the platform delivered it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchSample {
    /// The finger, stable from touch-down to lift.
    pub id: u64,
    pub phase: egui::TouchPhase,
    /// Where the finger is, points.
    pub pos: egui::Pos2,
    /// When, seconds on the input clock.
    pub time: f64,
    /// What lies under `pos`; read at the touch's start only.
    pub zone: TouchZone,
}

/// A finger that lifts within this long of touching down, having moved less
/// than [`TAP_SLOP_POINTS`], taps.
const TAP_MAX_SECONDS: f64 = 0.25;
/// Two fingers that are both lifted within this long of the first touching
/// down, neither having moved the slop, tap with the right button.
const TWO_FINGER_TAP_MAX_SECONDS: f64 = 0.35;
/// A touch that starts within this long of a tap's lift holds the left
/// button: lifted at once it is a double-click's second click, slid it drags.
const DOUBLE_TAP_WINDOW_SECONDS: f64 = 0.30;
/// How far a finger may wander, points, while still tapping rather than
/// moving the cursor.
const TAP_SLOP_POINTS: f32 = 10.0;
/// Two fingers' mean vertical travel, points, per wheel notch.
const WHEEL_STEP_POINTS: f32 = 24.0;

/// PS/2 button bits.
const LEFT: u8 = 0x01;
const RIGHT: u8 = 0x02;

/// Turns touches into a touchpad's mouse events.
#[derive(Debug)]
pub struct Touchpad {
    fingers: Vec<Finger>,
    gesture: Gesture,
    /// When the last tap lifted, while no other gesture has happened since.
    last_tap: Option<f64>,
    /// Guest pixels one point of finger travel moves the cursor.
    scale: f32,
    /// Motion, guest pixels, too small to have been sent yet.
    carry: egui::Vec2,
    /// Two fingers' mean vertical travel, points, not yet sent as a notch.
    wheel: f32,
}

/// A finger on the screen and what it has done since it touched down.
#[derive(Clone, Copy, Debug)]
struct Finger {
    id: u64,
    zone: TouchZone,
    start: egui::Pos2,
    last: egui::Pos2,
    start_time: f64,
    /// It has left the tap slop at least once.
    moved: bool,
}

/// What the fingers on the pad are doing together.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    /// No finger on the pad.
    Idle,
    /// One finger, which moves the cursor. `drag` holds the left button: the
    /// finger touched down soon after a tap.
    One { drag: bool },
    /// Two fingers or more: a right-button tap or the wheel, never motion.
    /// `started` is when the first of them touched down.
    Two { started: f64, moved: bool },
}

/// How a touch ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lift {
    /// The finger was lifted.
    Released,
    /// The platform took the touch away; nothing it began may click.
    Cancelled,
}

impl Default for Touchpad {
    fn default() -> Self {
        Self::new()
    }
}

impl Touchpad {
    pub fn new() -> Self {
        Self {
            fingers: Vec::new(),
            gesture: Gesture::Idle,
            last_tap: None,
            scale: 1.0,
            carry: egui::Vec2::ZERO,
            wheel: 0.0,
        }
    }

    /// Guest pixels one point of finger travel moves the cursor.
    pub fn set_scale(&mut self, guest_pixels_per_point: f32) {
        self.scale = guest_pixels_per_point;
    }

    /// The PS/2 button mask the trackpad holds now: bit 0 left, bit 1 right.
    pub fn held_buttons(&self) -> u8 {
        self.buttons()
    }

    /// Takes one touch event and pushes what the guest should see into `sink`.
    pub fn touch<S: HostInputSink>(&mut self, sample: TouchSample, sink: &mut S) {
        match sample.phase {
            egui::TouchPhase::Start => self.touched_down(sample, sink),
            egui::TouchPhase::Move => self.moved(sample, sink),
            egui::TouchPhase::End => self.lifted(sample, Lift::Released, sink),
            egui::TouchPhase::Cancel => self.lifted(sample, Lift::Cancelled, sink),
        }
    }

    /// The buttons the guest holds: the on-screen buttons under a finger, and
    /// the left button a drag holds.
    fn buttons(&self) -> u8 {
        let on_screen = self.fingers.iter().fold(0, |held, finger| match finger.zone {
            TouchZone::LeftButton => held | LEFT,
            TouchZone::RightButton => held | RIGHT,
            TouchZone::Pad | TouchZone::Elsewhere => held,
        });
        let dragging = match self.gesture {
            Gesture::One { drag: true } => LEFT,
            Gesture::One { drag: false } | Gesture::Idle | Gesture::Two { .. } => 0,
        };
        on_screen | dragging
    }

    fn pad_fingers(&self) -> usize {
        self.fingers
            .iter()
            .filter(|finger| finger.zone == TouchZone::Pad)
            .count()
    }

    fn touched_down<S: HostInputSink>(&mut self, sample: TouchSample, sink: &mut S) {
        self.fingers.push(Finger {
            id: sample.id,
            zone: sample.zone,
            start: sample.pos,
            last: sample.pos,
            start_time: sample.time,
            moved: false,
        });
        match sample.zone {
            TouchZone::LeftButton | TouchZone::RightButton => self.send(0, 0, 0, sink),
            TouchZone::Elsewhere => {}
            TouchZone::Pad => match self.gesture {
                Gesture::Idle => {
                    let drag = self
                        .last_tap
                        .is_some_and(|tapped| sample.time - tapped <= DOUBLE_TAP_WINDOW_SECONDS);
                    self.last_tap = None;
                    self.gesture = Gesture::One { drag };
                    if drag {
                        self.send(0, 0, 0, sink);
                    }
                }
                Gesture::One { drag } => {
                    let started = self
                        .fingers
                        .iter()
                        .filter(|finger| finger.zone == TouchZone::Pad)
                        .map(|finger| finger.start_time)
                        .fold(sample.time, f64::min);
                    self.gesture = Gesture::Two {
                        started,
                        moved: false,
                    };
                    self.carry = egui::Vec2::ZERO;
                    self.wheel = 0.0;
                    if drag {
                        // The second finger ends the drag: let the button go.
                        self.send(0, 0, 0, sink);
                    }
                }
                Gesture::Two { .. } => {}
            },
        }
    }

    fn moved<S: HostInputSink>(&mut self, sample: TouchSample, sink: &mut S) {
        let Some(finger) = self.fingers.iter_mut().find(|finger| finger.id == sample.id) else {
            return;
        };
        let previous = finger.last;
        finger.last = sample.pos;
        match finger.zone {
            TouchZone::Pad => {}
            TouchZone::LeftButton | TouchZone::RightButton | TouchZone::Elsewhere => return,
        }
        let was_moved = finger.moved;
        if (sample.pos - finger.start).length() >= TAP_SLOP_POINTS {
            finger.moved = true;
        }
        let moved = finger.moved;
        // The move that leaves the slop sends the whole travel since the
        // touch-down, so a slide loses nothing to the tap check.
        let from = if was_moved { previous } else { finger.start };
        match self.gesture {
            Gesture::One { .. } => {
                if moved {
                    self.move_cursor(sample.pos - from, sink);
                }
            }
            Gesture::Two { started, moved: pair_moved } => {
                self.gesture = Gesture::Two {
                    started,
                    moved: pair_moved || moved,
                };
                let fingers = self.pad_fingers().max(1) as f32;
                self.wheel += (sample.pos.y - previous.y) / fingers;
                while self.wheel.abs() >= WHEEL_STEP_POINTS {
                    let notch = self.wheel.signum();
                    self.wheel -= notch * WHEEL_STEP_POINTS;
                    // Fingers moving down scroll the page up, and PS/2
                    // wheel-up is negative (Bochs create_mouse_packet
                    // negates dz).
                    self.send(0, 0, -(notch as i32), sink);
                }
            }
            Gesture::Idle => {}
        }
    }

    fn lifted<S: HostInputSink>(&mut self, sample: TouchSample, lift: Lift, sink: &mut S) {
        let Some(index) = self.fingers.iter().position(|finger| finger.id == sample.id) else {
            return;
        };
        let finger = self.fingers.remove(index);
        match finger.zone {
            TouchZone::LeftButton | TouchZone::RightButton => self.send(0, 0, 0, sink),
            TouchZone::Elsewhere => {}
            TouchZone::Pad => match self.gesture {
                Gesture::One { drag } => {
                    self.gesture = Gesture::Idle;
                    self.carry = egui::Vec2::ZERO;
                    let tapped = lift == Lift::Released
                        && !finger.moved
                        && sample.time - finger.start_time <= TAP_MAX_SECONDS;
                    if drag {
                        self.send(0, 0, 0, sink);
                    } else if tapped {
                        self.click(LEFT, sink);
                        self.last_tap = Some(sample.time);
                    }
                }
                Gesture::Two { started, moved } => {
                    let moved = moved || finger.moved;
                    if self.pad_fingers() > 0 {
                        self.gesture = Gesture::Two { started, moved };
                        return;
                    }
                    self.gesture = Gesture::Idle;
                    self.wheel = 0.0;
                    let tapped = lift == Lift::Released
                        && !moved
                        && sample.time - started <= TWO_FINGER_TAP_MAX_SECONDS;
                    if tapped {
                        self.click(RIGHT, sink);
                    }
                }
                Gesture::Idle => {}
            },
        }
    }

    /// Moves the cursor by a finger's travel, `delta` points, keeping the
    /// fractions of a guest pixel for the next move.
    fn move_cursor<S: HostInputSink>(&mut self, delta: egui::Vec2, sink: &mut S) {
        self.carry += delta * self.scale;
        let whole = egui::vec2(self.carry.x.trunc(), self.carry.y.trunc());
        self.carry -= whole;
        if whole != egui::Vec2::ZERO {
            // Screen down is positive; PS/2 up is positive.
            self.send(whole.x as i32, -(whole.y as i32), 0, sink);
        }
    }

    /// Presses `button` and lets it go.
    fn click<S: HostInputSink>(&mut self, button: u8, sink: &mut S) {
        let held = self.buttons();
        self.push(
            HostMouseEvent {
                dx: 0,
                dy: 0,
                dz: 0,
                buttons: held | button,
            },
            sink,
        );
        self.send(0, 0, 0, sink);
    }

    /// Sends motion and wheel with the buttons held now.
    fn send<S: HostInputSink>(&mut self, dx: i32, dy: i32, dz: i32, sink: &mut S) {
        let buttons = self.buttons();
        self.push(HostMouseEvent { dx, dy, dz, buttons }, sink);
    }

    fn push<S: HostInputSink>(&mut self, event: HostMouseEvent, sink: &mut S) {
        if !sink.push(HostInputEvent::Mouse(event)) {
            tracing::debug!("touchpad mouse event refused by the sink: {event:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, TouchPhase};

    /// Every mouse event pushed, in order.
    #[derive(Default)]
    struct Recorder(Vec<HostMouseEvent>);

    impl HostInputSink for Recorder {
        fn push(&mut self, event: HostInputEvent) -> bool {
            if let HostInputEvent::Mouse(mouse) = event {
                self.0.push(mouse);
            }
            true
        }
    }

    fn mouse(dx: i32, dy: i32, dz: i32, buttons: u8) -> HostMouseEvent {
        HostMouseEvent { dx, dy, dz, buttons }
    }

    /// A touchpad at one guest pixel per point, fed through a recorder.
    struct Rig {
        pad: Touchpad,
        seen: Recorder,
    }

    impl Rig {
        fn new() -> Self {
            let mut pad = Touchpad::new();
            pad.set_scale(1.0);
            Self {
                pad,
                seen: Recorder::default(),
            }
        }

        fn at(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, time: f64, zone: TouchZone) {
            let sample = TouchSample {
                id,
                phase,
                pos: pos2(x, y),
                time,
                zone,
            };
            self.pad.touch(sample, &mut self.seen);
        }

        fn pad(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32, time: f64) {
            self.at(id, phase, x, y, time, TouchZone::Pad);
        }

        fn seen(&self) -> &[HostMouseEvent] {
            &self.seen.0
        }
    }

    #[test]
    fn a_sliding_finger_moves_the_cursor_with_no_button() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::Move, 130.0, 90.0, 0.1);
        rig.pad(1, TouchPhase::Move, 140.0, 95.0, 0.2);
        rig.pad(1, TouchPhase::End, 140.0, 95.0, 0.4);

        // The first move leaves the tap slop, and its whole travel from the
        // touch-down is sent; screen-down is PS/2 negative.
        assert_eq!(rig.seen(), &[mouse(30, 10, 0, 0), mouse(10, -5, 0, 0)]);
    }

    #[test]
    fn a_tap_is_a_left_click() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::Move, 103.0, 101.0, 0.05);
        rig.pad(1, TouchPhase::End, 103.0, 101.0, 0.1);

        assert_eq!(rig.seen(), &[mouse(0, 0, 0, 0x01), mouse(0, 0, 0, 0)]);
    }

    #[test]
    fn a_long_press_that_never_moves_clicks_nothing() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::End, 100.0, 100.0, 1.0);

        assert!(rig.seen().is_empty(), "{:?}", rig.seen());
    }

    #[test]
    fn a_touch_soon_after_a_tap_holds_the_left_button_and_drags() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::End, 100.0, 100.0, 0.1);
        rig.pad(2, TouchPhase::Start, 100.0, 100.0, 0.25);
        rig.pad(2, TouchPhase::Move, 150.0, 100.0, 0.4);
        rig.pad(2, TouchPhase::End, 150.0, 100.0, 0.6);

        assert_eq!(
            rig.seen(),
            &[
                mouse(0, 0, 0, 0x01), // the tap's click
                mouse(0, 0, 0, 0),
                mouse(0, 0, 0, 0x01), // held from the second touch-down
                mouse(50, 0, 0, 0x01),
                mouse(0, 0, 0, 0), // let go at the lift
            ]
        );
    }

    #[test]
    fn two_quick_taps_are_a_double_click() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::End, 100.0, 100.0, 0.1);
        rig.pad(2, TouchPhase::Start, 100.0, 100.0, 0.2);
        rig.pad(2, TouchPhase::End, 100.0, 100.0, 0.3);

        assert_eq!(
            rig.seen(),
            &[
                mouse(0, 0, 0, 0x01),
                mouse(0, 0, 0, 0),
                mouse(0, 0, 0, 0x01),
                mouse(0, 0, 0, 0),
            ]
        );
    }

    #[test]
    fn a_touch_long_after_a_tap_only_moves() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::End, 100.0, 100.0, 0.1);
        rig.pad(2, TouchPhase::Start, 100.0, 100.0, 1.0);
        rig.pad(2, TouchPhase::Move, 120.0, 100.0, 1.1);
        rig.pad(2, TouchPhase::End, 120.0, 100.0, 1.2);

        assert_eq!(
            rig.seen(),
            &[mouse(0, 0, 0, 0x01), mouse(0, 0, 0, 0), mouse(20, 0, 0, 0)]
        );
    }

    #[test]
    fn a_two_finger_tap_is_a_right_click() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(2, TouchPhase::Start, 140.0, 100.0, 0.02);
        rig.pad(1, TouchPhase::End, 100.0, 100.0, 0.15);
        rig.pad(2, TouchPhase::End, 140.0, 100.0, 0.16);

        assert_eq!(rig.seen(), &[mouse(0, 0, 0, 0x02), mouse(0, 0, 0, 0)]);
    }

    #[test]
    fn two_fingers_sliding_down_scroll_the_wheel_up_without_moving_the_cursor() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(2, TouchPhase::Start, 140.0, 100.0, 0.0);
        // Both fingers travel 50 points down: a mean of 50, two notches.
        rig.pad(1, TouchPhase::Move, 100.0, 150.0, 0.1);
        rig.pad(2, TouchPhase::Move, 140.0, 150.0, 0.1);
        rig.pad(1, TouchPhase::End, 100.0, 150.0, 0.3);
        rig.pad(2, TouchPhase::End, 140.0, 150.0, 0.3);

        // PS/2 wheel-up is negative (Bochs create_mouse_packet negates dz).
        assert_eq!(rig.seen(), &[mouse(0, 0, -1, 0), mouse(0, 0, -1, 0)]);
    }

    #[test]
    fn holding_the_left_button_while_another_finger_slides_drags() {
        let mut rig = Rig::new();
        rig.at(1, TouchPhase::Start, 600.0, 400.0, 0.0, TouchZone::LeftButton);
        rig.pad(2, TouchPhase::Start, 100.0, 100.0, 0.1);
        rig.pad(2, TouchPhase::Move, 100.0, 140.0, 0.2);
        rig.pad(2, TouchPhase::End, 100.0, 140.0, 0.3);
        rig.at(1, TouchPhase::End, 600.0, 400.0, 0.5, TouchZone::LeftButton);

        assert_eq!(
            rig.seen(),
            &[mouse(0, 0, 0, 0x01), mouse(0, -40, 0, 0x01), mouse(0, 0, 0, 0)]
        );
    }

    #[test]
    fn the_right_button_presses_and_releases_with_its_touch() {
        let mut rig = Rig::new();
        rig.at(1, TouchPhase::Start, 650.0, 400.0, 0.0, TouchZone::RightButton);
        rig.at(1, TouchPhase::End, 650.0, 400.0, 0.1, TouchZone::RightButton);

        assert_eq!(rig.seen(), &[mouse(0, 0, 0, 0x02), mouse(0, 0, 0, 0)]);
    }

    #[test]
    fn a_touch_that_starts_elsewhere_never_reaches_the_guest() {
        let mut rig = Rig::new();
        rig.at(1, TouchPhase::Start, 10.0, 10.0, 0.0, TouchZone::Elsewhere);
        rig.at(1, TouchPhase::Move, 200.0, 200.0, 0.1, TouchZone::Pad);
        rig.at(1, TouchPhase::End, 200.0, 200.0, 0.2, TouchZone::Pad);

        assert!(rig.seen().is_empty(), "{:?}", rig.seen());
    }

    #[test]
    fn a_cancelled_touch_releases_a_drag_and_never_clicks() {
        let mut rig = Rig::new();
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::Cancel, 100.0, 100.0, 0.1);
        rig.pad(2, TouchPhase::Start, 100.0, 100.0, 0.15);
        rig.pad(2, TouchPhase::Move, 130.0, 100.0, 0.2);
        rig.pad(2, TouchPhase::Cancel, 130.0, 100.0, 0.3);

        assert_eq!(rig.seen(), &[mouse(30, 0, 0, 0)]);
    }

    #[test]
    fn the_scale_turns_points_into_guest_pixels_and_keeps_the_fractions() {
        let mut rig = Rig::new();
        rig.pad.set_scale(1.5);
        rig.pad(1, TouchPhase::Start, 100.0, 100.0, 0.0);
        rig.pad(1, TouchPhase::Move, 111.0, 100.0, 0.1); // 16.5 px: 16 now
        rig.pad(1, TouchPhase::Move, 112.0, 100.0, 0.2); // 1.5 + 0.5 kept: 2
        rig.pad(1, TouchPhase::End, 112.0, 100.0, 0.3);

        assert_eq!(rig.seen(), &[mouse(16, 0, 0, 0), mouse(2, 0, 0, 0)]);
    }
}
