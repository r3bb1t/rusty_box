//! A pointing device as every rusty_box machine sees it: where the pointer
//! is on the guest's screen, which buttons are down, and how far the wheel
//! turned. What that becomes on the wire is the machine's business — a
//! virtio tablet reports absolute positions, a PS/2 mouse relative motion.

/// A pointer button.
///
/// Closed (R5): the buttons a USB HID or virtio tablet reports (Linux
/// `BTN_LEFT` to `BTN_EXTRA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
    /// The thumb button usually bound to "back".
    Side,
    /// The thumb button usually bound to "forward".
    Extra,
}

impl PointerButton {
    pub const ALL: [PointerButton; 5] = [Self::Left, Self::Right, Self::Middle, Self::Side, Self::Extra];
}

/// A point on the guest's screen as a fraction of its width and height,
/// independent of its resolution: `0` is the left or top edge and
/// [`PointerPos::MAX`] the right or bottom one, as an absolute pointing
/// device reports it (QEMU `INPUT_EVENT_ABS_MAX`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointerPos {
    x: u16,
    y: u16,
}

impl PointerPos {
    /// The largest coordinate on either axis.
    pub const MAX: u16 = 0x7FFF;

    /// The point `x`, `y`, each clamped to [`PointerPos::MAX`].
    pub const fn new(x: u16, y: u16) -> Self {
        Self {
            x: if x > Self::MAX { Self::MAX } else { x },
            y: if y > Self::MAX { Self::MAX } else { y },
        }
    }

    /// Pixel (`px`, `py`) of a `width` by `height` picture, scaled the way
    /// QEMU's `qemu_input_scale_axis` scales a window position (input range
    /// `0..=size`, an empty one landing in the middle); past the edge
    /// clamps to it.
    pub fn from_pixel(px: u32, py: u32, width: u32, height: u32) -> Self {
        let scale = |p: u32, size: u32| -> u16 {
            if size == 0 {
                return Self::MAX / 2;
            }
            let scaled = u64::from(p) * u64::from(Self::MAX) / u64::from(size);
            scaled.min(u64::from(Self::MAX)) as u16
        };
        Self::new(scale(px, width), scale(py, height))
    }

    pub const fn x(self) -> u16 {
        self.x
    }

    pub const fn y(self) -> u16 {
        self.y
    }
}

/// How far a wheel turned: positive away from the user (scrolling up),
/// negative towards (down), one per detent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WheelSteps(pub i32);

/// The verbs a host drives a guest's pointing device with. Each reports
/// whether the event reached the guest, which happens whole or not at all;
/// a refusal means the guest has not taken what it was sent, and the answer
/// is to step the machine and send again.
pub trait PointerInput {
    /// Move the pointer to `pos`.
    #[must_use = "a refused move never reached the guest"]
    fn move_to(&mut self, pos: PointerPos) -> bool;

    /// Press or release `button`.
    #[must_use = "a refused button never reached the guest"]
    fn button(&mut self, button: PointerButton, action: super::KeyAction) -> bool;

    /// Turn the wheel.
    #[must_use = "a refused wheel turn never reached the guest"]
    fn wheel(&mut self, steps: WheelSteps) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top-left pixel is the origin, a picture's far edge the far edge
    /// of the coordinate space, the middle the middle, past the edge clamps,
    /// and an empty picture lands in the middle (QEMU
    /// `qemu_input_scale_axis`).
    #[test]
    fn pixels_scale_as_qemu_scales_them() {
        assert_eq!(PointerPos::from_pixel(0, 0, 1280, 800), PointerPos::new(0, 0));
        assert_eq!(PointerPos::from_pixel(1280, 800, 1280, 800), PointerPos::new(0x7FFF, 0x7FFF));
        assert_eq!(PointerPos::from_pixel(640, 400, 1280, 800), PointerPos::new(0x3FFF, 0x3FFF));
        assert_eq!(PointerPos::from_pixel(5000, 5000, 1280, 800), PointerPos::new(0x7FFF, 0x7FFF));
        assert_eq!(PointerPos::from_pixel(3, 3, 0, 0), PointerPos::new(0x3FFF, 0x3FFF));
        assert_eq!(PointerPos::new(0xFFFF, 1).x(), PointerPos::MAX);
    }
}
