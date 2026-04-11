//! PCI Host Bridge (i440FX PMC)
//!
//! Matches Bochs `iodev/pci.cc` (706 lines) + `pci.h` (85 lines).
//!
//! Implements:
//! - i440FX PMC (PCI/Memory Controller) — bus 0, device 0, function 0
//! - PCI configuration address register (port 0xCF8)
//! - PCI configuration data register (port 0xCFC-0xCFF)
//! - PAM (Programmable Attribute Map) registers (0x59-0x5F)
//! - SMRAM control register (0x72)
//! - DRAM Row Boundary Array (DRBA) registers (0x60-0x67)
//!
//! The host bridge is the root of the PCI bus and handles configuration
//! space routing for all PCI devices.

/// PCI configuration space size (256 bytes per device/function)
const PCI_CONF_SIZE: usize = 256;

/// PCI configuration address port (Bochs: devices.cc uses pci_conf_addr)
pub const PCI_CONFIG_ADDR: u16 = 0x0CF8;
/// PCI configuration data port (base — also 0xCF9, 0xCFA, 0xCFB)
pub const PCI_CONFIG_DATA: u16 = 0x0CFC;

/// Encode a PCI device/function number: (device << 3) | function
/// Bochs: BX_PCI_DEVICE(device, function) macro (pci.h:32)
pub const fn pci_device(device: u8, function: u8) -> u8 {
    (device << 3) | (function & 7)
}

/// PCI interrupt pin constants (Bochs pci.h:34-39)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PciIntPin {
    IntA = 1,
    IntB = 2,
    IntC = 3,
    IntD = 4,
}

// ─── i440FX Host Bridge ─────────────────────────────────────────────────────

/// i440FX PMC (PCI/Memory Controller Hub).
/// Bus 0, Device 0, Function 0.
/// Bochs: bx_pci_bridge_c (pci.h:43-71, pci.cc)
#[derive(Debug)]
pub struct BxPciBridge {
    /// PCI configuration space (256 bytes)
    pub pci_conf: [u8; PCI_CONF_SIZE],
    /// DRAM Row Boundary Array (8 entries)
    /// Bochs: DRBA[8] (pci.h:67)
    drba: [u8; 8],
    /// DRAM detection state (bitmask of changed DRBA registers)
    /// Bochs: dram_detect (pci.h:68)
    dram_detect: u8,
}

impl Default for BxPciBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl BxPciBridge {
    /// Create a new i440FX host bridge.
    /// Bochs: bx_pci_bridge_c::bx_pci_bridge_c() + init() (pci.cc:58-199)
    pub fn new() -> Self {
        let mut bridge = Self {
            pci_conf: [0; PCI_CONF_SIZE],
            drba: [0; 8],
            dram_detect: 0,
        };
        bridge.init_pci_conf();
        bridge
    }

    /// Initialize PCI configuration space with i440FX identity.
    /// Bochs: init_pci_conf(0x8086, 0x1237, 0x00, 0x060000, 0x00, 0) (pci.cc:116)
    fn init_pci_conf(&mut self) {
        // Vendor ID: Intel (0x8086)
        self.pci_conf[0x00] = 0x86;
        self.pci_conf[0x01] = 0x80;
        // Device ID: i440FX (0x1237)
        self.pci_conf[0x02] = 0x37;
        self.pci_conf[0x03] = 0x12;
        // Revision: 0x00
        self.pci_conf[0x08] = 0x00;
        // Class code: Host bridge (0x060000)
        self.pci_conf[0x09] = 0x00;
        self.pci_conf[0x0A] = 0x00;
        self.pci_conf[0x0B] = 0x06;
        // Header type: 0x00 (single-function)
        self.pci_conf[0x0E] = 0x00;
    }

