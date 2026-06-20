mod error;

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub use error::BxImageError;

pub const BYTES_PER_MIB: u64 = 1024 * 1024;
pub const BYTES_PER_GIB: u64 = 1024 * BYTES_PER_MIB;
pub const FLAT_IMAGE_FINAL_ZERO_BYTES: u64 = 512;
pub const HARD_DISK_HEADS: u32 = 16;
pub const HARD_DISK_SECTORS_PER_TRACK: u32 = 63;
pub const MIN_HARD_DISK_SIZE: ImageSize = ImageSize::mib(10);
pub const DEFAULT_HARD_DISK_SIZE: ImageSize = ImageSize::gib(20);
pub const BOCHS_MAX_CYLINDERS: u64 = 1 << 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImageSize {
    bytes: u64,
}

impl ImageSize {
    pub const fn from_bytes(bytes: u64) -> Self {
        Self { bytes }
    }

    pub const fn mib(mib: u64) -> Self {
        Self {
            bytes: mib * BYTES_PER_MIB,
        }
    }

    pub const fn gib(gib: u64) -> Self {
        Self {
            bytes: gib * BYTES_PER_GIB,
        }
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    pub fn parse(input: &str) -> Result<Self, BxImageError> {
        let trimmed = input.trim_matches(|c: char| c.is_ascii_whitespace());
        if trimmed.is_empty() {
            return Err(BxImageError::MissingSize);
        }

        let suffix_start = trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len());
        let digits = &trimmed[..suffix_start];
        let suffix = &trimmed[suffix_start..];

        if digits.is_empty() || suffix.chars().any(|c| c.is_ascii_digit()) {
            return Err(BxImageError::InvalidSize {
                value: trimmed.to_owned(),
            });
        }

        let unit = if suffix.is_empty() {
            BYTES_PER_MIB
        } else if suffix.eq_ignore_ascii_case("m")
            || suffix.eq_ignore_ascii_case("mb")
            || suffix.eq_ignore_ascii_case("mib")
        {
            BYTES_PER_MIB
        } else if suffix.eq_ignore_ascii_case("g")
            || suffix.eq_ignore_ascii_case("gb")
            || suffix.eq_ignore_ascii_case("gib")
        {
            BYTES_PER_GIB
        } else {
            return Err(BxImageError::InvalidSize {
                value: trimmed.to_owned(),
            });
        };

        let amount = digits
            .parse::<u64>()
            .map_err(|_| BxImageError::SizeOverflow {
                value: trimmed.to_owned(),
            })?;
        if amount == 0 {
            return Err(BxImageError::InvalidSize {
                value: trimmed.to_owned(),
            });
        }

        let bytes = amount
            .checked_mul(unit)
            .ok_or_else(|| BxImageError::SizeOverflow {
                value: trimmed.to_owned(),
            })?;
        Ok(Self { bytes })
    }

    pub fn display(self) -> String {
        if self.bytes % BYTES_PER_GIB == 0 {
            format!("{} GiB", self.bytes / BYTES_PER_GIB)
        } else if self.bytes % BYTES_PER_MIB == 0 {
            format!("{} MiB", self.bytes / BYTES_PER_MIB)
        } else {
            format!("{} bytes", self.bytes)
        }
    }
}

impl fmt::Display for ImageSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorSize {
    Bytes512,
    Bytes1024,
    Bytes4096,
}

