//! Arm PrimeCell PL061 GPIO.
//!
//! Provenance (R7): QEMU `hw/gpio/pl061.c` — register map, `pl061_id`, and
//! the input path: `pl061_set_irq` drives an input line, and `pl061_update`
//! (run after every input change and every register write) latches edges
//! into the interrupt state as `GPIOIBE`/`GPIOIEV` select and re-asserts
//! level-sensitive lines while they hold their level. On QEMU's `virt` only
//! line 3 is wired, to the power button (`gpio-keys`, `KEY_POWER`).

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use crate::bus::{AccessWidth, RegOffset};

const PL061_ID: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, 0x61, 0x10, 0x04, 0x00, 0x0D, 0xF0, 0x05, 0xB1,
];

/// The GPIO line QEMU's `virt` wires its power button to.
pub const POWER_BUTTON_LINE: u8 = 3;

#[derive(Default)]
pub struct Pl061 {
    data: u8,
    /// The input levels the last update saw, to find edges.
    old_in_data: u8,
    dir: u8,
    isense: u8,
    ibe: u8,
    iev: u8,
    im: u8,
    istate: u8,
    afsel: u8,
}

impl Pl061 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn irq(&self) -> bool {
        self.istate & self.im != 0
    }

    /// QEMU `pl061_set_irq`: drive input `line` to `level`. A line the guest
    /// made an output ignores it.
    pub fn set_input(&mut self, line: u8, level: bool) {
        let mask = 1u8 << (line & 7);
        if self.dir & mask == 0 {
            self.data &= !mask;
            if level {
                self.data |= mask;
            }
            self.update();
        }
    }

    /// The input half of QEMU `pl061_update`.
    fn update(&mut self) {
        let changed = (self.old_in_data ^ self.data) & !self.dir;
        if changed != 0 {
            self.old_in_data = self.data;
            for line in 0..8 {
                let mask = 1u8 << line;
                if changed & mask != 0 && self.isense & mask == 0 {
                    if self.ibe & mask != 0 {
                        // Both edges interrupt.
                        self.istate |= mask;
                    } else {
                        // GPIOIEV picks the edge: rising when set.
                        self.istate |= !(self.data ^ self.iev) & mask;
                    }
                }
            }
        }
        // A level-sensitive line interrupts while it holds the level IEV names.
        self.istate |= !(self.data ^ self.iev) & self.isense;
    }

    pub fn read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        let off = offset.get();
        u64::from(match off {
            // GPIODATA: address bits [9:2] mask the bits read.
            0x000..=0x3FF => self.data & ((off >> 2) as u8),
            0x400 => self.dir,
            0x404 => self.isense,
            0x408 => self.ibe,
            0x40C => self.iev,
            0x410 => self.im,
            0x414 => self.istate,
            0x418 => self.istate & self.im,
            0x420 => self.afsel,
            0xFD0..=0xFFF => PL061_ID[((off - 0xFD0) >> 2) as usize % 12],
            _ => 0,
        })
    }

    pub fn write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64) {
        let off = offset.get();
        let v = value as u8;
        match off {
            0x000..=0x3FF => {
                // Only outputs, and only the bits the address selects, change.
                let mask = ((off >> 2) as u8) & self.dir;
                self.data = (self.data & !mask) | (v & mask);
            }
            0x400 => self.dir = v,
            0x404 => self.isense = v,
            0x408 => self.ibe = v,
            0x40C => self.iev = v,
            0x410 => self.im = v,
            0x41C => self.istate &= !v,
            0x420 => self.afsel = v,
            _ => {}
        }
        self.update();
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bytes(&[
            self.data,
            self.old_in_data,
            self.dir,
            self.isense,
            self.ibe,
            self.iev,
            self.im,
            self.istate,
            self.afsel,
        ])
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut b = [0u8; 9];
        r.read_bytes(&mut b)?;
        let [data, old_in_data, dir, isense, ibe, iev, im, istate, afsel] = b;
        *self = Self {
            data,
            old_in_data,
            dir,
            isense,
            ibe,
            iev,
            im,
            istate,
            afsel,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPIOIBE: u64 = 0x408;
    const GPIOIE: u64 = 0x410;
    const GPIOIC: u64 = 0x41C;

    fn write(gpio: &mut Pl061, off: u64, v: u8) {
        gpio.write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    #[test]
    fn a_power_button_press_and_release_each_interrupt_on_both_edges() {
        let mut gpio = Pl061::new();
        let line = 1u8 << POWER_BUTTON_LINE;
        // What Linux's gpio-pl061 programs for gpio-keys: both edges, unmasked.
        write(&mut gpio, GPIOIBE, line);
        write(&mut gpio, GPIOIE, line);
        assert!(!gpio.irq());
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert!(gpio.irq(), "the press is an edge");
        assert_eq!(
            gpio.read(RegOffset::new(0x3FC), AccessWidth::Word) as u8 & line,
            line
        );
        write(&mut gpio, GPIOIC, line);
        assert!(!gpio.irq());
        gpio.set_input(POWER_BUTTON_LINE, false);
        assert!(gpio.irq(), "so is the release");
    }

    #[test]
    fn an_output_line_ignores_its_input() {
        let mut gpio = Pl061::new();
        write(&mut gpio, 0x400, 1 << POWER_BUTTON_LINE);
        write(&mut gpio, GPIOIE, 0xFF);
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert!(!gpio.irq());
    }
}
