//! Arm PrimeCell PL061 GPIO.
//!
//! Provenance (R7): QEMU `hw/gpio/pl061.c` — register map and `pl061_id`.
//! On QEMU's `virt` only line 3 is wired, as the ACPI/DT power button input;
//! this board never presses it, so no input ever changes and the interrupt
//! state only moves through the guest's own register writes.

use crate::bus::{AccessWidth, RegOffset};

const PL061_ID: [u8; 12] = [0x00, 0x00, 0x00, 0x00, 0x61, 0x10, 0x04, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

#[derive(Default)]
pub struct Pl061 {
    data: u8,
    dir: u8,
    is: u8,
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

    pub fn read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        let off = offset.get();
        u64::from(match off {
            // GPIODATA: address bits [9:2] mask the bits read.
            0x000..=0x3FF => self.data & ((off >> 2) as u8),
            0x400 => self.dir,
            0x404 => self.is,
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
            0x404 => self.is = v,
            0x408 => self.ibe = v,
            0x40C => self.iev = v,
            0x410 => self.im = v,
            0x41C => self.istate &= !v,
            0x420 => self.afsel = v,
            _ => {}
        }
    }
}
