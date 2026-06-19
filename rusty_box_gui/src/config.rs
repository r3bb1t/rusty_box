use crate::args::{Args, BootDevice, DiskGeometry, DisplayBackend, LogLevel};
use crate::error::RunError;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const DEFAULT_CONFIG_FILE: &str = "rusty_box.toml";

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub emulator: EmulatorToml,
    pub display: DisplayToml,
    pub rom: RomToml,
    pub boot: BootToml,
    pub disk: Option<DiskToml>,
    pub cdrom: Option<CdromToml>,
    pub logging: LoggingToml,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmulatorToml {
    pub memory_mib: Option<u32>,
    pub host_memory_mib: Option<u32>,
    pub memory_block_kib: Option<u32>,
    pub ips: Option<u32>,
    pub pci: Option<bool>,
    pub sync_slowdown: Option<bool>,
    pub max_instructions: Option<u64>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayToml {
    pub backend: Option<DisplayBackend>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RomToml {
    pub bios: Option<PathBuf>,
    pub vga_bios: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BootToml {
    pub order: Vec<BootDevice>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiskToml {
    pub path: Option<PathBuf>,
    pub chs: Option<DiskGeometry>,
    pub channel: Option<usize>,
    pub drive: Option<usize>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CdromToml {
    pub path: Option<PathBuf>,
    pub channel: Option<usize>,
    pub drive: Option<usize>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingToml {
    pub level: Option<LogLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub memory_mib: u32,
    pub host_memory_mib: u32,
    pub memory_block_kib: u32,
    pub ips: u32,
    pub pci: bool,
    pub sync_slowdown: bool,
    pub max_instructions: u64,
    pub display: DisplayBackend,
    pub bios: PathBuf,
    pub vga_bios: Option<PathBuf>,
    pub boot_order: Vec<BootDevice>,
    pub disk: Option<ResolvedDisk>,
    pub cdrom: Option<ResolvedCdrom>,
    pub log_level: LogLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDisk {
    pub path: PathBuf,
    pub geometry: DiskGeometry,
    pub channel: usize,
    pub drive: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCdrom {
    pub path: PathBuf,
    pub channel: usize,
    pub drive: usize,
}

pub fn load_config(args: &Args) -> Result<ResolvedConfig, RunError> {
    let file = if args.no_config {
        FileConfig::default()
    } else if let Some(path) = &args.config {
        load_toml_file(path)?
    } else {
        let default_path = Path::new(DEFAULT_CONFIG_FILE);
        if default_path.exists() {
            load_toml_file(default_path)?
        } else {
            FileConfig::default()
        }
    };

    resolve_config(file, args)
}

pub fn resolve_config(file: FileConfig, args: &Args) -> Result<ResolvedConfig, RunError> {
    let memory_mib = args.memory_mib.or(file.emulator.memory_mib).unwrap_or(32);
    ensure_nonzero("memory_mib", memory_mib)?;

    let host_memory_mib = args
        .host_memory_mib
        .or(file.emulator.host_memory_mib)
        .unwrap_or(memory_mib);
    ensure_nonzero("host_memory_mib", host_memory_mib)?;

    let memory_block_kib = args
        .memory_block_kib
        .or(file.emulator.memory_block_kib)
        .unwrap_or(128);
    ensure_nonzero("memory_block_kib", memory_block_kib)?;

    let ips = args.ips.or(file.emulator.ips).unwrap_or(4_000_000);
    ensure_nonzero("ips", ips)?;

    let pci = if args.pci {
        true
    } else if args.no_pci {
        false
    } else {
        file.emulator.pci.unwrap_or(true)
    };
    let sync_slowdown = if args.sync_slowdown {
        true
    } else if args.no_sync_slowdown {
        false
    } else {
        file.emulator.sync_slowdown.unwrap_or(false)
    };
    let max_instructions = args
        .max_instructions
        .or(file.emulator.max_instructions)
        .unwrap_or(u64::MAX);
    let display = args
        .display
        .or(file.display.backend)
        .unwrap_or(DisplayBackend::Terminal);
    let log_level = args
        .log_level
        .or(file.logging.level)
        .unwrap_or(LogLevel::Warn);
    let bios = args
        .bios
        .clone()
        .or_else(|| file.rom.bios.clone())
        .ok_or(RunError::MissingBios)?;
    let vga_bios = args.vga_bios.clone().or_else(|| file.rom.vga_bios.clone());

    let disk = resolve_disk(&file, args)?;
    let cdrom = resolve_cdrom(&file, args);
    let boot_order = resolve_boot_order(&file, args, disk.is_some(), cdrom.is_some())?;
    validate_boot_order(&boot_order, disk.is_some(), cdrom.is_some())?;

    Ok(ResolvedConfig {
        memory_mib,
        host_memory_mib,
        memory_block_kib,
        ips,
        pci,
        sync_slowdown,
        max_instructions,
        display,
        bios,
        vga_bios,
        boot_order,
        disk,
        cdrom,
        log_level,
    })
}

pub fn load_toml_file(path: &Path) -> Result<FileConfig, RunError> {
    let contents = fs::read_to_string(path).map_err(|source| RunError::ConfigRead {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&contents).map_err(|source| RunError::ConfigParse {
        path: path.to_owned(),
        source,
    })
}

fn resolve_disk(file: &FileConfig, args: &Args) -> Result<Option<ResolvedDisk>, RunError> {
    let Some(path) = args
        .disk
        .clone()
        .or_else(|| file.disk.as_ref().and_then(|disk| disk.path.clone()))
    else {
        return Ok(None);
    };

    let geometry = match args
        .disk_chs
        .or_else(|| file.disk.as_ref().and_then(|disk| disk.chs))
    {
        Some(geometry) => {
            validate_disk_geometry(geometry)?;
            geometry
        }
        None => auto_detect_chs(&path)?,
    };
    let channel = file
        .disk
        .as_ref()
        .and_then(|disk| disk.channel)
        .unwrap_or(0);
    let drive = file.disk.as_ref().and_then(|disk| disk.drive).unwrap_or(0);

    Ok(Some(ResolvedDisk {
        path,
        geometry,
        channel,
        drive,
    }))
}

fn resolve_cdrom(file: &FileConfig, args: &Args) -> Option<ResolvedCdrom> {
    let path = args
        .cdrom
        .clone()
        .or_else(|| file.cdrom.as_ref().and_then(|cdrom| cdrom.path.clone()))?;
    let channel = file
        .cdrom
        .as_ref()
        .and_then(|cdrom| cdrom.channel)
        .unwrap_or(1);
    let drive = file
        .cdrom
        .as_ref()
        .and_then(|cdrom| cdrom.drive)
        .unwrap_or(0);

    Some(ResolvedCdrom {
        path,
        channel,
        drive,
    })
}

fn resolve_boot_order(
    file: &FileConfig,
    args: &Args,
    has_disk: bool,
    has_cdrom: bool,
) -> Result<Vec<BootDevice>, RunError> {
    let order = if !args.boot.is_empty() {
        args.boot.clone()
    } else if !file.boot.order.is_empty() {
        file.boot.order.clone()
    } else if has_disk {
        vec![BootDevice::Disk]
    } else if has_cdrom {
        vec![BootDevice::Cdrom]
    } else {
        return Err(RunError::EmptyBootOrder);
    };

    if order.len() > 3 {
        return Err(RunError::TooManyBootDevices);
    }

    Ok(order)
}

fn validate_boot_order(
    boot_order: &[BootDevice],
    has_disk: bool,
    has_cdrom: bool,
) -> Result<(), RunError> {
    for (index, device) in boot_order.iter().enumerate() {
        if boot_order[index + 1..].contains(device) {
            return Err(RunError::DuplicateBootDevice { device: *device });
        }
    }

    for device in boot_order {
        match device {
            BootDevice::Disk if !has_disk => {
                return Err(RunError::MissingBootMedia {
                    device: *device,
                    field: "disk.path",
                });
            }
            BootDevice::Cdrom if !has_cdrom => {
                return Err(RunError::MissingBootMedia {
                    device: *device,
                    field: "cdrom.path",
                });
            }
            _ => {}
        }
    }

    Ok(())
}

fn validate_disk_geometry(geometry: DiskGeometry) -> Result<(), RunError> {
    if geometry.cylinders == 0 || geometry.heads == 0 || geometry.sectors_per_track == 0 {
        Err(RunError::ZeroValue { field: "disk.chs" })
    } else {
        Ok(())
    }
}

fn ensure_nonzero(field: &'static str, value: u32) -> Result<(), RunError> {
    if value == 0 {
        Err(RunError::ZeroValue { field })
    } else {
        Ok(())
    }
}

fn auto_detect_chs(path: &Path) -> Result<DiskGeometry, RunError> {
    const SECTOR_SIZE: u64 = 512;
    const HEADS: u64 = 16;
    const SECTORS_PER_TRACK: u64 = 63;
    const MAX_CYLINDERS: u64 = 16_383;

    let metadata = fs::metadata(path).map_err(|source| RunError::FileRead {
        kind: "disk",
        path: path.to_owned(),
        source: map_metadata_error(source),
    })?;
    let len = metadata.len();
    if len == 0 || len % SECTOR_SIZE != 0 {
        return Err(RunError::InvalidDiskSize {
            path: path.to_owned(),
            len,
        });
    }

    let total_sectors = len / SECTOR_SIZE;
    let sectors_per_cylinder = HEADS * SECTORS_PER_TRACK;
    let cylinders = total_sectors.div_ceil(sectors_per_cylinder);
    if cylinders > MAX_CYLINDERS {
        return Err(RunError::DiskTooLargeForChs {
            path: path.to_owned(),
            cylinders,
        });
    }

    Ok(DiskGeometry {
        cylinders: cylinders as u16,
        heads: HEADS as u8,
        sectors_per_track: SECTORS_PER_TRACK as u8,
    })
}

fn map_metadata_error(error: io::Error) -> io::Error {
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args<const N: usize>(values: [&str; N]) -> Args {
        Args::parse_from(values)
    }

    fn config(toml: &str) -> FileConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn cli_overrides_toml_without_losing_other_toml_values() {
        let file = config(
            r#"
[emulator]
memory_mib = 64

[rom]
bios = "old.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui", "--bios", "new.bin"])).unwrap();

        assert_eq!(resolved.bios, PathBuf::from("new.bin"));
        assert_eq!(resolved.memory_mib, 64);
    }

    #[test]
    fn infers_disk_boot_when_disk_present() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();

        assert_eq!(resolved.boot_order, vec![BootDevice::Disk]);
    }

    #[test]
    fn rejects_duplicate_boot_devices() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[boot]
order = ["disk", "disk"]

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(
            error,
            RunError::DuplicateBootDevice {
                device: BootDevice::Disk
            }
        ));
    }

    #[test]
    fn rejects_zero_toml_disk_geometry() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 0, heads = 4, sectors_per_track = 17 }
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::ZeroValue { field: "disk.chs" }));
    }

    #[test]
    fn rejects_missing_bios() {
        let error = resolve_config(FileConfig::default(), &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::MissingBios));
    }
}
