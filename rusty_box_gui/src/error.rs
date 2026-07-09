use crate::args::BootDevice;
use std::{io, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("failed to read config {}: {source}", path.display())]
    ConfigRead { path: PathBuf, source: io::Error },

    #[error("failed to parse TOML config {}: {source}", path.display())]
    ConfigParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("failed to write config {}: {source}", path.display())]
    ConfigWrite { path: PathBuf, source: io::Error },

    #[error("failed to serialize config: {source}")]
    ConfigSerialize { source: toml::ser::Error },

    #[error("BIOS path is required; pass --bios PATH or set rom.bios in TOML")]
    MissingBios,

    #[error("{field} must be greater than zero")]
    ZeroValue { field: &'static str },

    #[error("{field} is too large for this platform")]
    ValueOverflow { field: &'static str },

    #[error("invalid CPU topology: {message}")]
    InvalidCpuTopology { message: String },

    #[error("boot order cannot be empty; set boot.order or pass --boot disk|cdrom")]
    EmptyBootOrder,

    #[error("boot order supports at most 3 devices")]
    TooManyBootDevices,

    #[error("boot order contains duplicate device {device}")]
    DuplicateBootDevice { device: BootDevice },

    #[error("boot device {device} requires {field}")]
    MissingBootMedia {
        device: BootDevice,
        field: &'static str,
    },

    #[error("failed to read {kind} file {}: {source}", path.display())]
    FileRead {
        kind: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    #[error("{kind} file is empty: {}", path.display())]
    EmptyFile { kind: &'static str, path: PathBuf },

    #[error("VGA BIOS size must be a non-zero multiple of 512 bytes: {} has {len} bytes", path.display())]
    InvalidVgaBiosSize { path: PathBuf, len: usize },

    #[error("invalid disk image size for CHS auto-detection: {} has {len} bytes", path.display())]
    InvalidDiskSize { path: PathBuf, len: u64 },

    #[error("disk image too large for BIOS CHS auto-detection: {} needs {cylinders} cylinders", path.display())]
    DiskTooLargeForChs { path: PathBuf, cylinders: u64 },

    #[error("disk.create path is required when disk creation is requested")]
    MissingDiskCreatePath,

    #[error("conflicting disk options: {first} cannot be used with {second}")]
    ConflictingDiskOptions {
        first: &'static str,
        second: &'static str,
    },

    #[error("created disk CHS exceeds current GUI geometry field: {} needs {cylinders} cylinders", path.display())]
    CreatedDiskChsOverflow { path: PathBuf, cylinders: u64 },

    #[error("invalid disk.create.size: {source}")]
    DiskCreateSize {
        source: rusty_box_bximage::BxImageError,
    },

    #[error("failed to create disk image: {source}")]
    DiskCreate {
        source: rusty_box_bximage::BxImageError,
    },

    #[error("path must be valid UTF-8 for current emulator media API: {path:?}")]
    NonUtf8Path { path: PathBuf },

    #[error("failed to attach {kind} {}: {source}", path.display())]
    MediaAttach {
        kind: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    #[error("failed to start emulator thread: {source}")]
    ThreadStart { source: io::Error },

    #[error("emulator thread panicked")]
    EmulatorThreadPanic,

    #[cfg(feature = "gui-egui")]
    #[error("egui window failed: {message}")]
    Gui { message: String },

    #[error(transparent)]
    Emulator(#[from] rusty_box::Error),
}
