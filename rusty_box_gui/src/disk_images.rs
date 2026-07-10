#[cfg(not(target_arch = "wasm32"))]
use crate::config::ResolvedDiskCreation;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::RunError;
use rusty_box_bximage::CreatedImage;
#[cfg(feature = "gui-egui")]
use rusty_box_bximage::{CreatedImageKind, ImageSize, BYTES_PER_GIB, BYTES_PER_MIB};
#[cfg(not(target_arch = "wasm32"))]
use rusty_box_bximage::{ExistingFilePolicy, SectorSize};

/// Result of provisioning a startup disk image.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiskProvisionOutcome {
    /// A new image was written to disk.
    Created(CreatedImage),
    /// An existing, valid image was kept as-is (no data written).
    Reused { path: std::path::PathBuf, bytes: u64 },
}

#[cfg(not(target_arch = "wasm32"))]
const SECTOR_BYTES: u64 = 512;

/// Provision the configured startup disk image *idempotently*.
///
/// Unlike a raw create, this sanity-checks any existing image instead of forcing
/// a rewrite on every launch:
/// - image missing → create it;
/// - image present and a valid flat image (non-empty, 512-byte aligned) → reuse
///   it untouched (no multi-gigabyte rewrite, no data loss);
/// - image present but not a usable flat image → error, telling the user to set
///   `overwrite = true` or remove it.
///
/// `overwrite = true` still forces a fresh (truncating) create, for the rare case
/// where the user really wants to start over.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn provision_startup_disk(
    creation: &ResolvedDiskCreation,
) -> Result<DiskProvisionOutcome, RunError> {
    if creation.overwrite {
        return create_startup_disk(creation).map(DiskProvisionOutcome::Created);
    }

    match std::fs::metadata(&creation.path) {
        Ok(metadata) => {
            let bytes = metadata.len();
            if bytes == 0 || bytes % SECTOR_BYTES != 0 {
                Err(RunError::InvalidExistingDiskImage {
                    path: creation.path.clone(),
                    len: bytes,
                })
            } else {
                Ok(DiskProvisionOutcome::Reused {
                    path: creation.path.clone(),
                    bytes,
                })
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_startup_disk(creation).map(DiskProvisionOutcome::Created)
        }
        Err(source) => Err(RunError::FileRead {
            kind: "disk",
            path: creation.path.clone(),
            source,
        }),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_startup_disk(
    creation: &ResolvedDiskCreation,
) -> Result<CreatedImage, RunError> {
    let policy = if creation.overwrite {
        ExistingFilePolicy::Truncate
    } else {
        ExistingFilePolicy::CreateNew
    };
    rusty_box_bximage::create_flat_hard_disk(
        &creation.path,
        creation.size,
        SectorSize::Bytes512,
        policy,
    )
    .map_err(|source| RunError::DiskCreate { source })
}

#[cfg(feature = "gui-egui")]
pub(crate) fn format_created_image_message(created: &CreatedImage) -> String {
    match &created.kind {
        CreatedImageKind::HardDisk { geometry } => format!(
            "Created hard disk {} ({}, {} bytes, CHS={}/{}/{})",
            created.path.display(),
            display_hard_disk_size(created.bytes),
            created.bytes,
            geometry.cylinders,
            geometry.heads,
            geometry.sectors_per_track
        ),
        CreatedImageKind::Floppy { format } => format!(
            "Created floppy {} ({}, {} bytes)",
            created.path.display(),
            format.friendly_label(),
            created.bytes
        ),
    }
}

#[cfg(feature = "gui-egui")]
fn display_hard_disk_size(bytes: u64) -> String {
    let mib = bytes.div_ceil(BYTES_PER_MIB);
    if mib != 0 && mib % (BYTES_PER_GIB / BYTES_PER_MIB) == 0 {
        ImageSize::gib(mib / (BYTES_PER_GIB / BYTES_PER_MIB)).display()
    } else {
        ImageSize::mib(mib).display()
    }
}
