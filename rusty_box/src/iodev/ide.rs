//! The IDE controller as one device.
//!
//! Bochs models the ATA/ATAPI drives (`harddrv.cc`) and the PIIX bus-master
//! engine (`pci_ide.cc`) as separate objects, but they are one controller: the
//! drives raise the bus-master interrupt bit, the engine drives sector
//! transfers through the drives, and a DMA transfer needs both plus a bounce
//! buffer at once. This port inherited the split and paid for it by threading
//! `&mut BxPicC, &mut BxPciIde` through every ATA entry point and by parking
//! the bounce buffer on the device manager, where three unrelated fields had
//! to be destructured together to run one transfer.
//!
//! `IdeSubsystem` owns all three. The threading becomes an internal detail, so
//! callers hold one device instead of reaching across the manager.

use super::harddrv::BxHardDriveC;
use super::pci_ide::BxPciIde;
use crate::pic::BxPicC;

/// Bounce-buffer capacity for one bus-master transfer — Bochs pci_ide.cc
/// copies through a scratch region of this size.
const BMDMA_SCRATCH_LEN: usize = 0x10000;

pub struct IdeSubsystem {
    /// ATA/ATAPI drives — Bochs `bx_hard_drive_c`.
    pub(crate) drives: BxHardDriveC,
    /// PIIX bus-master engine — Bochs `bx_pci_ide_c`.
    pub(crate) bus_master: BxPciIde,
    /// Bounce buffer for a bus-master transfer. Owned here because a transfer
    /// needs it together with both halves above.
    pub(crate) scratch: [u8; BMDMA_SCRATCH_LEN],
}

impl core::fmt::Debug for IdeSubsystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IdeSubsystem")
            .field("drives", &self.drives)
            .field("bus_master", &self.bus_master)
            .field("scratch", &format_args!("[u8; {BMDMA_SCRATCH_LEN}]"))
            .finish()
    }
}

impl Default for IdeSubsystem {
    fn default() -> Self {
        Self::new()
    }
}

impl IdeSubsystem {
    pub fn new() -> Self {
        Self {
            drives: BxHardDriveC::new(),
            bus_master: BxPciIde::new(),
            scratch: [0; BMDMA_SCRATCH_LEN],
        }
    }

    /// The two halves plus the bounce buffer, borrowed disjointly.
    ///
    /// For the transfer path, which genuinely needs all three at once. Every
    /// other caller should use the forwarding methods below rather than
    /// re-creating the cross-field threading this type exists to remove.
    #[inline]
    pub(crate) fn split(
        &mut self,
    ) -> (
        &mut BxHardDriveC,
        &mut BxPciIde,
        &mut [u8; BMDMA_SCRATCH_LEN],
    ) {
        (&mut self.drives, &mut self.bus_master, &mut self.scratch)
    }

    #[inline]
    pub(crate) fn read(&mut self, port: u16, io_len: u8, pic: &mut BxPicC) -> u32 {
        self.drives.read(port, io_len, pic, &mut self.bus_master)
    }

    #[inline]
    pub(crate) fn write(&mut self, port: u16, value: u32, io_len: u8, pic: &mut BxPicC) {
        self.drives
            .write(port, value, io_len, pic, &mut self.bus_master)
    }

    #[inline]
    pub(crate) fn bulk_read_data(
        &mut self,
        port: u16,
        io_len: u8,
        buf: &mut [u8],
        pic: &mut BxPicC,
    ) -> usize {
        self.drives
            .bulk_read_data(port, io_len, buf, pic, &mut self.bus_master)
    }

    #[inline]
    pub(crate) fn seek_timer(&mut self, param: u8, pic: &mut BxPicC) {
        self.drives.seek_timer(param, pic, &mut self.bus_master)
    }

}