    /// Initialize DRAM row boundary registers based on RAM size.
    /// Bochs: pci.cc:171-190 (i440FX path)
    pub fn init_dram(&mut self, ramsize_mb: u32) {
        let mut ramsize = ramsize_mb;
        let module_types: [u8; 3] = [128, 32, 8];

        if (ramsize & 0x07) != 0 {
            ramsize = (ramsize & !0x07) + 8;
        }
        if ramsize > 1024 {
            ramsize = 1024;
        }

        let mut drbval: u8 = 0;
        let mut row: usize = 0;
        let mut ti: usize = 0;

        while ramsize > 0 && row < 8 && ti < 3 {
            let mc = ramsize / module_types[ti] as u32;
            ramsize %= module_types[ti] as u32;
            for _ in 0..mc {
                drbval += module_types[ti] >> 3;
                self.drba[row] = drbval;
                row += 1;
                if row == 8 {
                    break;
                }
            }
            ti += 1;
        }
        while row < 8 {
            self.drba[row] = drbval;
            row += 1;
        }

        // Copy DRBA to config space registers 0x60-0x67
        for i in 0..8 {
            self.pci_conf[0x60 + i] = self.drba[i];
        }
    }

    /// Reset the host bridge.
    /// Bochs: bx_pci_bridge_c::reset() (pci.cc:201-245) — i440FX path
    pub fn reset(&mut self) {
        // Command register (pci.cc:206-207)
        self.pci_conf[0x04] = 0x06;
        self.pci_conf[0x05] = 0x00;
        // Status register (pci.cc:226)
        self.pci_conf[0x06] = 0x80;
        self.pci_conf[0x07] = 0x02;
        // Latency timer and header type
        self.pci_conf[0x0D] = 0x00;
        self.pci_conf[0x0F] = 0x00;
        // Host bridge control (pci.cc:211-217)
        self.pci_conf[0x50] = 0x00;
        self.pci_conf[0x51] = 0x01; // i440FX: pci.cc:227
        self.pci_conf[0x52] = 0x00;
        self.pci_conf[0x53] = 0x80;
        self.pci_conf[0x54] = 0x00;
        self.pci_conf[0x55] = 0x00;
        self.pci_conf[0x56] = 0x00;
        self.pci_conf[0x57] = 0x01;
        self.pci_conf[0x58] = 0x10; // i440FX: pci.cc:228
                                    // PAM registers 0x59-0x5F: all zeros (pci.cc:235-236)
        for i in 0x59..0x60 {
            self.pci_conf[i] = 0x00;
        }
        // SMRAM control (pci.cc:241)
        self.pci_conf[0x72] = 0x02;
        // ERRCMD/ERRSTS (pci.cc:229-232)
        self.pci_conf[0xB4] = 0x00;
        self.pci_conf[0xB9] = 0x00;
        self.pci_conf[0xBA] = 0x00;
        self.pci_conf[0xBB] = 0x00;

        self.dram_detect = 0;
    }

