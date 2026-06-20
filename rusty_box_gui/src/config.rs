use crate::args::{Args, BootDevice, DiskGeometry, DisplayBackend, LogLevel};
use crate::error::RunError;
use std::{
    env, fs, io,
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
    pub create: Option<DiskCreateToml>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiskCreateToml {
    pub path: Option<PathBuf>,
    pub size: Option<String>,
    pub overwrite: Option<bool>,
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
pub struct ResolvedDiskCreation {
    pub path: PathBuf,
    pub size: rusty_box_bximage::ImageSize,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDisk {
    pub path: PathBuf,
    pub geometry: DiskGeometry,
    pub channel: usize,
    pub drive: usize,
    pub creation: Option<ResolvedDiskCreation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCdrom {
    pub path: PathBuf,
    pub channel: usize,
    pub drive: usize,
}

pub fn load_config(args: &Args) -> Result<ResolvedConfig, RunError> {
    let (file, config_dir) = if args.no_config {
        (FileConfig::default(), None)
    } else if let Some(path) = &args.config {
        (load_toml_file(path)?, path.parent().map(Path::to_path_buf))
    } else {
        if let Some(default_path) = env::current_dir()
            .ok()
            .and_then(|cwd| find_default_config_file(&cwd))
        {
            (
                load_toml_file(&default_path)?,
                default_path.parent().map(Path::to_path_buf),
            )
        } else {
            (FileConfig::default(), None)
        }
    };

    resolve_config_with_base(file, args, config_dir.as_deref())
}

fn find_default_config_file(start_dir: &Path) -> Option<PathBuf> {
    let local = start_dir.join(DEFAULT_CONFIG_FILE);
    if local.exists() {
        return Some(local);
    }

    let parent = start_dir.parent()?.join(DEFAULT_CONFIG_FILE);
    parent.exists().then_some(parent)
}

pub fn resolve_config(file: FileConfig, args: &Args) -> Result<ResolvedConfig, RunError> {
    resolve_config_with_base(file, args, None)
}

fn resolve_config_with_base(
    file: FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
) -> Result<ResolvedConfig, RunError> {
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
        .unwrap_or_else(default_display_backend);
    let log_level = args
        .log_level
        .or(file.logging.level)
        .unwrap_or(LogLevel::Warn);
    let bios = match args.bios.clone() {
        Some(path) => path,
        None => file
            .rom
            .bios
            .clone()
            .map(|path| resolve_toml_path(config_dir, path))
            .ok_or(RunError::MissingBios)?,
    };
    let vga_bios = match args.vga_bios.clone() {
        Some(path) => Some(path),
        None => file
            .rom
            .vga_bios
            .clone()
            .map(|path| resolve_toml_path(config_dir, path)),
    };

    let disk = resolve_disk(&file, args, config_dir)?;
    let cdrom = resolve_cdrom(&file, args, config_dir);
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

fn resolve_toml_path(config_dir: Option<&Path>, path: PathBuf) -> PathBuf {
    if path.is_relative() {
        config_dir.map_or(path.clone(), |dir| dir.join(path))
    } else {
        path
    }
}

fn resolve_disk(
    file: &FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
) -> Result<Option<ResolvedDisk>, RunError> {
    let disk_toml = file.disk.as_ref();
    let toml_create = disk_toml.and_then(|disk| disk.create.as_ref());
    let has_cli_create = args.create_disk.is_some()
        || args.create_disk_size.is_some()
        || args.overwrite_created_disk;

    if args.disk.is_some() {
        return resolve_existing_disk(disk_toml, args, config_dir);
    }

    if has_cli_create || toml_create.is_some() {
        return resolve_created_disk(disk_toml, toml_create, args, has_cli_create, config_dir);
    }

    if let Some(disk) = disk_toml {
        if disk.path.is_some() && disk.create.is_some() {
            return Err(RunError::ConflictingDiskOptions {
                first: "disk.path",
                second: "disk.create",
            });
        }
    }

    resolve_existing_disk(disk_toml, args, config_dir)
}

fn resolve_existing_disk(
    disk_toml: Option<&DiskToml>,
    args: &Args,
    config_dir: Option<&Path>,
) -> Result<Option<ResolvedDisk>, RunError> {
    let path = if let Some(path) = args.disk.clone() {
        path
    } else if let Some(path) = disk_toml.and_then(|disk| disk.path.clone()) {
        resolve_toml_path(config_dir, path)
    } else {
        return Ok(None);
    };

    let geometry = match args
        .disk_chs
        .or_else(|| disk_toml.and_then(|disk| disk.chs))
    {
        Some(geometry) => {
            validate_disk_geometry(geometry)?;
            geometry
        }
        None => auto_detect_chs(&path)?,
    };
    let channel = disk_toml.and_then(|disk| disk.channel).unwrap_or(0);
    let drive = disk_toml.and_then(|disk| disk.drive).unwrap_or(0);

    Ok(Some(ResolvedDisk {
        path,
        geometry,
        channel,
        drive,
        creation: None,
    }))
}

fn resolve_created_disk(
    disk_toml: Option<&DiskToml>,
    toml_create: Option<&DiskCreateToml>,
    args: &Args,
    has_cli_create: bool,
    config_dir: Option<&Path>,
) -> Result<Option<ResolvedDisk>, RunError> {
    if !has_cli_create && disk_toml.and_then(|disk| disk.path.as_ref()).is_some() {
        return Err(RunError::ConflictingDiskOptions {
            first: "disk.path",
            second: "disk.create",
        });
    }
    if args.disk_chs.is_some() || (!has_cli_create && disk_toml.and_then(|disk| disk.chs).is_some())
    {
        return Err(RunError::ConflictingDiskOptions {
            first: "disk.chs",
            second: "disk.create",
        });
    }

    let path = if let Some(path) = args.create_disk.clone() {
        path
    } else if let Some(path) = toml_create.and_then(|create| create.path.clone()) {
        resolve_toml_path(config_dir, path)
    } else {
        return Err(RunError::MissingDiskCreatePath);
    };
    let size = match args.create_disk_size {
        Some(size) => size.0,
        None => match toml_create.and_then(|create| create.size.as_deref()) {
            Some(size) => rusty_box_bximage::ImageSize::parse(size)
                .map_err(|source| RunError::DiskCreateSize { source })?,
            None => rusty_box_bximage::DEFAULT_HARD_DISK_SIZE,
        },
    };
    let overwrite = args.overwrite_created_disk
        || toml_create
            .and_then(|create| create.overwrite)
            .unwrap_or(false);

    let geometry = rusty_box_bximage::calculate_hard_disk_geometry(
        size,
        rusty_box_bximage::SectorSize::Bytes512,
    )
    .map_err(|source| RunError::DiskCreateSize { source })?;
    if geometry.cylinders > u16::MAX as u64 {
        return Err(RunError::CreatedDiskChsOverflow {
            path,
            cylinders: geometry.cylinders,
        });
    }

    let channel = disk_toml.and_then(|disk| disk.channel).unwrap_or(0);
    let drive = disk_toml.and_then(|disk| disk.drive).unwrap_or(0);
    let geometry = DiskGeometry {
        cylinders: geometry.cylinders as u16,
        heads: geometry.heads as u8,
        sectors_per_track: geometry.sectors_per_track as u8,
    };

    Ok(Some(ResolvedDisk {
        path: path.clone(),
        geometry,
        channel,
        drive,
        creation: Some(ResolvedDiskCreation {
            path,
            size,
            overwrite,
        }),
    }))
}

fn resolve_cdrom(
    file: &FileConfig,
    args: &Args,
    config_dir: Option<&Path>,
) -> Option<ResolvedCdrom> {
    let path = match args.cdrom.clone() {
        Some(path) => path,
        None => resolve_toml_path(config_dir, file.cdrom.as_ref()?.path.clone()?),
    };
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

#[cfg(feature = "gui-egui")]
fn default_display_backend() -> DisplayBackend {
    DisplayBackend::Egui
}

#[cfg(not(feature = "gui-egui"))]
fn default_display_backend() -> DisplayBackend {
    DisplayBackend::Terminal
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

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn detect_disk_geometry(path: &Path) -> Result<DiskGeometry, RunError> {
    auto_detect_chs(path)
}

fn map_metadata_error(error: io::Error) -> io::Error {
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rusty_box_bximage::ImageSize;

    fn args<const N: usize>(values: [&str; N]) -> Args {
        Args::parse_from(values)
    }

    fn config(toml: &str) -> FileConfig {
        toml::from_str(toml).unwrap()
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()))
    }

    fn remove_test_file(path: &Path) {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove {}: {error}", path.display()),
        }
    }

    fn remove_test_dir(path: &Path) {
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove {}: {error}", path.display()),
        }
    }

    #[test]
    fn finds_parent_default_config_when_run_from_crate_dir() {
        let parent = unique_temp_dir("rusty-box-gui-parent-config");
        let child = parent.join("rusty_box_gui");
        fs::create_dir_all(&child).unwrap();
        let config_path = parent.join(DEFAULT_CONFIG_FILE);
        fs::write(&config_path, "[rom]\nbios = \"bios.bin\"\n").unwrap();

        let found = find_default_config_file(&child);

        assert_eq!(found, Some(config_path.clone()));
        remove_test_file(&config_path);
        remove_test_dir(&child);
        remove_test_dir(&parent);
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
    fn resolves_toml_created_disk_with_vmware_like_default() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
        let disk = resolved.disk.unwrap();
        let creation = disk.creation.unwrap();

        assert_eq!(disk.path, PathBuf::from("c.img"));
        assert_eq!(
            disk.geometry,
            DiskGeometry {
                cylinders: 41_610,
                heads: 16,
                sectors_per_track: 63,
            }
        );
        assert_eq!(creation.size, ImageSize::gib(20));
        assert!(!creation.overwrite);
        assert_eq!(resolved.boot_order, vec![BootDevice::Disk]);
    }

    #[test]
    fn resolves_toml_created_disk_size_string() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
size = "512M"
"#,
        );
        let resolved = resolve_config(file, &args(["rusty_box_gui"])).unwrap();
        let creation = resolved.disk.unwrap().creation.unwrap();

        assert_eq!(creation.size, ImageSize::mib(512));
    }

    #[test]
    fn cli_create_disk_overrides_toml_disk_path() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "old.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        );
        let resolved =
            resolve_config(file, &args(["rusty_box_gui", "--create-disk", "new.img"])).unwrap();
        let disk = resolved.disk.unwrap();

        assert_eq!(disk.path, PathBuf::from("new.img"));
        assert!(disk.creation.is_some());
    }

    #[test]
    fn rejects_toml_disk_path_and_create_without_cli_override() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "old.img"

[disk.create]
path = "new.img"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(
            error,
            RunError::ConflictingDiskOptions {
                first: "disk.path",
                second: "disk.create"
            }
        ));
    }

    #[test]
    fn rejects_create_size_without_path() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui", "--create-disk-size", "20G"]))
            .unwrap_err();

        assert!(matches!(error, RunError::MissingDiskCreatePath));
    }

    #[test]
    fn rejects_created_disk_that_exceeds_chs_api() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "huge.img"
