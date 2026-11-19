//! The model depends only on the architecture-independent device contracts.
use rusty_box_devices::api::{BxPhyAddress, WindowOffset};
use rusty_box_devices::composition::{DmaWrite, Emit, InterruptOutput, MmioWrite, Reaction};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileWritten {
    pub destination: BxPhyAddress,
    pub pixels: [u8; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileRejected {
    pub destination: BxPhyAddress,
}

#[derive(Default)]
pub struct Gpu {
    destination: u32,
    status: u32,
}

enum Notification {
    Written(TileWritten),
    Rejected(TileRejected),
}

impl Gpu {
    pub fn read(&self, at: WindowOffset, len: usize) -> u64 {
        match (at.get(), len) {
            (0, 4) => u64::from(self.destination),
            (8, 4) => u64::from(self.status),
            _ => u64::MAX,
        }
    }

    fn command(
        &mut self,
        access: MmioWrite,
        dma: &mut impl DmaWrite,
        irq: &mut impl InterruptOutput,
    ) -> Option<Notification> {
        match (access.offset.get(), access.len) {
            (0, 4) => self.destination = access.value as u32,
            (4, 4) => {
                let destination = BxPhyAddress::new(u64::from(self.destination));
                let pixels = (access.value as u32).to_le_bytes();
                if dma.write(destination, &pixels).is_err() {
                    self.status = 2;
                    return Some(Notification::Rejected(TileRejected { destination }));
                }
                self.status = 1;
                irq.set_level(true);
                return Some(Notification::Written(TileWritten {
                    destination,
                    pixels,
                }));
            }
            (12, 4) => irq.set_level(false),
            _ => {}
        }
        None
    }

    pub fn subscriber_failed(&mut self) {
        self.status = 3;
    }
}

// The owner wires two distinct event sinks. A notification is produced only
// after command() returns and releases its borrow of the device and the DMA.
pub fn write<I: InterruptOutput, X>(
    gpu: &mut Gpu,
    access: MmioWrite,
    dma: &mut impl DmaWrite,
    irq: &mut I,
    written: &mut impl Emit<TileWritten, I, Error = X>,
    rejected: &mut impl Emit<TileRejected, I, Error = X>,
) -> Result<Reaction, X> {
    match gpu.command(access, dma, irq) {
        Some(Notification::Written(event)) => written.emit(&event, irq),
        Some(Notification::Rejected(event)) => rejected.emit(&event, irq),
        None => Ok(Reaction::Continue),
    }
}
