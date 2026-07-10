use clap::{ArgAction, Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, str::FromStr};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "rusty_box_gui",
    version,
    about = "Run Rusty Box with typed CLI flags and TOML config"
)]
pub struct Args {
    #[arg(
        short = 'f',
        long = "config",
        value_name = "TOML",
        conflicts_with = "no_config"
    )]
    pub config: Option<PathBuf>,

    #[arg(long = "no-config", action = ArgAction::SetTrue)]
    pub no_config: bool,

    #[arg(long = "bios", value_name = "PATH")]
    pub bios: Option<PathBuf>,

    #[arg(long = "vga-bios", value_name = "PATH")]
    pub vga_bios: Option<PathBuf>,

    #[arg(long = "display", value_enum)]
    pub display: Option<DisplayBackend>,

    #[arg(long = "boot", value_delimiter = ',', num_args = 1..=3, value_enum)]
    pub boot: Vec<BootDevice>,

    #[arg(long = "disk", value_name = "PATH")]
    pub disk: Option<PathBuf>,

    #[arg(long = "disk-chs", value_name = "CYLINDERS:HEADS:SPT")]
    pub disk_chs: Option<DiskGeometry>,

    #[arg(long = "create-disk", value_name = "PATH", conflicts_with_all = ["disk", "disk_chs"])]
    pub create_disk: Option<PathBuf>,

    #[arg(long = "create-disk-size", value_name = "SIZE", conflicts_with_all = ["disk", "disk_chs"])]
    pub create_disk_size: Option<HumanDiskSize>,

    #[arg(long = "overwrite-created-disk", action = ArgAction::SetTrue, conflicts_with_all = ["disk", "disk_chs"])]
    pub overwrite_created_disk: bool,

    #[arg(long = "cdrom", value_name = "PATH")]
    pub cdrom: Option<PathBuf>,

    #[arg(long = "memory-mib", value_name = "MIB")]
    pub memory_mib: Option<u32>,

    #[arg(long = "host-memory-mib", value_name = "MIB")]
    pub host_memory_mib: Option<u32>,

    #[arg(long = "memory-block-kib", value_name = "KIB")]
    pub memory_block_kib: Option<u32>,

    #[arg(long = "ips", value_name = "N")]
    pub ips: Option<u32>,

    #[arg(long = "max-instructions", value_name = "N")]
    pub max_instructions: Option<u64>,
    #[arg(long = "cpus", value_name = "N")]
    pub cpus: Option<u32>,

    #[arg(long = "cpu-sockets", value_name = "N", conflicts_with = "cpus")]
    pub cpu_sockets: Option<u32>,

    #[arg(long = "cpu-cores", value_name = "N", conflicts_with = "cpus")]
    pub cpu_cores: Option<u32>,

    #[arg(long = "cpu-threads", value_name = "N", conflicts_with = "cpus")]
    pub cpu_threads: Option<u32>,

    #[arg(long = "pci", action = ArgAction::SetTrue, conflicts_with = "no_pci")]
    pub pci: bool,

    #[arg(long = "no-pci", action = ArgAction::SetTrue)]
    pub no_pci: bool,

    #[arg(long = "sync-slowdown", action = ArgAction::SetTrue, conflicts_with = "no_sync_slowdown")]
    pub sync_slowdown: bool,

    #[arg(long = "no-sync-slowdown", action = ArgAction::SetTrue)]
    pub no_sync_slowdown: bool,

    #[arg(long = "log-level", value_enum)]
    pub log_level: Option<LogLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayBackend {
    Terminal,
    Headless,
    #[cfg(feature = "gui-egui")]
    Egui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BootDevice {
    Disk,
    Cdrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct DiskGeometry {
    pub cylinders: u16,
    pub heads: u8,
    pub sectors_per_track: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HumanDiskSize(pub rusty_box_bximage::ImageSize);

impl FromStr for HumanDiskSize {
    type Err = rusty_box_bximage::BxImageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        rusty_box_bximage::ImageSize::parse(value).map(Self)
    }
}

impl fmt::Display for BootDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Disk => "disk",
            Self::Cdrom => "cdrom",
        })
    }
}

impl fmt::Display for DiskGeometry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}",
            self.cylinders, self.heads, self.sectors_per_track
        )
    }
}

impl FromStr for DiskGeometry {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut parts = input.split([':', ',']);
        let cylinders = parts
            .next()
            .ok_or_else(wrong_disk_chs_part_count)?
            .parse::<u16>()
            .map_err(|error| error.to_string())?;
        let heads = parts
            .next()
            .ok_or_else(wrong_disk_chs_part_count)?
            .parse::<u8>()
            .map_err(|error| error.to_string())?;
        let sectors_per_track = parts
            .next()
            .ok_or_else(wrong_disk_chs_part_count)?
            .parse::<u8>()
            .map_err(|error| error.to_string())?;

        if parts.next().is_some() {
            return Err(wrong_disk_chs_part_count());
        }
        if cylinders == 0 || heads == 0 || sectors_per_track == 0 {
            return Err("disk CHS values must be non-zero".to_owned());
        }

        Ok(Self {
            cylinders,
            heads,
            sectors_per_track,
        })
    }
}

fn wrong_disk_chs_part_count() -> String {
    "disk CHS must use CYLINDERS:HEADS:SPT".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disk_geometry_with_colons() {
        let geometry: DiskGeometry = "306:4:17".parse().unwrap();

        assert_eq!(
            geometry,
            DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            }
        );
    }

    #[test]
    fn parses_disk_geometry_with_commas() {
        let geometry: DiskGeometry = "306,4,17".parse().unwrap();

        assert_eq!(
            geometry,
            DiskGeometry {
                cylinders: 306,
                heads: 4,
                sectors_per_track: 17,
            }
        );
    }

    #[test]
    fn rejects_zero_disk_geometry() {
        let error = "306:0:17".parse::<DiskGeometry>().unwrap_err();

        assert_eq!(error, "disk CHS values must be non-zero");
    }
}
