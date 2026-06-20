use crate::ImageSize;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BxImageError {
    #[error("disk size is required; use values like 20G or 512M")]
    MissingSize,

    #[error("invalid disk size '{value}'; use whole-number sizes like 20G or 512M")]
    InvalidSize { value: String },

    #[error("disk size must be at least {min}: requested {requested}")]
    HardDiskTooSmall {
        requested: ImageSize,
        min: ImageSize,
    },

    #[error("disk size overflows u64: {value}")]
    SizeOverflow { value: String },

    #[error(
        "hard disk geometry has zero cylinders for {requested} with {sector_size} byte sectors"
    )]
    ZeroCylinders {
        requested: ImageSize,
        sector_size: u32,
    },

    #[error("hard disk cylinder count {cylinders} exceeds Bochs limit {max_cylinders}")]
    CylinderOverflow { cylinders: u64, max_cylinders: u64 },

    #[error("unsupported hard disk sector size {bytes}; expected 512, 1024, or 4096")]
    UnsupportedSectorSize { bytes: u32 },

    #[error("image path already exists: {}", path.display())]
    AlreadyExists { path: PathBuf },

    #[error("failed to open image {}: {source}", path.display())]
    Open {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write image {}: {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}
