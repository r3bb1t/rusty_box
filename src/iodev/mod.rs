use alloc::{boxed::Box, string::String};

pub(crate) mod devices;

struct IoHandlerStruct {
    next: Box<Option<Self>>,
    prev: Box<Option<Self>>,
    funct: fn(),
    this_ptr: Option<()>,
    handler_name: String,
    usage_count: u16,
    mask: u8, // io_len mask
}

struct BxDevicesC {}

impl BxDevicesC {
    pub(crate) fn default_read_handler(&self, _address: u32, _io_len: u8) -> u32 {
        0xffffffff
    }

    pub(crate) fn default_write_handler(&self, _address: u32, _value: u32, _io_len: u8) {}

    // pub(crate) fn register_default_io_read_handler(
    //     &mut self,
    //     this_ptr: Option<&self>,
    //     f: bx_read_handler_t,
    //     name: &'static str,
    //     mask: u8,
    // ) {
    // }
}
