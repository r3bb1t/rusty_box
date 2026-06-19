use clap::Parser;
use rusty_box_gui::{Args, DiskGeometry, DisplayBackend};

#[test]
fn parses_core_runner_flags() {
    let args = Args::try_parse_from([
        "rusty_box_gui",
        "--no-config",
        "--display",
        "headless",
        "--bios",
        "bios.bin",
        "--disk",
        "disk.img",
        "--disk-chs",
        "306:4:17",
        "--boot",
        "disk",
        "--memory-mib",
        "64",
        "--ips",
        "15000000",
        "--max-instructions",
        "1000",
    ])
    .unwrap();

    assert!(args.no_config);
    assert_eq!(args.display, Some(DisplayBackend::Headless));
    assert_eq!(args.bios.unwrap().as_os_str(), "bios.bin");
    assert_eq!(args.disk.unwrap().as_os_str(), "disk.img");
    assert_eq!(
        args.disk_chs,
        Some(DiskGeometry {
            cylinders: 306,
            heads: 4,
            sectors_per_track: 17,
        })
    );
    assert_eq!(args.memory_mib, Some(64));
    assert_eq!(args.ips, Some(15_000_000));
    assert_eq!(args.max_instructions, Some(1000));
}

#[test]
fn config_and_no_config_conflict() {
    let result = Args::try_parse_from(["rusty_box_gui", "--config", "a.toml", "--no-config"]);

    assert!(result.is_err());
}

#[test]
fn pci_negation_is_explicit() {
    let args = Args::try_parse_from(["rusty_box_gui", "--no-pci"]).unwrap();

    assert!(args.no_pci);
    assert!(!args.pci);
}