size = "32G"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(
            error,
            RunError::CreatedDiskChsOverflow {
                path,
                cylinders: 66_576
            } if path == PathBuf::from("huge.img")
        ));
    }

    #[test]
    fn rejects_invalid_toml_create_size() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"

[disk.create]
path = "c.img"
size = "1.5G"
"#,
        );
        let error = resolve_config(file, &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::DiskCreateSize { .. }));
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

    #[cfg(feature = "gui-egui")]
    #[test]
    fn defaults_to_egui_display_when_feature_enabled() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"
"#,
        );
        let resolved = resolve_config(
            file,
            &args(["rusty_box_gui", "--boot", "cdrom", "--cdrom", "boot.iso"]),
        )
        .unwrap();

        assert_eq!(resolved.display, DisplayBackend::Egui);
    }

    #[cfg(feature = "gui-egui")]
    #[test]
    fn resolves_egui_display_backend_from_toml() {
        let file = config(
            r#"
[display]
backend = "egui"

[rom]
bios = "bios.bin"
"#,
        );
        let resolved = resolve_config(
            file,
            &args(["rusty_box_gui", "--boot", "cdrom", "--cdrom", "boot.iso"]),
        )
        .unwrap();

        assert_eq!(resolved.display, DisplayBackend::Egui);
    }

    #[cfg(not(feature = "gui-egui"))]
    #[test]
    fn defaults_to_terminal_display_when_egui_disabled() {
        let file = config(
            r#"
[rom]
bios = "bios.bin"
"#,
        );
        let resolved = resolve_config(
            file,
            &args(["rusty_box_gui", "--boot", "cdrom", "--cdrom", "boot.iso"]),
        )
        .unwrap();

        assert_eq!(resolved.display, DisplayBackend::Terminal);
    }

    #[test]
    fn config_relative_paths_resolve_from_toml_directory() {
        let dir = unique_temp_dir("rusty-box-gui-relative-config");
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("rusty_box.toml");
        fs::write(
            &config_path,
            r#"
[rom]
bios = "bios.bin"
vga_bios = "vgabios.bin"

[boot]
order = ["disk", "cdrom"]

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }

[cdrom]
path = "boot.iso"
"#,
        )
        .unwrap();

        let resolved = load_config(&args([
            "rusty_box_gui",
            "--config",
            config_path.to_str().unwrap(),
        ]))
        .unwrap();

        assert_eq!(resolved.bios, dir.join("bios.bin"));
        assert_eq!(resolved.vga_bios, Some(dir.join("vgabios.bin")));
        assert_eq!(
            resolved.disk.as_ref().map(|disk| disk.path.as_path()),
            Some(dir.join("disk.img").as_path())
        );
        assert_eq!(
            resolved.cdrom.as_ref().map(|cdrom| cdrom.path.as_path()),
            Some(dir.join("boot.iso").as_path())
        );

        remove_test_file(&config_path);
        remove_test_dir(&dir);
    }

    #[test]
    fn config_relative_paths_preserve_cli_override_cwd_semantics() {
        let dir = unique_temp_dir("rusty-box-gui-relative-cli");
        fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("rusty_box.toml");
        fs::write(
            &config_path,
            r#"
[rom]
bios = "bios.bin"

[disk]
path = "disk.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
"#,
        )
        .unwrap();

        let resolved = load_config(&args([
            "rusty_box_gui",
            "--config",
            config_path.to_str().unwrap(),
            "--bios",
            "new.bin",
            "--disk",
            "new.img",
            "--disk-chs",
            "306:4:17",
        ]))
        .unwrap();

        assert_eq!(resolved.bios, PathBuf::from("new.bin"));
        assert_eq!(
            resolved.disk.as_ref().map(|disk| disk.path.as_path()),
            Some(Path::new("new.img"))
        );

        remove_test_file(&config_path);
        remove_test_dir(&dir);
    }
    #[test]
    fn rejects_missing_bios() {
        let error = resolve_config(FileConfig::default(), &args(["rusty_box_gui"])).unwrap_err();

        assert!(matches!(error, RunError::MissingBios));
    }
}
