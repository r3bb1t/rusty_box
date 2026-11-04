//! The device models the `virt` board instantiates. Each is decoded by
//! register offset within its own window and knows nothing of where the
//! board places it.

pub mod cfi_flash;
pub mod gicv2;
pub mod pl011;
pub mod pl031;
pub mod pl061;
pub(crate) mod virtio;
pub mod virtio_mmio;
