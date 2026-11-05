//! Unit tests of `src/boot/tree.rs`: the board's own device tree, read as a
//! guest reads it (Devicetree Specification v0.4) — what each node says,
//! that it says it the board's way rather than QEMU's, and that the tree
//! is well formed.

use std::vec;
use std::vec::Vec;

use super::{write, BoardTree, Chosen, InitrdRange};
use crate::boot::fdt;
use crate::bus::PhysAddr;
use crate::cpu::id::{CCSIDR_L1D, CCSIDR_L1I, CCSIDR_L2};
use crate::cpu::PsciConduit;
use crate::devices::gicv2::GicExtensions;
use crate::identity::{BoardIdentity, IdentityNames};
use crate::CpuModel;

const RAM: u64 = 256 << 20;

fn board(identity: &BoardIdentity, gic: GicExtensions, chosen: Chosen<'static>) -> Vec<u8> {
    let tree = BoardTree {
        identity,
        ram_len: RAM,
        cpu: CpuModel::CortexA57,
        psci: PsciConduit::Hvc,
        gic,
        chosen,
    };
    let mut out = vec![0u8; 64 * 1024];
    let len = write(&tree, &mut out).expect("fits");
    out.truncate(len);
    out
}

const PLAIN: Chosen<'static> = Chosen {
    bootargs: None,
    initrd: None,
};

fn plain() -> Vec<u8> {
    board(&BoardIdentity::RUSTY_BOX, GicExtensions::Absent, PLAIN)
}

fn prop<'t>(tree: &'t [u8], path: &str, name: &str) -> Option<&'t [u8]> {
    fdt::property(tree, path, name).expect("well formed")
}