impl SectorSize {
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Bytes512 => 512,
            Self::Bytes1024 => 1024,
            Self::Bytes4096 => 4096,
        }
    }

    pub fn from_bytes(bytes: u32) -> Result<Self, BxImageError> {
        match bytes {
            512 => Ok(Self::Bytes512),
            1024 => Ok(Self::Bytes1024),
            4096 => Ok(Self::Bytes4096),
            _ => Err(BxImageError::UnsupportedSectorSize { bytes }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardDiskGeometry {
    pub cylinders: u64,
    pub heads: u32,
    pub sectors_per_track: u32,
    pub sector_size: SectorSize,
    pub final_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloppyFormat {
    K160,
    K180,
    K320,
    K360,
    K720,
    M1_2,
    M1_44,
    M1_68,
    M1_72,
    M2_88,
}

impl FloppyFormat {
    pub const ALL: [FloppyFormat; 10] = [
        FloppyFormat::K160,
        FloppyFormat::K180,
        FloppyFormat::K320,
        FloppyFormat::K360,
        FloppyFormat::K720,
        FloppyFormat::M1_2,
        FloppyFormat::M1_44,
        FloppyFormat::M1_68,
        FloppyFormat::M1_72,
        FloppyFormat::M2_88,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::K160 => "160k",
            Self::K180 => "180k",
            Self::K320 => "320k",
            Self::K360 => "360k",
            Self::K720 => "720k",
            Self::M1_2 => "1.2M",
            Self::M1_44 => "1.44M",
            Self::M1_68 => "1.68M",
            Self::M1_72 => "1.72M",
            Self::M2_88 => "2.88M",
        }
    }

    pub const fn friendly_label(self) -> &'static str {
        match self {
            Self::K160 => "160 KB (5.25\" DD)",
            Self::K180 => "180 KB (5.25\" DD)",
            Self::K320 => "320 KB (5.25\" DD)",
            Self::K360 => "360 KB (5.25\" DD)",
            Self::K720 => "720 KB (3.5\" DD)",
            Self::M1_2 => "1.2 MB (5.25\" HD)",
            Self::M1_44 => "1.44 MB (3.5\" HD)",
            Self::M1_68 => "1.68 MB (3.5\" extended)",
            Self::M1_72 => "1.72 MB (3.5\" extended)",
            Self::M2_88 => "2.88 MB (3.5\" ED)",
        }
    }

    pub const fn sectors(self) -> u64 {
        match self {
            Self::K160 => 320,
            Self::K180 => 360,
            Self::K320 => 640,
            Self::K360 => 720,
            Self::K720 => 1440,
            Self::M1_2 => 2400,
            Self::M1_44 => 2880,
            Self::M1_68 => 3360,
            Self::M1_72 => 3444,
            Self::M2_88 => 5760,
        }
    }

    pub const fn bytes(self) -> u64 {
        self.sectors() * FLAT_IMAGE_FINAL_ZERO_BYTES
    }

    pub fn parse_label(input: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|format| format.label().eq_ignore_ascii_case(input))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingFilePolicy {
    CreateNew,
    Truncate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreatedImageKind {
    HardDisk { geometry: HardDiskGeometry },
    Floppy { format: FloppyFormat },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedImage {
    pub path: PathBuf,
    pub bytes: u64,
    pub kind: CreatedImageKind,
    pub bochsrc_line: String,
}

pub fn calculate_hard_disk_geometry(
    requested_size: ImageSize,
    sector_size: SectorSize,
) -> Result<HardDiskGeometry, BxImageError> {
    if requested_size < MIN_HARD_DISK_SIZE {
        return Err(BxImageError::HardDiskTooSmall {
            requested: requested_size,
            min: MIN_HARD_DISK_SIZE,
        });
    }

    let sector_size_bytes = sector_size.bytes() as u64;
    let track_bytes =
        HARD_DISK_HEADS as u64 * HARD_DISK_SECTORS_PER_TRACK as u64 * sector_size_bytes;
    let cylinders = requested_size.bytes() / track_bytes;
    if cylinders == 0 {
        return Err(BxImageError::ZeroCylinders {
            requested: requested_size,
            sector_size: sector_size.bytes(),
        });
    }
    if cylinders >= BOCHS_MAX_CYLINDERS {
        return Err(BxImageError::CylinderOverflow {
            cylinders,
            max_cylinders: BOCHS_MAX_CYLINDERS,
        });
    }

    Ok(HardDiskGeometry {
        cylinders,
        heads: HARD_DISK_HEADS,
        sectors_per_track: HARD_DISK_SECTORS_PER_TRACK,
        sector_size,
        final_bytes: cylinders * track_bytes,
    })
}

pub fn create_flat_hard_disk(
    path: impl AsRef<Path>,
    requested_size: ImageSize,
    sector_size: SectorSize,
    policy: ExistingFilePolicy,
) -> Result<CreatedImage, BxImageError> {
    let path = path.as_ref();
    let mut file = open_image_file(path, policy)?;
    create_flat_hard_disk_to_writer(path, &mut file, requested_size, sector_size)
}

pub fn create_flat_hard_disk_to_writer<W: Write + Seek>(
    display_path: impl AsRef<Path>,
    writer: &mut W,
    requested_size: ImageSize,
    sector_size: SectorSize,
) -> Result<CreatedImage, BxImageError> {
    let display_path = display_path.as_ref();
    let geometry = calculate_hard_disk_geometry(requested_size, sector_size)?;
    write_flat_image(display_path, writer, geometry.final_bytes)?;

    let path = display_path.to_path_buf();
    let sect_size = if sector_size == SectorSize::Bytes512 {
        String::new()
    } else {
        format!(", sect_size={}", sector_size.bytes())
    };

    Ok(CreatedImage {
        path: path.clone(),
        bytes: geometry.final_bytes,
        kind: CreatedImageKind::HardDisk { geometry },
        bochsrc_line: format!(
            "ata0-master: type=disk, path=\"{}\", mode=flat{}",
            path.display(),
            sect_size
        ),
    })
}

pub fn create_floppy(
    path: impl AsRef<Path>,
    format: FloppyFormat,
    policy: ExistingFilePolicy,
) -> Result<CreatedImage, BxImageError> {
    let path = path.as_ref();
    let mut file = open_image_file(path, policy)?;
    create_floppy_to_writer(path, &mut file, format)
}

pub fn create_floppy_to_writer<W: Write + Seek>(
    display_path: impl AsRef<Path>,
    writer: &mut W,
    format: FloppyFormat,
) -> Result<CreatedImage, BxImageError> {
    let display_path = display_path.as_ref();
    let bytes = format.bytes();
    write_flat_image(display_path, writer, bytes)?;

    let path = display_path.to_path_buf();
    Ok(CreatedImage {
        path: path.clone(),
        bytes,
        kind: CreatedImageKind::Floppy { format },
        bochsrc_line: format!("floppya: image=\"{}\", status=inserted", path.display()),
    })
}

fn write_flat_image<W: Write + Seek>(
    display_path: &Path,
    writer: &mut W,
    bytes: u64,
) -> Result<(), BxImageError> {
    writer
        .seek(SeekFrom::Start(bytes - FLAT_IMAGE_FINAL_ZERO_BYTES))
        .map_err(|source| BxImageError::Write {
            path: display_path.to_path_buf(),
            source,
        })?;
    writer
        .write_all(&[0u8; FLAT_IMAGE_FINAL_ZERO_BYTES as usize])
        .map_err(|source| BxImageError::Write {
            path: display_path.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn open_image_file(path: &Path, policy: ExistingFilePolicy) -> Result<File, BxImageError> {
    let mut options = OpenOptions::new();
    options.write(true);
    match policy {
        ExistingFilePolicy::CreateNew => {
            options.create_new(true);
        }
        ExistingFilePolicy::Truncate => {
            options.create(true).truncate(true);
        }
    }

    options.open(path).map_err(|source| {
        if policy == ExistingFilePolicy::CreateNew
            && source.kind() == std::io::ErrorKind::AlreadyExists
        {
            BxImageError::AlreadyExists {
                path: path.to_path_buf(),
            }
        } else {
            BxImageError::Open {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{nanos}.img", std::process::id()))
    }

    #[test]
    fn parses_human_sizes() {
        assert_eq!(ImageSize::parse("20G").unwrap(), ImageSize::gib(20));
        assert_eq!(ImageSize::parse("512M").unwrap(), ImageSize::mib(512));
        assert_eq!(ImageSize::parse("100").unwrap(), ImageSize::mib(100));
        assert!(matches!(
            ImageSize::parse("1.5G"),
            Err(BxImageError::InvalidSize { .. })
        ));
    }

    #[test]
    fn calculates_10m_geometry_like_bximage() {
        let geometry =
            calculate_hard_disk_geometry(ImageSize::mib(10), SectorSize::Bytes512).unwrap();

        assert_eq!(geometry.cylinders, 20);
        assert_eq!(geometry.heads, 16);
        assert_eq!(geometry.sectors_per_track, 63);
        assert_eq!(geometry.final_bytes, 10_321_920);
    }

    #[test]
    fn calculates_20g_geometry_for_vmware_like_default() {
        let geometry =
            calculate_hard_disk_geometry(ImageSize::gib(20), SectorSize::Bytes512).unwrap();

        assert_eq!(geometry.cylinders, 41_610);
        assert_eq!(geometry.heads, 16);
        assert_eq!(geometry.sectors_per_track, 63);
        assert_eq!(geometry.final_bytes, 21_474_754_560);
    }

    #[test]
    fn truncates_to_whole_cylinders() {
        let geometry =
            calculate_hard_disk_geometry(ImageSize::mib(11), SectorSize::Bytes512).unwrap();

        assert_eq!(geometry.cylinders, 22);
        assert_eq!(geometry.final_bytes, 11_354_112);
    }

    #[test]
    fn rejects_small_hard_disk() {
        assert!(matches!(
            calculate_hard_disk_geometry(ImageSize::mib(9), SectorSize::Bytes512),
            Err(BxImageError::HardDiskTooSmall { .. })
        ));
    }

    #[test]
    fn creates_flat_hard_disk_file_with_final_size() {
        let path = unique_temp_path("rusty-box-bximage-hard-disk");

        let created = create_flat_hard_disk(
            &path,
            ImageSize::mib(10),
            SectorSize::Bytes512,
            ExistingFilePolicy::CreateNew,
        )
        .unwrap();

        assert_eq!(created.bytes, 10_321_920);
        assert_eq!(fs::metadata(&path).unwrap().len(), 10_321_920);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_existing_file_without_overwrite() {
        let path = unique_temp_path("rusty-box-bximage-existing");
        File::create(&path).unwrap();

        let result = create_flat_hard_disk(
            &path,
            ImageSize::mib(10),
            SectorSize::Bytes512,
            ExistingFilePolicy::CreateNew,
        );

        assert!(matches!(result, Err(BxImageError::AlreadyExists { .. })));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn creates_all_bximage_floppy_sizes() {
        for format in FloppyFormat::ALL {
            let path = unique_temp_path(format.label());

            let created = create_floppy(&path, format, ExistingFilePolicy::CreateNew).unwrap();

            assert_eq!(created.bytes, format.bytes());
            assert_eq!(fs::metadata(&path).unwrap().len(), format.bytes());
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn writes_flat_hard_disk_to_cursor() {
        let mut cursor = std::io::Cursor::new(Vec::new());

        let created = create_flat_hard_disk_to_writer(
            "memory.img",
            &mut cursor,
            ImageSize::mib(10),
            SectorSize::Bytes512,
        )
        .unwrap();

        assert_eq!(created.bytes, 10_321_920);
        assert_eq!(cursor.into_inner().len() as u64, created.bytes);
    }

    #[test]
    fn writes_floppy_to_cursor() {
        let mut cursor = std::io::Cursor::new(Vec::new());

        let created =
            create_floppy_to_writer("floppy.img", &mut cursor, FloppyFormat::M1_44).unwrap();

        assert_eq!(created.bytes, 1_474_560);
        assert_eq!(cursor.into_inner().len() as u64, created.bytes);
    }
}
