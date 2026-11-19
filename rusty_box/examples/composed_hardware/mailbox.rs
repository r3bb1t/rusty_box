//! An independent register model with a different event and no DMA.
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{DmaWrite, InterruptOutput, MmioModel, MmioWrite};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessagePosted(pub u32);

#[derive(Default)]
pub struct Mailbox {
    message: u32,
}

impl MmioModel for Mailbox {
    type Event = MessagePosted;

    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        match (offset.get(), len) {
            (0, 4) => self.message.into(),
            _ => u64::MAX,
        }
    }

    fn write(
        &mut self,
        access: MmioWrite,
        _: &mut impl DmaWrite,
        irq: &mut impl InterruptOutput,
    ) -> Option<Self::Event> {
        match (access.offset.get(), access.len) {
            (0, 4) => {
                self.message = access.value as u32;
                irq.set_level(true);
                Some(MessagePosted(self.message))
            }
            (4, 4) => {
                irq.set_level(false);
                None
            }
            _ => None,
        }
    }
}