fn cells(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// The tree's header (5.2) holds together: its blocks lie where it says,
/// in order, and the tree is as long as it says. Rebuilding it with no
/// edit, which re-reads every token, gives it back byte for byte.
#[test]
fn the_tree_is_a_well_formed_flattened_tree() {
    let tree = plain();
    let word = |at: usize| u32::from_be_bytes(tree[at..at + 4].try_into().expect("four bytes"));
    assert_eq!(word(0), 0xD00D_FEED);
    assert_eq!(word(4) as usize, tree.len(), "totalsize");
    let (structure, strings, reserve) = (word(8) as usize, word(12) as usize, word(16) as usize);
    assert_eq!((reserve, structure), (40, 56), "an empty reservation block");
    assert_eq!(&tree[40..56], &[0; 16]);
    assert_eq!(structure + word(36) as usize, strings, "the strings follow");
    assert_eq!(strings + word(32) as usize, tree.len());
    assert_eq!((word(20), word(24)), (17, 16), "version 17, back to 16");
    let mut again = vec![0u8; 64 * 1024];
    let len = fdt::rebuild(&tree, &[], &[], &mut again).expect("rebuilds");
    assert_eq!(&again[..len], &tree[..]);
}

/// The root names the board by its identity and describes the RAM it
/// was built with; `/chosen` names the console by its alias, as EBBR
/// requires, at the rate a directly booted kernel finds it at.
#[test]
fn the_root_memory_and_console_are_the_boards() {
    let tree = plain();
    assert_eq!(prop(&tree, "", "model"), Some(&b"Rusty Box virt\0"[..]));
    assert_eq!(prop(&tree, "", "compatible"), Some(&b"rusty-box,virt\0"[..]));
    assert_eq!(
        prop(&tree, "memory@40000000", "reg").map(cells),
        Some(vec![0, 0x4000_0000, 0, (RAM) as u32])
    );
    assert_eq!(
        prop(&tree, "chosen", "stdout-path"),
        Some(&b"serial0:115200n8\0"[..])
    );
    assert_eq!(
        prop(&tree, "aliases", "serial0"),
        Some(&b"/serial@9000000\0"[..])
    );
    assert_eq!(
        prop(&tree, "serial@9000000", "compatible"),
        Some(&b"arm,pl011\0arm,primecell\0"[..])
    );

    let host = BoardIdentity::new(IdentityNames {
        model: "Acme Board",
        compatible: &["acme,board-1", "acme,board"],
        ..IdentityNames::RUSTY_BOX
    })
    .unwrap_or_else(|e| panic!("{e}"));
    let tree = board(&host, GicExtensions::Absent, PLAIN);
    assert_eq!(prop(&tree, "", "model"), Some(&b"Acme Board\0"[..]));
    assert_eq!(
        prop(&tree, "", "compatible"),
        Some(&b"acme,board-1\0acme,board\0"[..])
    );
}

/// The processor node's L1 cache properties and the L2 node it names
/// (Devicetree Specification v0.4, 3.7.3 and 3.8) describe the caches
/// `CCSIDR_EL1` reports, the Cortex-A57's (TRM DDI 0488C, 6.1: a 32 KiB
/// 2-way data cache and a 48 KiB 3-way instruction cache, 64-byte lines;
/// a 2 MiB 16-way L2 of the sizes the TRM allows).
#[test]
fn the_caches_are_those_ccsidr_reports() {
    let tree = plain();
    let cell = |path: &str, name: &str| -> u64 {
        let bytes = prop(&tree, path, name).unwrap_or_else(|| panic!("{path} {name}"));
        u64::from(u32::from_be_bytes(bytes.try_into().expect("one cell")))
    };
    // CCSIDR_EL1 without FEAT_CCIDX: LineSize [2:0] is log2(bytes) - 4,
    // Associativity [12:3] ways - 1, NumSets [27:13] sets - 1.
    let caches = [
        (CCSIDR_L1D, 2, 32 * 1024, "cpus/cpu@0", ["d-cache-block-size", "d-cache-sets", "d-cache-size"]),
        (CCSIDR_L1I, 3, 48 * 1024, "cpus/cpu@0", ["i-cache-block-size", "i-cache-sets", "i-cache-size"]),
        (CCSIDR_L2, 16, 2 * 1024 * 1024, "cpus/l2-cache", ["cache-block-size", "cache-sets", "cache-size"]),
    ];
    for (ccsidr, ways, size, path, [block_size, set_count, total]) in caches {
        let line = 1u64 << ((ccsidr & 7) + 4);
        let associativity = ((ccsidr >> 3) & 0x3FF) + 1;
        let sets = ((ccsidr >> 13) & 0x7FFF) + 1;
        assert_eq!(
            (line, associativity, line * associativity * sets),
            (64, ways, size),
            "{total}"
        );
        assert_eq!(cell(path, block_size), line);
        assert_eq!(cell(path, set_count), sets);
        assert_eq!(cell(path, total), size);
    }
    assert_eq!(
        cell("cpus/cpu@0", "next-level-cache"),
        cell("cpus/l2-cache", "phandle"),
        "the L1 caches' next level is the L2"
    );
    assert_eq!(cell("cpus/l2-cache", "cache-level"), 2);
    assert_eq!(prop(&tree, "cpus/l2-cache", "cache-unified"), Some(&[][..]));
    assert_eq!(
        prop(&tree, "cpus/cpu@0", "compatible"),
        Some(&b"arm,cortex-a57\0"[..])
    );
}

/// The GIC is described by the GICv2 binding, each frame by the region it
/// decodes; with the Virtualization Extensions, the virtual interface's
/// two frames follow and the maintenance interrupt, PPI 9, is named.
#[test]
fn the_gic_names_the_frames_it_decodes() {
    let gic = "interrupt-controller@8000000";
    let tree = plain();
    assert_eq!(
        prop(&tree, gic, "compatible"),
        Some(&b"arm,cortex-a15-gic\0"[..])
    );
    assert_eq!(
        prop(&tree, gic, "reg").map(cells),
        Some(vec![0, 0x0800_0000, 0, 0x1000, 0, 0x0801_0000, 0, 0x2000])
    );
    assert_eq!(prop(&tree, gic, "interrupts"), None);
    let tree = board(
        &BoardIdentity::RUSTY_BOX,
        GicExtensions::Virtualization,
        PLAIN,
    );
    assert_eq!(
        prop(&tree, gic, "reg").map(cells),
        Some(vec![
            0,
            0x0800_0000,
            0,
            0x1000,
            0,
            0x0801_0000,
            0,
            0x2000,
            0,
            0x0803_0000,
            0,
            0x2000,
            0,
            0x0804_0000,
            0,
            0x2000
        ])
    );
    assert_eq!(
        prop(&tree, gic, "interrupts").map(cells),
        Some(vec![1, 9, 0x104])
    );
    assert_eq!(
        prop(&tree, &std::format!("{gic}/msi-controller@8020000"), "compatible"),
        Some(&b"arm,gic-v2m-frame\0"[..])
    );
}

/// Nothing a guest reads names QEMU or describes what the board lacks: no
/// `qemu,` compatible, no `fw-cfg` or platform bus, no `linux,dummy-virt`,
/// no PMU node (`ID_AA64DFR0_EL1.PMUVer` is 0), no PSCI 0.1 binding or
/// its function identifiers, and no node named after QEMU's
/// (`pl011@`, `intc@`, `virtio_mmio@`, `apb-pclk`).
#[test]
fn nothing_in_the_tree_is_qemus() {
    for gic in [GicExtensions::Absent, GicExtensions::Virtualization] {
        let tree = board(&BoardIdentity::RUSTY_BOX, gic, PLAIN);
        let lower: Vec<u8> = tree.iter().map(u8::to_ascii_lowercase).collect();
        for needle in [
            &b"qemu"[..],
            b"fw-cfg",
            b"platform",
            b"dummy-virt",
            b"pmu",
            b"pl011@",
            b"pl031@",
            b"pl061@",
            b"intc@",
            b"v2m@",
            b"virtio_mmio",
            b"apb-pclk",
            b"clk24mhz",
            b"poweroff",
            b"cpu_on",
            b"cpu_off",
            b"cpu_suspend",
            b"migrate",
            b"linux,pci-domain",
        ] {
            assert!(
                !lower.windows(needle.len()).any(|w| w == needle),
                "{}",
                std::string::String::from_utf8_lossy(needle)
            );
        }
        assert_eq!(
            prop(&tree, "psci", "compatible"),
            Some(&b"arm,psci-1.0\0arm,psci-0.2\0"[..])
        );
        assert_eq!(prop(&tree, "psci", "method"), Some(&b"hvc\0"[..]));
    }
}

/// `/chosen` carries the command line and the initrd's range when there
/// are any, and never an `rng-seed` or `kaslr-seed`, which a guest would
/// credit as entropy though a deterministic machine gives every boot the
/// same bytes.
#[test]
fn chosen_carries_the_boot_and_no_entropy() {
    let tree = plain();
    assert_eq!(prop(&tree, "chosen", "bootargs"), None);
    assert_eq!(prop(&tree, "chosen", "linux,initrd-start"), None);

    let tree = board(
        &BoardIdentity::RUSTY_BOX,
        GicExtensions::Absent,
        Chosen {
            bootargs: Some("console=ttyAMA0"),
            initrd: Some(InitrdRange {
                start: PhysAddr::new(0x4800_0000),
                end: PhysAddr::new(0x4812_3456),
            }),
        },
    );
    for seed in ["rng-seed", "kaslr-seed"] {
        assert_eq!(prop(&tree, "chosen", seed), None, "{seed}");
    }
    assert_eq!(
        prop(&tree, "chosen", "bootargs"),
        Some(&b"console=ttyAMA0\0"[..])
    );
    assert_eq!(
        prop(&tree, "chosen", "linux,initrd-start").map(cells),
        Some(vec![0, 0x4800_0000])
    );
    assert_eq!(
        prop(&tree, "chosen", "linux,initrd-end").map(cells),
        Some(vec![0, 0x4812_3456])
    );
}

/// Each device the board has is where its node says, raising what its
/// node says: the PrimeCells, the power button on GPIO line 3, the flash
/// banks, every virtio transport by level, and the PCIe host whose node is
/// named for its ECAM window, its INTx swizzled across the bridge's SPIs.
#[test]
fn the_devices_are_described_where_they_are() {
    let tree = plain();
    assert_eq!(
        prop(&tree, "rtc@9010000", "interrupts").map(cells),
        Some(vec![0, 2, 4])
    );
    assert_eq!(
        prop(&tree, "gpio@9030000", "interrupts").map(cells),
        Some(vec![0, 7, 4])
    );
    let gpio = prop(&tree, "gpio@9030000", "phandle").map(cells).expect("phandle");
    assert_eq!(
        prop(&tree, "gpio-keys/key-power", "gpios").map(cells),
        Some(vec![gpio[0], 3, 0])
    );
    assert_eq!(
        prop(&tree, "gpio-keys/key-power", "linux,code").map(cells),
        Some(vec![116])
    );
    assert_eq!(
        prop(&tree, "flash@0", "reg").map(cells),
        Some(vec![0, 0, 0, 0x0400_0000, 0, 0x0400_0000, 0, 0x0400_0000])
    );
    for (n, node) in [(0, "virtio@a000000"), (31, "virtio@a003e00")] {
        assert_eq!(
            prop(&tree, node, "interrupts").map(cells),
            Some(vec![0, 16 + n, 4]),
            "{node}"
        );
        assert_eq!(prop(&tree, node, "dma-coherent"), Some(&[][..]), "{node}");
    }
    // Coherence is each device's own (Devicetree Specification 2.3), so
    // the root does not say it for them.
    assert_eq!(prop(&tree, "", "dma-coherent"), None);
    assert_eq!(prop(&tree, "serial@9000000", "dma-coherent"), None);
    let pcie = "pcie@4010000000";
    assert_eq!(
        prop(&tree, pcie, "reg").map(cells),
        Some(vec![0x40, 0x1000_0000, 0, 0x1000_0000])
    );
    assert_eq!(prop(&tree, pcie, "bus-range").map(cells), Some(vec![0, 255]));
    // I/O space from 0 at 0x3EFF_0000, 64 KiB; 32-bit memory at
    // 0x1000_0000; 64-bit memory at 512 GiB, 512 GiB of it.
    assert_eq!(
        prop(&tree, pcie, "ranges").map(cells),
        Some(vec![
            0x0100_0000, 0, 0, 0, 0x3EFF_0000, 0, 0x1_0000, //
            0x0200_0000, 0, 0x1000_0000, 0, 0x1000_0000, 0, 0x2EFF_0000, //
            0x0300_0000, 0x80, 0, 0x80, 0, 0x80, 0,
        ])
    );
    assert_eq!(prop(&tree, pcie, "dma-coherent"), Some(&[][..]));
    let msi = prop(&tree, "interrupt-controller@8000000/msi-controller@8020000", "phandle");
    assert_eq!(prop(&tree, pcie, "msi-parent"), msi, "the GICv2m frame");
    assert_eq!(prop(&tree, pcie, "msi-map"), None);
    let map = prop(&tree, pcie, "interrupt-map").map(cells).expect("a map");
    assert_eq!(map.len(), 16 * 10);
    // Device 1's INTA is the bridge's second line, device 3's INTD its
    // third: (device + pin - 1) mod 4.
    let spi = |device: u32, pin: u32| map[((device * 4 + pin - 1) * 10 + 8) as usize];
    assert_eq!([spi(0, 1), spi(1, 1), spi(3, 4), spi(2, 3)], [3, 4, 5, 3]);
}

/// The board's tree, with EL2 its largest, uses its property names well
/// within the writer's table, so the table filling is a test failing here
/// and never a machine refusing to build.
#[test]
fn the_tree_leaves_room_in_the_name_table() {
    let tree = board(
        &BoardIdentity::RUSTY_BOX,
        GicExtensions::Virtualization,
        Chosen {
            bootargs: Some("console=ttyAMA0"),
            initrd: Some(InitrdRange {
                start: PhysAddr::new(0x4800_0000),
                end: PhysAddr::new(0x4800_1000),
            }),
        },
    );
    let word = |at: usize| u32::from_be_bytes(tree[at..at + 4].try_into().expect("four bytes"));
    let (at, len) = (word(12) as usize, word(32) as usize);
    let names = tree[at..at + len].iter().filter(|&&b| b == 0).count();
    assert!(
        names + 8 <= super::MAX_NAMES,
        "{names} names of {}",
        super::MAX_NAMES
    );
}

/// A tree that does not fit is refused, and a buffer just large enough
/// holds it.
#[test]
fn a_tree_that_does_not_fit_is_refused() {
    let len = plain().len();
    let tree = BoardTree {
        identity: &BoardIdentity::RUSTY_BOX,
        ram_len: RAM,
        cpu: CpuModel::CortexA57,
        psci: PsciConduit::Hvc,
        gic: GicExtensions::Absent,
        chosen: PLAIN,
    };
    let mut short = vec![0u8; len - 1];
    assert_eq!(
        write(&tree, &mut short),
        Err(crate::boot::DeviceTreeError::TooLarge)
    );
    let mut exact = vec![0u8; len];
    assert_eq!(write(&tree, &mut exact), Ok(len));
}
