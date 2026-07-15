//! DDC (Display Data Channel) monitor emulation — Bochs iodev/display/ddc.cc.
//!
//! Provides the I2C bit-bang interface the Bochs VGABIOS uses to read the
//! monitor's EDID block through VBE_DISPI register 0xB (vga.cc
//! VBE_DISPI_INDEX_DDC). This is a full port of Bochs's default
//! `vga: ddc_mode=builtin` behavior (config.cc BX_DDC_MODE_BUILTIN, the
//! default): the built-in 1920x1200 "Bochs Screen" EDID with a computed
//! checksum. Bochs's alternative modes (`disabled`, `builtin_gui` — EDID
//! adapted to GUI capabilities — and `file`) are bochsrc configuration
//! surface that rusty_box does not expose; guests see the identical
//! default-configuration behavior.

/// Built-in 128-byte VESA EDID block — Bochs ddc.cc vesa_EDID.
/// 1920x1200 preferred timing, "Bochs Screen" product name. The checksum
/// byte (offset 127) is computed in `BxDdcC::new` exactly like Bochs
/// ddc.cc init().
const VESA_EDID: [u8; 128] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, // 8-byte header
    0x04, 0x21, // Vendor ID ("AAA")
    0xAB, 0xCD, // Product ID
    0x00, 0x00, 0x00, 0x00, // Serial number (none)
    12, 11, // Week (12) and year (2001) of manufacture
    0x01, 0x03, // EDID version 1.3
    0x0F, // Video signal interface (analogue)
    0x21, 0x19, // Screen size (330 mm x 250 mm)
    0x78, // Display gamma (2.2)
    0x0F, // Feature flags
    0x78, 0xF5, // Chromaticity LSBs
    0xA6, 0x55, 0x48, 0x9B, 0x26, 0x12, 0x50, 0x54, // Chromaticity MSBs
    0xFF, // Established timings 1
    0xEF, // Established timings 2
    0x80, // Established timings 3 (1152x870@75)
    0x31, 0x59, // Standard timing #1 (640x480@85)
    0x45, 0x59, // Standard timing #2 (800x600@85)
    0x61, 0x59, // Standard timing #3 (1024x768@85)
    0x81, 0xCA, // Standard timing #4 (1280x720@70)
    0x81, 0x0A, // Standard timing #5 (1280x800@70)
    0xA9, 0xC0, // Standard timing #6 (1600x900@60)
    0xA9, 0x40, // Standard timing #7 (1600x1200@60)
    0xD1, 0x00, // Standard timing #8 (1920x1080@60)
    // First 18-byte descriptor (1920x1200, pixel clock 154 MHz)
    0x28, 0x3C, 0x80, 0xA0, 0x70, 0xB0, 0x23, 0x40, 0x30, 0x20, 0x36, 0x00, 0x06, 0x44, 0x21, 0x00,
    0x00, 0x1E, // Second 18-byte descriptor — display product serial number
    0x00, 0x00, 0x00, 0xFF, 0x00, b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', 0x0A,
    0x20, 0x20, // Third 18-byte descriptor — display product name
    0x00, 0x00, 0x00, 0xFC, 0x00, b'B', b'o', b'c', b'h', b's', b' ', b'S', b'c', b'r', b'e', b'e',
    b'n', 0x0A, // Fourth 18-byte descriptor (unused)
    0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, // Extension block count (none)
    0x00, // Checksum (computed in new())
];

/// I2C protocol stage — Bochs ddc.cc DDC_STAGE_* enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DdcStage {
    Start,
    Address,
    Rw,
    DataIn,
    DataOut,
    AckIn,
    AckOut,
    Stop,
}

/// DDC monitor with built-in EDID — Bochs ddc.cc bx_ddc_c.
///
/// The internal I2C state is deliberately not part of any snapshot: Bochs
/// ddc.cc registers no save-state either (only the VGA's `vbe.ddc_enabled`
/// flag is persisted, vga.cc register_state).
#[derive(Debug, Clone)]
pub(crate) struct BxDdcC {
    /// Host-driven clock line (DCK) — Bochs ddc.cc s.DCKhost.
    dck_host: bool,
    /// Host-driven data line (DDA) — Bochs ddc.cc s.DDAhost.
    dda_host: bool,
    /// Monitor-driven data line — Bochs ddc.cc s.DDAmon.
    dda_mon: bool,
    stage: DdcStage,
    bitshift: u8,
    /// ACK/NAK to report (false = ACK) — Bochs ddc.cc s.ddc_ack.
    ack: bool,
    /// Transfer direction from the address byte (true = read).
    rw: bool,
    byte: u8,
    edid_index: u8,
    edid_data: [u8; 128],
}

