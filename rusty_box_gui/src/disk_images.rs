#[cfg(not(target_arch = "wasm32"))]
use crate::config::ResolvedDiskCreation;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::RunError;
use rusty_box_bximage::CreatedImage;
#[cfg(feature = "gui-egui")]
use rusty_box_bximage::{CreatedImageKind, ImageSize, BYTES_PER_GIB, BYTES_PER_MIB};
#[cfg(not(target_arch = "wasm32"))]
use rusty_box_bximage::{ExistingFilePolicy, SectorSize};

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
