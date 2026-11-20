//! A narrow register model using ordinary methods and a typed notification.

use rusty_box_devices::api::WindowOffset;
use rusty_box_devices::composition::MmioWrite;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisterChanged {
    pub offset: WindowOffset,
    pub value: u32,
}

pub struct RegisterBank(u32);

impl RegisterBank {
    pub fn new(initial: u32) -> Self {
        Self(initial)
    }

    pub fn read(&mut self, offset: WindowOffset, len: usize) -> u64 {
        match (offset.get(), len) {
            (0 | 4096, 4) => u64::from(self.0),
            (0 | 4095 | 8191, 1) => u64::from(self.0 as u8),
            _ => u64::MAX,
        }
    }

    pub fn write(&mut self, access: MmioWrite) -> Option<RegisterChanged> {
        match (access.offset.get(), access.len) {
            (0 | 4096, 4) => self.0 = access.value as u32,
            (0 | 4095 | 8191, 1) => self.0 = (self.0 & !255) | u32::from(access.value as u8),
            _ => return None,
        }
        Some(RegisterChanged {
            offset: access.offset,
            value: self.0,
        })
    }
}
