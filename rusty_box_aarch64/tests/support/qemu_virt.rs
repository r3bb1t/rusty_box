//! The interrupt map QEMU's `virt` wires (`hw/arm/virt.c` `a15irqmap`), for
//! the tests that boot a tree QEMU wrote (`assets/qemu-virt-a57-256m.dtb`),
//! which names these SPIs: a host booting such a tree gives the board this
//! map, as the board refuses a tree naming other SPIs than its map's.

use rusty_box_aarch64::{InterruptMap, MsiCount, MsiExtent, MsiSpis, Spi};

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
