//! Arm PrimeCell PL061 GPIO.
//!
//! Provenance (R7): the PrimeCell General Purpose Input/Output (PL061)
//! Technical Reference Manual, ARM DDI 0190B, first; QEMU 11.1.2
//! `hw/gpio/pl061.c` second.
//!
//! From DDI 0190B:
//!
//! - The register map, widths, reset values and identification registers
//!   (Table 3-1).
//! - `GPIODATA` (§2.3.3, §3.3.1): address bits `PADDR[9:2]` mask the bits a
//!   write changes and the bits a read returns, the rest reading as zero. A
//!   write changes the register whatever the lines' direction; what it
//!   holds reaches a line only while the line is an output. A read returns
//!   each line's level: what the register drives on an output, what the
//!   outside drives on an input.
//! - `GPIOAFSEL` (§2.3.5): a line under hardware control ignores `GPIODIR`
//!   and `GPIODATA`. This board wires nothing to the hardware control
//!   interface, so such a line is driven only from outside, as an input is.
//! - Interrupt detection (§2.2.2, §2.3.4, Figure 2-1) watches the lines'
//!   levels — the pads loop an output's level back to `GPIN` (Table 2-2) —
//!   so an output that changes level is an edge too. `GPIOIS` picks edge or
//!   level per line, `GPIOIBE` both edges, `GPIOIEV` the rising edge or high
//!   level when set. An edge is latched until a 1 is written to that bit of
//!   `GPIOIC`, which clears only "the interrupt edge detection logic
//!   register" (§3.3.9); a level-sensitive line's raw status is its level,
//!   which "the external source holds ... constant for the interrupt to be
//!   recognized" (§2.3.4). `GPIORIS` reads the raw status (§3.3.7), with no
//!   mask, and `GPIOIE` selects what reaches `GPIOMIS` and the combined
//!   `GPIOINTR` (§3.3.6, §3.3.8).
//! - Reset (§2.3.1, §2.3.2): every register clears and the lines become
//!   inputs; the levels the outside drives are not the PL061's to reset.
//! - The APB interface decodes `PADDR[11:2]` (Figure 2-1), so an access
//!   narrower than a word reaches the register that holds it; this board's
//!   bridge hands over the register value unshifted, as for the PL011.
//!
//! From QEMU where DDI 0190B leaves the choice to the system: on `virt`
//! only line 3 is wired, to the power button (`gpio-keys`, `KEY_POWER`);
//! the other lines read low. Reserved locations read as zero and ignore
//! writes. QEMU differs from DDI 0190B where README's "Where the references
//! differ" says.

use rusty_box_core::snap::{SnapRead, SnapResult, SnapWrite};

use crate::bus::{AccessWidth, RegOffset};

/// The registers' offsets in the region (DDI 0190B Table 3-1).
const GPIODATA_LAST: u64 = 0x3FC;
const GPIODIR: u64 = 0x400;
const GPIOIS: u64 = 0x404;
const GPIOIBE: u64 = 0x408;
const GPIOIEV: u64 = 0x40C;
const GPIOIE: u64 = 0x410;
const GPIORIS: u64 = 0x414;
const GPIOMIS: u64 = 0x418;
const GPIOIC: u64 = 0x41C;
const GPIOAFSEL: u64 = 0x420;
/// `GPIOPeriphID0` to `GPIOPCellID3`: the eight identification registers
/// that end the region, one byte of [`PL061_ID`] each.
const GPIOPERIPHID0: u64 = 0xFE0;
const GPIOPCELLID3: u64 = 0xFFC;

/// DDI 0190B Table 3-1.
const PL061_ID: [u8; 8] = [0x61, 0x10, 0x04, 0x00, 0x0D, 0xF0, 0x05, 0xB1];

/// The GPIO line QEMU's `virt` wires its power button to.
pub const POWER_BUTTON_LINE: u8 = 3;

/// The register an access at `offset` reaches: the interface decodes
/// `PADDR[11:2]` (DDI 0190B Figure 2-1).
const fn register_at(offset: RegOffset) -> u64 {
    offset.get() & 0xFFC
}

#[derive(Default)]
pub struct Pl061 {
    /// `GPIODATA`: what an output line drives.
    data: u8,
    /// The level the outside drives on each line.
    external: u8,
    /// The lines' levels when interrupt detection last looked, to find
    /// edges.
    old_levels: u8,
    dir: u8,
    isense: u8,
    ibe: u8,
    iev: u8,
    im: u8,
    /// The edge detection latches, which `GPIOIC` clears.
    edges: u8,
    afsel: u8,
}

impl Pl061 {
    pub fn new() -> Self {
        Self::default()
    }