impl Default for BxDdcC {
    fn default() -> Self {
        Self::new()
    }
}

impl BxDdcC {
    /// Power-on state — Bochs ddc.cc bx_ddc_c::init for BX_DDC_MODE_BUILTIN.
    pub(crate) fn new() -> Self {
        let mut edid_data = VESA_EDID;
        // Bochs ddc.cc init: zero the checksum byte, then store the value
        // that makes the 128-byte block sum to 0.
        edid_data[127] = 0;
        let checksum = edid_data
            .iter()
            .fold(0u8, |sum, &byte| sum.wrapping_add(byte));
        if checksum != 0 {
            edid_data[127] = checksum.wrapping_neg();
        }
        Self {
            dck_host: true,
            dda_host: true,
            dda_mon: true,
            stage: DdcStage::Stop,
            bitshift: 0,
            ack: true,
            rw: true,
            byte: 0,
            edid_index: 0,
            edid_data,
        }
    }

    /// Compose the DDC status bits — Bochs ddc.cc bx_ddc_c::read.
    /// Bit 3 = monitor data (wire-AND with the host), bit 2 = clock,
    /// bit 1 = host data, bit 0 = clock.
    pub(crate) fn read(&self) -> u8 {
        (((self.dda_mon & self.dda_host) as u8) << 3)
            | ((self.dck_host as u8) << 2)
            | ((self.dda_host as u8) << 1)
            | (self.dck_host as u8)
    }

    /// Drive the I2C lines — Bochs ddc.cc bx_ddc_c::write.
    pub(crate) fn write(&mut self, dck: bool, dda: bool) {
        if dck == self.dck_host && dda == self.dda_host {
            return;
        }
        let dck_change = dck != self.dck_host;
        let dda_change = dda != self.dda_host;
        if dck_change && dda_change {
            tracing::error!("DDC unknown: DCK={} DDA={}", dck as u8, dda as u8);
        } else if dck_change {
            if !dck {
                // Falling clock edge — advance the protocol stage.
                match self.stage {
                    DdcStage::Start => {
                        self.stage = DdcStage::Address;
                        self.bitshift = 6;
                        self.byte = 0;
                    }
                    DdcStage::Address => {
                        if self.bitshift > 0 {
                            self.bitshift -= 1;
                        } else {
                            self.ack = self.byte != 0x50;
                            self.stage = DdcStage::Rw;
                        }
                    }
                    DdcStage::Rw => {
                        self.stage = DdcStage::AckOut;
                        self.dda_mon = self.ack;
                    }
                    DdcStage::DataIn => {
                        if self.bitshift > 0 {
                            self.bitshift -= 1;
                        } else {
                            self.ack = false;
                            // Data byte sets the EDID offset address.
                            self.edid_index = self.byte;
                            self.dda_mon = self.ack;
                            self.stage = DdcStage::AckOut;
                        }
                    }
                    DdcStage::DataOut => {
                        if self.bitshift > 0 {
                            self.bitshift -= 1;
                            self.dda_mon = ((self.byte >> self.bitshift) & 1) != 0;
                        } else {
                            self.stage = DdcStage::AckIn;
                            self.dda_mon = true;
                        }
                    }
                    DdcStage::AckIn => {
                        if !self.ack {
                            self.bitshift = 7;
                            self.stage = DdcStage::DataOut;
                            self.byte = self.get_edid_byte();
                            self.dda_mon = ((self.byte >> self.bitshift) & 1) != 0;
                        } else {
                            self.stage = DdcStage::Stop;
                        }
                    }
                    DdcStage::AckOut => {
                        self.bitshift = 7;
                        if self.rw {
                            self.stage = DdcStage::DataOut;
                            self.byte = self.get_edid_byte();
                            self.dda_mon = ((self.byte >> self.bitshift) & 1) != 0;
                        } else {
                            self.stage = DdcStage::DataIn;
                            self.dda_mon = true;
                            self.byte = 0;
                        }
                    }
                    DdcStage::Stop => {}
                }
            } else {
                // Rising clock edge — sample the host data line.
                match self.stage {
                    DdcStage::Address | DdcStage::DataIn => {
                        self.byte |= (self.dda_host as u8) << self.bitshift;
                    }
                    DdcStage::Rw => {
                        self.rw = self.dda_host;
                    }
                    DdcStage::AckIn => {
                        self.ack = self.dda_host;
                    }
                    _ => {}
                }
            }
        } else {
            // Data transition with the clock steady: START/STOP conditions.
            if self.dck_host {
                if !dda {
                    self.stage = DdcStage::Start;
                } else {
                    self.stage = DdcStage::Stop;
                }
            }
        }
        self.dck_host = dck;
        self.dda_host = dda;
    }