    /// Write to PCI configuration space.
    /// Bochs: bx_pci_bridge_c::pci_write_handler() (pci.cc:265-452) — i440FX path
    /// Returns `true` if PAM registers were modified (caller must update memory types).
    pub fn pci_write(&mut self, address: u8, value: u32, io_len: u8) -> bool {
        let mut pam_changed = false;
        // BARs are read-only (pci.cc:275-276)
        if (0x10..0x34).contains(&address) {
            return false;
        }

        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr >= PCI_CONF_SIZE {
                break;
            }
            let value8 = ((value >> (i * 8)) & 0xFF) as u8;
            let oldval = self.pci_conf[addr];

            match addr {
                // Command register (pci.cc:286-288) — i440FX
                0x04 => {
                    self.pci_conf[addr] = (value8 & 0x40) | 0x06;
                }
                // Command high byte (pci.cc:291-293)
                0x05 => {
                    self.pci_conf[addr] = value8 & 0x01;
                }
                // Status lo — read-only (pci.cc:308-311)
                0x06 | 0x0C | 0x0F => {}
                // Status hi — write-1-to-clear (pci.cc:298-299)
                0x07 => {
                    let clear_bits = (self.pci_conf[0x07] & !value8) | 0x02;
                    self.pci_conf[addr] = clear_bits;
                }
                // Latency timer (pci.cc:305-306)
                0x0D => {
                    self.pci_conf[addr] = value8 & 0xF8;
                }
                // NBXCFG (pci.cc:317-319) — i440FX
                0x50 => {
                    self.pci_conf[addr] = value8 & 0x70;
                }
                // NBXCFG+1 (pci.cc:324-326) — i440FX
                0x51 => {
                    self.pci_conf[addr] = (value8 & 0x80) | 0x01;
                }
                // PAM registers (pci.cc:328-352)
                0x59..=0x5F => {
                    if value8 != oldval {
                        self.pci_conf[addr] = value8;
                        pam_changed = true;
                        tracing::info!(
                            "i440FX PAM register {:#04x} = {:#04x} (memory shadowing changed)",
                            addr,
                            value8
                        );
                    }
                }
                // DRBA registers (pci.cc:353-369)
                0x60..=0x67 => {
                    self.pci_conf[addr] = value8;
                    let drba_reg = addr & 0x07;
                    let drba_changed = self.pci_conf[0x60 + drba_reg] != self.drba[drba_reg];
                    if drba_changed {
                        self.dram_detect |= 1 << drba_reg;
                    } else if self.dram_detect != 0 {
                        self.dram_detect &= !(1 << drba_reg);
                    }
                }
                // SMRAM control (pci.cc:370-372)
                0x72 => {
                    self.smram_control(value8);
                }
                // ERRCMD (pci.cc:380-383) — preserve bits 1,3 from old, write bits 0,2,4-7 from new
                0x7A => {
                    self.pci_conf[addr] = (value8 & 0xF5) | (self.pci_conf[addr] & 0x0A);
                }
                // ERRSTS (pci.cc:417-418) — read-only in i440FX
                0xB8 => {}
                // Default: store value (pci.cc:434-436)
                _ => {
                    self.pci_conf[addr] = value8;
                }
            }
        }

