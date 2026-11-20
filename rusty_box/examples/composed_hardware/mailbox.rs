//! An independent register model that needs only an interrupt output.
use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::{InterruptOutput, IrqMmioModel, MmioWrite};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessagePosted(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoorbellRequested;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageTaken(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxEvent {
    Posted(MessagePosted),
    Doorbell(DoorbellRequested),
    Taken(MessageTaken),
}

impl MailboxEvent {
    pub fn posted(&self) -> Option<&MessagePosted> {
        match self {
            Self::Posted(message) => Some(message),
            _ => None,
        }
    }
}

type Notifications = core::iter::Flatten<core::option::IntoIter<[MailboxEvent; 2]>>;

#[derive(Default)]
pub struct Mailbox {
    message: Option<u32>,
}

impl Mailbox {
    pub fn take(&mut self, irq: &mut impl InterruptOutput) -> Option<MailboxEvent> {
        let message = self.message.take()?;
        irq.set_level(false);
        Some(MailboxEvent::Taken(MessageTaken(message)))
    }
}

impl IrqMmioModel for Mailbox {
    type Events = Notifications;

    fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        match (offset.get(), len) {
            (0, 4) => self.message.unwrap_or(0).into(),
            _ => u64::MAX,
        }
    }

    fn write(&mut self, access: MmioWrite, irq: &mut impl InterruptOutput) -> Self::Events {
        let events = match (access.offset.get(), access.len) {
            (0, 4) => {
                let message = access.value as u32;
                self.message = Some(message);
                irq.set_level(true);
                Some([
                    MailboxEvent::Posted(MessagePosted(message)),
                    MailboxEvent::Doorbell(DoorbellRequested),
                ])
            }
            (4, 4) => {
                irq.set_level(false);
                None
            }
            _ => None,
        };
        events.into_iter().flatten()
    }
}