    /// Bochs ddc.cc bx_ddc_c::get_edid_byte — the builtin EDID has no
    /// extension block, so the index wraps at 128 bytes.
    fn get_edid_byte(&mut self) -> u8 {
        let value = self.edid_data[self.edid_index as usize & 0x7F];
        self.edid_index = self.edid_index.wrapping_add(1) & 0x7F;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive one line at a time, like the VGABIOS does through VBE_DISPI_INDEX_DDC.
    struct Bus {
        dck: bool,
        dda: bool,
    }

    impl Bus {
        fn new() -> Self {
            Bus {
                dck: true,
                dda: true,
            }
        }
        fn set_dck(&mut self, ddc: &mut BxDdcC, dck: bool) {
            self.dck = dck;
            ddc.write(self.dck, self.dda);
        }
        fn set_dda(&mut self, ddc: &mut BxDdcC, dda: bool) {
            self.dda = dda;
            ddc.write(self.dck, self.dda);
        }
        /// Clock out one host bit: SDA while SCL low, then SCL high+low.
        fn send_bit(&mut self, ddc: &mut BxDdcC, bit: bool) {
            self.set_dda(ddc, bit);
            self.set_dck(ddc, true);
            self.set_dck(ddc, false);
        }
        /// Sample one monitor bit at SCL high, then take SCL low.
        fn recv_bit(&mut self, ddc: &mut BxDdcC) -> bool {
            self.set_dck(ddc, true);
            let bit = (ddc.read() >> 3) & 1 != 0;
            self.set_dck(ddc, false);
            bit
        }
    }

    #[test]
    fn edid_header_and_checksum_match_bochs() {
        let ddc = BxDdcC::new();
        assert_eq!(
            &ddc.edid_data[0..8],
            &[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00],
            "EDID header pattern"
        );
        let sum = ddc
            .edid_data
            .iter()
            .fold(0u8, |sum, &byte| sum.wrapping_add(byte));
        assert_eq!(sum, 0, "EDID block must sum to zero (ddc.cc init)");
    }

    #[test]
    fn idle_read_composes_released_lines() {
        // Power-on: DCK=1, DDA=1, DDAmon=1 → bits 3..0 all set.
        let ddc = BxDdcC::new();
        assert_eq!(ddc.read(), 0x0F);
    }

    #[test]
    fn i2c_read_transaction_returns_edid_bytes() {
        let mut ddc = BxDdcC::new();
        let mut bus = Bus::new();

        // START: SDA 1→0 with SCL high.
        bus.set_dda(&mut ddc, false);
        // First falling clock edge enters the address stage.
        bus.set_dck(&mut ddc, false);

        // 7-bit address 0x50 (MSB first), then the R/W bit (1 = read).
        for shift in (0..7).rev() {
            bus.send_bit(&mut ddc, (0x50 >> shift) & 1 != 0);
        }
        bus.send_bit(&mut ddc, true); // R/W = read

        // Monitor ACK (wire low) while the host releases SDA.
        bus.set_dda(&mut ddc, true);
        bus.set_dck(&mut ddc, true);
        assert_eq!((ddc.read() >> 3) & 1, 0, "address 0x50 must be ACKed");
        bus.set_dck(&mut ddc, false);

        // First data byte: EDID[0] = 0x00.
        let mut byte0 = 0u8;
        for _ in 0..8 {
            byte0 = (byte0 << 1) | bus.recv_bit(&mut ddc) as u8;
        }
        assert_eq!(byte0, 0x00, "EDID byte 0");

        // Host ACK (SDA low) requests the next byte.
        bus.send_bit(&mut ddc, false);
        bus.set_dda(&mut ddc, true); // release SDA again (SCL low)

        // Second data byte: EDID[1] = 0xFF.
        let mut byte1 = 0u8;
        for _ in 0..8 {
            byte1 = (byte1 << 1) | bus.recv_bit(&mut ddc) as u8;
        }
        assert_eq!(byte1, 0xFF, "EDID byte 1");

        // STOP: SDA 0→1 with SCL high.
        bus.set_dda(&mut ddc, false);
        bus.set_dck(&mut ddc, true);
        bus.set_dda(&mut ddc, true);
    }
}