        if self.dram_detect > 0 {
            tracing::debug!(
                "DRAM module detection triggered (detect={:#04x})",
                self.dram_detect
            );
        }
        pam_changed
    }


    /// Apply PAM register settings to the memory subsystem.
    /// Called after pci_write returns pam_changed=true.
    pub fn apply_pam_to_memory<'c>(
        &self,
        mem: &mut crate::memory::BxMemC<'c>,
    ) {
        let pam59 = self.pci_conf[0x59];
        mem.set_memory_type(12, 0, (pam59 >> 4) & 0x1 != 0);
        mem.set_memory_type(12, 1, (pam59 >> 5) & 0x1 != 0);

        for reg_idx in 0x5Au8..=0x5F {
            let pam_val = self.pci_conf[reg_idx as usize];
            let base_area = ((reg_idx - 0x5A) as usize) << 1;
            mem.set_memory_type(base_area, 0, pam_val & 0x1 != 0);
            mem.set_memory_type(base_area, 1, (pam_val >> 1) & 0x1 != 0);
            mem.set_memory_type(base_area + 1, 0, (pam_val >> 4) & 0x1 != 0);
            mem.set_memory_type(base_area + 1, 1, (pam_val >> 5) & 0x1 != 0);
        }

        tracing::info!("PAM registers applied to memory subsystem (deferred)");
    }

    /// Read from PCI configuration space.
    pub fn pci_read(&self, address: u8, io_len: u8) -> u32 {
        let mut value: u32 = 0;
        for i in 0..io_len as usize {
            let addr = address as usize + i;
            if addr < PCI_CONF_SIZE {
                value |= (self.pci_conf[addr] as u32) << (i * 8);
            }
        }
        value
    }

    /// SMRAM control register handler.
    /// Bochs: bx_pci_bridge_c::smram_control() (pci.cc:504-558)
    fn smram_control(&mut self, value8: u8) {
        let mut v = (value8 & 0x78) | 0x02; // ignore reserved bits

        // If DLCK is set, force DOPEN=0 and keep DLCK=1
        if self.pci_conf[0x72] & 0x10 != 0 {
            v &= 0xBF; // clear DOPEN
            v |= 0x10; // set DLCK
        }

        if (v & 0x08) == 0 {
            // SMRAME=0: disable SMRAM
            tracing::debug!("SMRAM disabled");
        } else {
            let dopen = (v & 0x40) != 0;
            let dcls = (v & 0x20) != 0;
            if dopen && dcls {
                tracing::warn!("SMRAM: DOPEN and DCLS both set (invalid)");
            }
            tracing::debug!("SMRAM enabled: DOPEN={}, DCLS={}", dopen, dcls);
        }

        tracing::info!("SMRAM control register set to {:#04x}", v);
        self.pci_conf[0x72] = v;
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pci_bridge_new() {
        let bridge = BxPciBridge::new();
        // Vendor ID: Intel
        assert_eq!(bridge.pci_conf[0x00], 0x86);
        assert_eq!(bridge.pci_conf[0x01], 0x80);
        // Device ID: i440FX
        assert_eq!(bridge.pci_conf[0x02], 0x37);
        assert_eq!(bridge.pci_conf[0x03], 0x12);
        // Class: Host bridge
        assert_eq!(bridge.pci_conf[0x0B], 0x06);
    }

    #[test]
    fn test_pci_bridge_reset() {
        let mut bridge = BxPciBridge::new();
        bridge.pci_conf[0x59] = 0xFF;
        bridge.reset();
        // PAM should be cleared
        assert_eq!(bridge.pci_conf[0x59], 0x00);
        // Command register
        assert_eq!(bridge.pci_conf[0x04], 0x06);
        // SMRAM
        assert_eq!(bridge.pci_conf[0x72], 0x02);
    }

    #[test]
    fn test_pci_device_macro() {
        assert_eq!(pci_device(0, 0), 0x00); // Device 0, Func 0
        assert_eq!(pci_device(1, 0), 0x08); // Device 1, Func 0
        assert_eq!(pci_device(1, 1), 0x09); // Device 1, Func 1
        assert_eq!(pci_device(1, 3), 0x0B); // Device 1, Func 3
    }

    #[test]
    fn test_dram_init_32mb() {
        let mut bridge = BxPciBridge::new();
        bridge.init_dram(32);
        // 32MB = 4 * 8MB modules -> drbval increments by 1 (8>>3=1) per row
        // Actually: type[0]=128, 32/128=0; type[1]=32, 32/32=1 -> drbval=4
        assert_eq!(bridge.drba[0], 4); // 32 >> 3 = 4
        for i in 1..8 {
            assert_eq!(bridge.drba[i], 4);
        }
    }

    #[test]
    fn test_pci_write_command_reg() {
        let mut bridge = BxPciBridge::new();
        bridge.reset();
        // Write to command register — only bit 6 writable, others forced
        bridge.pci_write(0x04, 0xFF, 1);
        assert_eq!(bridge.pci_conf[0x04], 0x46); // (0xFF & 0x40) | 0x06
    }

    #[test]
    fn test_pci_write_bar_region_ignored() {
        let mut bridge = BxPciBridge::new();
        bridge.reset();
        let old_10 = bridge.pci_conf[0x10];
        bridge.pci_write(0x10, 0xFFFFFFFF, 4);
        // BAR region (0x10-0x33) writes are ignored
        assert_eq!(bridge.pci_conf[0x10], old_10);
    }

    #[test]
    fn test_smram_dlck() {
        let mut bridge = BxPciBridge::new();
        bridge.reset();
        // Set DLCK (bit 4) — should lock DOPEN to 0
        bridge.pci_write(0x72, 0x18, 1); // SMRAME=1, DLCK=1
        assert_ne!(bridge.pci_conf[0x72] & 0x10, 0); // DLCK set
                                                     // Try to set DOPEN — should fail because DLCK is set
        bridge.pci_write(0x72, 0x48, 1); // DOPEN=1
        assert_eq!(bridge.pci_conf[0x72] & 0x40, 0); // DOPEN stays 0
    }
}