    /// A system reset (DDI 0190B §2.3.2): every register clears; the lines
    /// become inputs at the levels the outside drives, which latch no edge.
    pub fn reset(&mut self) {
        *self = Self {
            external: self.external,
            old_levels: self.external,
            ..Self::default()
        };
    }

    /// The lines `GPIODATA` drives: outputs under software control.
    fn driven(&self) -> u8 {
        self.dir & !self.afsel
    }

    /// Each line's level.
    fn levels(&self) -> u8 {
        let driven = self.driven();
        (self.data & driven) | (self.external & !driven)
    }

    /// `GPIORIS`: latched edges on edge-sensitive lines, and the level on
    /// level-sensitive ones.
    fn raw_status(&self) -> u8 {
        let at_level = !(self.levels() ^ self.iev);
        (self.edges & !self.isense) | (at_level & self.isense)
    }

    pub fn irq(&self) -> bool {
        self.raw_status() & self.im != 0
    }

    /// Drive `line` to `level` from outside. An output line keeps the level
    /// it drives until the guest makes it an input.
    pub fn set_input(&mut self, line: u8, level: bool) {
        let mask = 1u8 << (line & 7);
        self.external = (self.external & !mask) | if level { mask } else { 0 };
        self.detect_edges();
    }

    /// Latch the edges since the last look on edge-sensitive lines: either
    /// edge with `GPIOIBE` set, else the one `GPIOIEV` names.
    fn detect_edges(&mut self) {
        let levels = self.levels();
        let changed = (levels ^ self.old_levels) & !self.isense;
        self.old_levels = levels;
        let wanted = self.ibe | !(levels ^ self.iev);
        self.edges |= changed & wanted;
    }

    pub fn read(&mut self, offset: RegOffset, _width: AccessWidth) -> u64 {
        let reg = register_at(offset);
        u64::from(match reg {
            0..=GPIODATA_LAST => self.levels() & (reg >> 2) as u8,
            GPIODIR => self.dir,
            GPIOIS => self.isense,
            GPIOIBE => self.ibe,
            GPIOIEV => self.iev,
            GPIOIE => self.im,
            GPIORIS => self.raw_status(),
            GPIOMIS => self.raw_status() & self.im,
            GPIOAFSEL => self.afsel,
            GPIOPERIPHID0..=GPIOPCELLID3 => PL061_ID[((reg - GPIOPERIPHID0) >> 2) as usize],
            _ => 0,
        })
    }

    pub fn write(&mut self, offset: RegOffset, _width: AccessWidth, value: u64) {
        let reg = register_at(offset);
        let v = value as u8;
        match reg {
            0..=GPIODATA_LAST => {
                let mask = (reg >> 2) as u8;
                self.data = (self.data & !mask) | (v & mask);
            }
            GPIODIR => self.dir = v,
            GPIOIS => self.isense = v,
            GPIOIBE => self.ibe = v,
            GPIOIEV => self.iev = v,
            GPIOIE => self.im = v,
            GPIOIC => self.edges &= !v,
            GPIOAFSEL => self.afsel = v,
            _ => {}
        }
        self.detect_edges();
    }

    pub(crate) fn save<W: SnapWrite>(&self, w: &mut W) -> SnapResult {
        w.write_bytes(&[
            self.data,
            self.external,
            self.old_levels,
            self.dir,
            self.isense,
            self.ibe,
            self.iev,
            self.im,
            self.edges,
            self.afsel,
        ])
    }

