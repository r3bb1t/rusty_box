use crate::{
    args::{Args, BootDevice, DisplayBackend, LogLevel},
    config::ResolvedConfig,
    error::RunError,
};
use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::{NoGui, TermGui},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSummary {
    pub instructions_executed: u64,
}

pub fn run(args: Args) -> Result<RunSummary, RunError> {
    let config = crate::config::load_config(&args)?;
    run_resolved(config)
}

pub fn run_resolved(config: ResolvedConfig) -> Result<RunSummary, RunError> {
    init_tracing(config.log_level);

    let bios_data = read_required_file("BIOS", &config.bios)?;
    let vga_data = match &config.vga_bios {
        Some(path) => Some(read_vga_bios_file(path)?),
        None => None,
    };
    if let Some(disk) = &config.disk {
        verify_media_file("disk", &disk.path)?;
    }
    if let Some(cdrom) = &config.cdrom {
        verify_media_file("CD-ROM", &cdrom.path)?;
    }
    validate_configured_media_slots(&config)?;

    let emulator_config = EmulatorConfig {
        guest_memory_size: mib_to_bytes("memory_mib", config.memory_mib)?,
        host_memory_size: mib_to_bytes("host_memory_mib", config.host_memory_mib)?,
        memory_block_size: kib_to_bytes("memory_block_kib", config.memory_block_kib)?,
        ips: config.ips,
        pci_enabled: config.pci,
        sync_slowdown: config.sync_slowdown,
        ..EmulatorConfig::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX>::new(emulator_config)?;
    match config.display {
        DisplayBackend::Headless => emu.set_gui(NoGui::new()),
        DisplayBackend::Terminal => emu.set_gui(TermGui::new()),
    }

    emu.init_memory_and_pc_system()?;
    let bios_load_addr = !(bios_data.len() as u64 - 1);
    emu.load_bios(&bios_data, bios_load_addr)?;
    if let Some(data) = &vga_data {
        emu.load_optional_rom(data, 0xC0000)?;
    }
    emu.init_cpu_and_devices()?;
    emu.configure_memory_in_cmos_from_config();
    let (first_boot, second_boot, third_boot) = boot_sequence(&config.boot_order);
    emu.configure_boot_sequence(first_boot, second_boot, third_boot);

    if let Some(disk) = &config.disk {
        let geometry = disk.geometry;
        emu.configure_disk_geometry_in_cmos(
            disk_cmos_drive(disk),
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        );
        let disk_path = path_to_str(&disk.path)?;
        emu.attach_disk(
            disk.channel,
            disk.drive,
            disk_path,
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track,
        )
        .map_err(|source| RunError::MediaAttach {
            kind: "disk",
            path: disk.path.clone(),
            source,
        })?;
    }

    if let Some(cdrom) = &config.cdrom {
        let cdrom_path = path_to_str(&cdrom.path)?;
        emu.attach_cdrom(cdrom.channel, cdrom.drive, cdrom_path)
            .map_err(|source| RunError::MediaAttach {
                kind: "CD-ROM",
                path: cdrom.path.clone(),
                source,
            })?;
    }

    emu.init_gui(0, &[])?;
    emu.reset(ResetReason::Hardware)?;
    emu.init_gui_signal_handlers();
    emu.start();
    let instructions_executed = emu.run_interactive(config.max_instructions)?;

    Ok(RunSummary {
        instructions_executed,
    })
}

fn init_tracing(log_level: LogLevel) {
    let _ = tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .with_max_level(match log_level {
            LogLevel::Trace => tracing::Level::TRACE,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Error => tracing::Level::ERROR,
        })
        .try_init();
}

fn read_required_file(kind: &'static str, path: &Path) -> Result<Vec<u8>, RunError> {
    let data = fs::read(path).map_err(|source| RunError::FileRead {
        kind,
        path: path.to_owned(),
        source,
    })?;
    if data.is_empty() {
        return Err(RunError::EmptyFile {
            kind,
            path: path.to_owned(),
        });
    }
    Ok(data)
}

fn read_vga_bios_file(path: &Path) -> Result<Vec<u8>, RunError> {
    let data = fs::read(path).map_err(|source| RunError::FileRead {
        kind: "VGA BIOS",
        path: path.to_owned(),
        source,
    })?;
    if data.is_empty() || data.len() % 512 != 0 {
        return Err(RunError::InvalidVgaBiosSize {
            path: path.to_owned(),
            len: data.len(),
        });
    }
    Ok(data)
}

fn verify_media_file(kind: &'static str, path: &Path) -> Result<(), RunError> {
    let metadata = fs::metadata(path).map_err(|source| RunError::FileRead {
        kind,
        path: path.to_owned(),
        source,
    })?;
    if metadata.len() == 0 {
        return Err(RunError::EmptyFile {
            kind,
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn mib_to_bytes(field: &'static str, mib: u32) -> Result<usize, RunError> {
    bytes_from_units(field, mib, 1024 * 1024)
}

fn kib_to_bytes(field: &'static str, kib: u32) -> Result<usize, RunError> {
    bytes_from_units(field, kib, 1024)
}

fn bytes_from_units(field: &'static str, value: u32, scale: u64) -> Result<usize, RunError> {
    let bytes = u64::from(value)
        .checked_mul(scale)
        .ok_or(RunError::ValueOverflow { field })?;
    usize::try_from(bytes).map_err(|_| RunError::ValueOverflow { field })
}

fn boot_sequence(boot_order: &[BootDevice]) -> (u8, u8, u8) {
    let mut values = [0; 3];
    for (index, device) in boot_order.iter().take(3).enumerate() {
        values[index] = match device {
            BootDevice::Disk => 2,
            BootDevice::Cdrom => 3,
        };
    }
    (values[0], values[1], values[2])
}

fn validate_configured_media_slots(config: &ResolvedConfig) -> Result<(), RunError> {
    if let Some(disk) = &config.disk {
        validate_ata_slot("disk.channel", disk.channel)?;
        validate_ata_slot("disk.drive", disk.drive)?;
        if let Some(cdrom) = &config.cdrom {
            validate_distinct_media_slots(disk, cdrom)?;
        }
    }
    if let Some(cdrom) = &config.cdrom {
        validate_ata_slot("cdrom.channel", cdrom.channel)?;
        validate_ata_slot("cdrom.drive", cdrom.drive)?;
    }
    Ok(())
}

fn validate_distinct_media_slots(
    disk: &crate::config::ResolvedDisk,
    cdrom: &crate::config::ResolvedCdrom,
) -> Result<(), RunError> {
    if disk.channel == cdrom.channel && disk.drive == cdrom.drive {
        Err(RunError::MediaAttach {
            kind: "CD-ROM",
            path: cdrom.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "ATA slot already used by disk"),
        })
    } else {
        Ok(())
    }
}

fn validate_ata_slot(field: &'static str, value: usize) -> Result<(), RunError> {
    if value > 1 {
        Err(RunError::ValueOverflow { field })
    } else {
        Ok(())
    }
}

fn disk_cmos_drive(disk: &crate::config::ResolvedDisk) -> u8 {
    disk.drive as u8
}

fn path_to_str(path: &PathBuf) -> Result<&str, RunError> {
    path.to_str()
        .ok_or_else(|| RunError::NonUtf8Path { path: path.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::DiskGeometry;
    use crate::config::{ResolvedCdrom, ResolvedDisk};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn empty_vga_bios_returns_vga_size_error() {
        let dir = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let bios = dir.join(format!("rusty_box_gui_bios_{suffix}.bin"));
        let vga = dir.join(format!("rusty_box_gui_vga_{suffix}.bin"));
        fs::write(&bios, [0xEA]).unwrap();
        fs::write(&vga, []).unwrap();

        let error = run_resolved(ResolvedConfig {
            memory_mib: 32,
            host_memory_mib: 32,
            memory_block_kib: 128,
            ips: 4_000_000,
            pci: true,
            sync_slowdown: false,
            max_instructions: 0,
            display: DisplayBackend::Headless,
            bios: bios.clone(),
            vga_bios: Some(vga.clone()),
            boot_order: Vec::new(),
            disk: None::<ResolvedDisk>,
            cdrom: None::<ResolvedCdrom>,
            log_level: LogLevel::Warn,
        })
        .unwrap_err();

        let _ = fs::remove_file(&bios);
        let _ = fs::remove_file(&vga);

        assert!(matches!(
            error,
            RunError::InvalidVgaBiosSize { path, len: 0 } if path == vga
        ));
    }

    #[test]
    fn boot_sequence_pads_missing_devices_with_zero() {
        assert_eq!(boot_sequence(&[BootDevice::Disk]), (2, 0, 0));
        assert_eq!(
            boot_sequence(&[BootDevice::Cdrom, BootDevice::Disk]),
            (3, 2, 0)
        );
    }

    #[test]
    fn rejects_out_of_range_ata_slots() {
        assert!(matches!(
            validate_ata_slot("disk.channel", 2),
            Err(RunError::ValueOverflow {
                field: "disk.channel"
            })
        ));
        assert!(matches!(
            validate_ata_slot("cdrom.drive", 2),
            Err(RunError::ValueOverflow {
                field: "cdrom.drive"
            })
        ));
    }

    #[test]
    fn cmos_drive_uses_configured_disk_drive() {
        let disk = ResolvedDisk {
            path: PathBuf::from("disk.img"),
            geometry: DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            },
            channel: 0,
            drive: 1,
        };

        assert_eq!(disk_cmos_drive(&disk), 1);
    }

    #[test]
    fn rejects_disk_cdrom_same_ata_slot() {
        let disk = ResolvedDisk {
            path: PathBuf::from("disk.img"),
            geometry: DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            },
            channel: 0,
            drive: 0,
        };
        let cdrom = ResolvedCdrom {
            path: PathBuf::from("cdrom.iso"),
            channel: 0,
            drive: 0,
        };

        let error = validate_distinct_media_slots(&disk, &cdrom).unwrap_err();

        assert!(matches!(
            error,
            RunError::MediaAttach { kind: "CD-ROM", path, source }
                if path == PathBuf::from("cdrom.iso")
                    && source.kind() == std::io::ErrorKind::InvalidInput
        ));
    }
}
