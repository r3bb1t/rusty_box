//! QEMU's `virt` board as the tests that boot a tree QEMU wrote
//! (`assets/qemu-virt-a57-256m.dtb`) configure the board: its interrupt map
//! (`hw/arm/virt.c` `a15irqmap`) and its device layout, which the tree
//! names, as the board refuses a tree naming other SPIs or addresses than
//! its own.

use rusty_box_aarch64::{
    DeviceLayout, InterruptMap, MsiCount, MsiExtent, MsiSpis, Spi, VirtConfig,
};

/// `config` with QEMU's interrupt map and device layout.
pub fn qemu_virt(config: VirtConfig) -> VirtConfig {
    VirtConfig {
        interrupts: qemu_virt_interrupts(),
        layout: DeviceLayout::QemuVirt,
        ..config
    }
}

pub fn qemu_virt_interrupts() -> InterruptMap {
    let spi = |n| Spi::new(n).expect("an SPI");
    InterruptMap {
        uart: spi(1),
        rtc: spi(2),
        gpio: spi(7),
        virtio: spi(16),
        pcie_intx: spi(3),
        msi: MsiSpis {
            first: spi(48),
            count: MsiExtent::Count(MsiCount::new(64).expect("a count")),
        },
    }
}