    pub(crate) fn restore<R: SnapRead>(&mut self, r: &mut R) -> SnapResult {
        let mut b = [0u8; 10];
        r.read_bytes(&mut b)?;
        let [data, external, old_levels, dir, isense, ibe, iev, im, edges, afsel] = b;
        *self = Self {
            data,
            external,
            old_levels,
            dir,
            isense,
            ibe,
            iev,
            im,
            edges,
            afsel,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: u8 = 1 << POWER_BUTTON_LINE;

    fn write(gpio: &mut Pl061, off: u64, v: u8) {
        gpio.write(RegOffset::new(off), AccessWidth::Word, u64::from(v));
    }

    fn read(gpio: &mut Pl061, off: u64) -> u8 {
        gpio.read(RegOffset::new(off), AccessWidth::Word) as u8
    }

    #[test]
    fn a_power_button_press_and_release_each_interrupt_on_both_edges() {
        let mut gpio = Pl061::new();
        // What Linux's gpio-pl061 programs for gpio-keys: both edges, unmasked.
        write(&mut gpio, GPIOIBE, LINE);
        write(&mut gpio, GPIOIE, LINE);
        assert!(!gpio.irq());
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert!(gpio.irq(), "the press is an edge");
        assert_eq!(read(&mut gpio, GPIODATA_LAST) & LINE, LINE);
        write(&mut gpio, GPIOIC, LINE);
        assert!(!gpio.irq());
        gpio.set_input(POWER_BUTTON_LINE, false);
        assert!(gpio.irq(), "so is the release");
    }

    /// DDI 0190B §3.3.1: a line made an output reads what `GPIODATA` drives,
    /// whatever the outside drives, and reads the outside again once it is
    /// an input.
    #[test]
    fn an_output_reads_what_it_drives_and_an_input_what_the_outside_drives() {
        let mut gpio = Pl061::new();
        gpio.set_input(POWER_BUTTON_LINE, true);
        write(&mut gpio, GPIODIR, LINE);
        write(&mut gpio, GPIODATA_LAST, 0);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), 0);
        write(&mut gpio, GPIODIR, 0);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), LINE);
    }

    /// DDI 0190B §3.3.1: a write changes `GPIODATA` whatever the direction;
    /// a line that becomes an output drives what was written while it was
    /// an input.
    #[test]
    fn a_write_to_an_input_reaches_the_line_once_it_is_an_output() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIODATA_LAST, 0x81);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), 0, "inputs read the outside");
        write(&mut gpio, GPIODIR, 0xFF);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), 0x81);
    }

    /// DDI 0190B §2.3.3: `PADDR[9:2]` masks a write and a read.
    #[test]
    fn the_address_masks_data_bits() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIODIR, 0xFF);
        // The manual's example: 0xFB at offset 0x098 sets bits 5 and 1 and
        // clears bit 2.
        write(&mut gpio, 0x098, 0xFB);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), 0b0010_0010);
        assert_eq!(read(&mut gpio, 0x080), 0b0010_0000, "only bit 5 is read");
    }

    /// The pads loop an output back to the detector: an output that rises is
    /// a rising edge.
    #[test]
    fn an_output_changing_level_is_an_edge() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIODIR, 0x01);
        write(&mut gpio, GPIOIEV, 0x01);
        write(&mut gpio, GPIOIE, 0x01);
        assert!(!gpio.irq());
        write(&mut gpio, 0x004, 0x01);
        assert!(gpio.irq());
        assert_eq!(read(&mut gpio, GPIORIS), 0x01);
    }

    /// DDI 0190B §2.3.4, §3.3.9: a level-sensitive line's raw status is its
    /// level — `GPIOIC` does not clear it while the level holds, and it
    /// clears when the level goes.
    #[test]
    fn a_level_interrupt_follows_the_line() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIOIS, LINE);
        write(&mut gpio, GPIOIEV, LINE);
        write(&mut gpio, GPIOIE, LINE);
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert!(gpio.irq());
        write(&mut gpio, GPIOIC, LINE);
        assert!(gpio.irq(), "still held high");
        gpio.set_input(POWER_BUTTON_LINE, false);
        assert!(!gpio.irq(), "the level went");
        assert_eq!(read(&mut gpio, GPIORIS), 0);
    }

    /// DDI 0190B §3.3.7: the raw status records an edge on a masked line.
    #[test]
    fn a_masked_edge_shows_in_the_raw_status_only() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIOIEV, LINE);
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert_eq!(read(&mut gpio, GPIORIS), LINE);
        assert_eq!(read(&mut gpio, GPIOMIS), 0);
        assert!(!gpio.irq());
    }

    /// DDI 0190B §2.3.5: a line under hardware control ignores `GPIODIR` and
    /// `GPIODATA`; nothing on this board drives it but the outside.
    #[test]
    fn a_line_under_hardware_control_reads_the_outside() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIODIR, LINE);
        write(&mut gpio, GPIODATA_LAST, LINE);
        write(&mut gpio, GPIOAFSEL, LINE);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), 0);
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), LINE);
    }

    /// A reset clears every register and keeps the levels the outside drives.
    #[test]
    fn a_reset_clears_the_registers_and_keeps_the_outside() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIOIBE, LINE);
        write(&mut gpio, GPIOIE, LINE);
        gpio.set_input(POWER_BUTTON_LINE, true);
        assert!(gpio.irq());
        gpio.reset();
        assert!(!gpio.irq());
        assert_eq!(read(&mut gpio, GPIORIS), 0);
        assert_eq!(read(&mut gpio, GPIOIE), 0);
        assert_eq!(read(&mut gpio, GPIODATA_LAST), LINE);
    }

    /// The interface decodes `PADDR[11:2]`.
    #[test]
    fn a_narrow_access_reaches_the_register_that_holds_it() {
        let mut gpio = Pl061::new();
        write(&mut gpio, GPIODIR, 0x5A);
        assert_eq!(gpio.read(RegOffset::new(0x401), AccessWidth::Byte), 0x5A);
        assert_eq!(gpio.read(RegOffset::new(0xFE2), AccessWidth::Byte), 0x61);
    }
}
