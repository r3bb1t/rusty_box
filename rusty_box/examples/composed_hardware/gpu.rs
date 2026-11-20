//! The model depends only on the architecture-independent device contracts.
use rusty_box_devices::api::{BxPhyAddress, WindowOffset};
use rusty_box_devices::composition::{DmaWrite, InterruptOutput, MmioModel, MmioWrite};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuEvent {
    Written(TileWritten),
    Rejected(TileRejected),
}

impl GpuEvent {
    pub fn written(&self) -> Option<&TileWritten> {
        match self {
            Self::Written(event) => Some(event),
            Self::Rejected(_) => None,
        }
    }

    pub fn rejected(&self) -> Option<&TileRejected> {
        match self {
            Self::Rejected(event) => Some(event),
            Self::Written(_) => None,
        }
    }
}

impl MmioModel for Gpu {
    type Events = Option<GpuEvent>;

    fn read(&mut self, at: WindowOffset, len: usize) -> u64 {
        match (at.get(), len) {
            (0, 4) => u64::from(self.destination),
            (8, 4) => u64::from(self.status),
            _ => u64::MAX,
        }
    }

    fn write(
        &mut self,
        access: MmioWrite,
        dma: &mut impl DmaWrite,
        irq: &mut impl InterruptOutput,
    ) -> Self::Events {
        match (access.offset.get(), access.len) {
            (0, 4) => self.destination = access.value as u32,
            (4, 4) => {
                let destination = BxPhyAddress::new(u64::from(self.destination));
                let pixels = (access.value as u32).to_le_bytes();
                if dma.write(destination, &pixels).is_err() {
                    self.status = 2;
                    return Some(GpuEvent::Rejected(TileRejected { destination }));
                }
                self.status = 1;
                irq.set_level(true);
                return Some(GpuEvent::Written(TileWritten {
                    destination,
                    pixels,
                }));
            }
            (12, 4) => irq.set_level(false),
            _ => {}
        }
        None
    }
}
