//! The hard disk a VM boots from, as the Hard disk page offers it: the sizes
//! a new disk comes in, the name and folder it is given, and which of the
//! page's states a VM's settings put it in.

use rusty_box_bximage::ImageSize;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// A new hard disk's size, whole MiB.
///
/// A native shell writes the image sparse, so a large disk costs only what
/// the guest writes to it. A browser builds the whole image in memory before
/// saving it, so the browser's sizes stay small.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiskSize {
    mib: u32,
}

impl DiskSize {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const fn gib(gib: u32) -> Self {
        Self { mib: gib * 1024 }
    }

    /// The browser's sizes are in MiB; a native shell offers whole GB.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) const fn mib(mib: u32) -> Self {
        Self { mib }
    }

    /// The sizes offered as chips, smallest first.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const PRESETS: [Self; 5] = [
        Self::gib(2),
        Self::gib(8),
        Self::gib(16),
        Self::gib(32),
        Self::gib(64),
    ];
    /// Enough for Windows 10, which asks for 32 GB.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const DEFAULT: Self = Self::gib(32);
    /// A custom size is counted in whole units of this many MiB.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const CUSTOM_UNIT_MIB: u32 = 1024;
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const CUSTOM_UNIT: &'static str = "GB";
    /// The custom sizes allowed, in custom units.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const CUSTOM_RANGE: core::ops::RangeInclusive<u32> = 1..=2048;

    /// The sizes offered as chips, smallest first.
    #[cfg(target_arch = "wasm32")]
    pub(crate) const PRESETS: [Self; 4] = [
        Self::mib(16),
        Self::mib(64),
        Self::mib(256),
        Self::mib(512),
    ];
    #[cfg(target_arch = "wasm32")]
    pub(crate) const DEFAULT: Self = Self::mib(64);
    #[cfg(target_arch = "wasm32")]
    pub(crate) const CUSTOM_UNIT_MIB: u32 = 1;
    #[cfg(target_arch = "wasm32")]
    pub(crate) const CUSTOM_UNIT: &'static str = "MB";
    #[cfg(target_arch = "wasm32")]
    pub(crate) const CUSTOM_RANGE: core::ops::RangeInclusive<u32> = 1..=1024;

    /// A custom size of `count` custom units, held to [`Self::CUSTOM_RANGE`].
    pub(crate) fn custom(count: u32) -> Self {
        let count = count.clamp(*Self::CUSTOM_RANGE.start(), *Self::CUSTOM_RANGE.end());
        Self {
            mib: count * Self::CUSTOM_UNIT_MIB,
        }
    }

    pub(crate) fn image_size(self) -> ImageSize {
        ImageSize::mib(u64::from(self.mib))
    }

    /// The size as a chip and the page caption it: "32 GB", "64 MB".
    pub(crate) fn label(self) -> String {
        if self.mib % 1024 == 0 {
            format!("{} GB", self.mib / 1024)
        } else {
            format!("{} MB", self.mib)
        }
    }
}

/// A disk file's length as the page captions it: "32.0 GB", "512 MB".
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn format_disk_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else {
        format!("{} MB", bytes.div_ceil(MIB))
    }
}

/// Characters a file name may not hold on the platforms the shell runs on.
const FORBIDDEN_IN_NAMES: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// The file name a new disk is offered under: the VM's name with the
/// characters a file system refuses replaced, and `.img` after it.
pub(crate) fn default_disk_name(vm_name: &str) -> String {
    let cleaned: String = vm_name
        .chars()
        .map(|c| {
            if c.is_control() || FORBIDDEN_IN_NAMES.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Windows drops trailing dots and spaces, so a name ending in them would
    // not be the name the file gets.
    let stem = cleaned.trim().trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() { "disk" } else { stem };
    if stem.to_ascii_lowercase().ends_with(".img") {
        stem.to_owned()
    } else {
        format!("{stem}.img")
    }
}

/// `name` if nothing is called that yet, otherwise the first `stem (n).ext`
/// from n = 2 that is free. `taken` says whether a name is in use.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn first_free_name<F: Fn(&str) -> bool>(name: &str, taken: F) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let stem = path.file_stem().map_or_else(|| name.to_owned(), |stem| stem.to_string_lossy().into_owned());
    let extension = path.extension().map(|ext| ext.to_string_lossy().into_owned());
    let candidate = |n: u32| match &extension {
        Some(ext) => format!("{stem} ({n}).{ext}"),
        None => format!("{stem} ({n})"),
    };
    (2..=FREE_NAME_ATTEMPTS)
        .map(candidate)
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| candidate(FREE_NAME_ATTEMPTS + 1))
}

/// How many numbered names are tried before giving up on finding a free one.
#[cfg(not(target_arch = "wasm32"))]
const FREE_NAME_ATTEMPTS: u32 = 10_000;

/// Where new disks are saved unless another folder is chosen: `Rusty Box`
/// under the user's Documents folder, which lies under `base`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn documents_folder_under(base: &Path) -> PathBuf {
    base.join("Documents").join("Rusty Box")
}

/// The folder whose `Documents` holds the shell's disks on this platform:
/// shared storage on a phone, the user's home elsewhere. `None` when the
/// platform does not say where that is.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn documents_base() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    let base = Some(
        std::env::var_os("EXTERNAL_STORAGE")
            .map_or_else(|| PathBuf::from("/storage/emulated/0"), PathBuf::from),
    );
    #[cfg(all(windows, not(target_os = "android")))]
    let base = std::env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(all(not(windows), not(target_os = "android")))]
    let base = std::env::var_os("HOME").map(PathBuf::from);
    base
}

/// Creates a sparse hard disk image of `size` at `path`, making its folder
/// first. `existing` says what happens to a file already there.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_disk(
    path: &Path,
    size: DiskSize,
    existing: rusty_box_bximage::ExistingFilePolicy,
) -> Result<rusty_box_bximage::CreatedImage, String> {
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    }
    rusty_box_bximage::create_flat_hard_disk(
        path,
        size.image_size(),
        rusty_box_bximage::SectorSize::Bytes512,
        existing,
    )
    .map_err(|error| error.to_string())
}

/// What the Hard disk page shows for a VM.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HardDiskState {
    /// No disk: the page offers a new one or an existing file.
    Empty,
    /// This file is the VM's hard disk.
    Attached { path: PathBuf },
}

#[cfg(not(target_arch = "wasm32"))]
impl HardDiskState {
    /// The state a VM's disk settings describe: an enabled disk with a path
    /// is attached; anything else is no disk.
    pub(crate) fn of(disk_enabled: bool, disk_path: &str) -> Self {
        let path = disk_path.trim();
        if disk_enabled && !path.is_empty() {
            Self::Attached {
                path: PathBuf::from(path),
            }
        } else {
            Self::Empty
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_run_2_to_64_gb_and_the_default_is_32() {
        assert_eq!(
            DiskSize::PRESETS.map(DiskSize::label),
            ["2 GB", "8 GB", "16 GB", "32 GB", "64 GB"]
        );
        assert_eq!(DiskSize::DEFAULT, DiskSize::gib(32));
        assert_eq!(DiskSize::gib(16).image_size().bytes(), 16 * 1024 * 1024 * 1024);
        assert_eq!(DiskSize::mib(64).label(), "64 MB");
    }

    #[test]
    fn a_disk_length_reads_in_gb_or_mb() {
        assert_eq!(format_disk_bytes(34_359_738_368), "32.0 GB");
        assert_eq!(format_disk_bytes(10_321_920), "10 MB");
    }

    #[test]
    fn a_custom_size_is_whole_gb_held_to_its_range() {
        assert_eq!(DiskSize::custom(0), DiskSize::gib(1));
        assert_eq!(DiskSize::custom(100), DiskSize::gib(100));
        assert_eq!(DiskSize::custom(99_999), DiskSize::gib(2048));
    }

    #[test]
    fn the_default_name_is_the_vm_name_made_safe_for_a_file() {
        assert_eq!(default_disk_name("Rusty Box"), "Rusty Box.img");
        assert_eq!(default_disk_name("Win: 10/Pro?"), "Win_ 10_Pro_.img");
        assert_eq!(default_disk_name("  ... "), "disk.img");
        assert_eq!(default_disk_name("Alpine. "), "Alpine.img");
        assert_eq!(default_disk_name("backup.IMG"), "backup.IMG");
    }

    #[test]
    fn a_taken_name_gets_the_first_free_number() {
        let taken = ["Rusty Box.img", "Rusty Box (2).img"];
        let name = first_free_name("Rusty Box.img", |candidate| taken.contains(&candidate));
        assert_eq!(name, "Rusty Box (3).img");
        assert_eq!(first_free_name("free.img", |_| false), "free.img");
    }

    #[test]
    fn disks_go_to_rusty_box_under_documents() {
        assert_eq!(
            documents_folder_under(Path::new("/storage/emulated/0")),
            Path::new("/storage/emulated/0").join("Documents").join("Rusty Box")
        );
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("rusty_box_gui_hard_disk_{tag}_{}", std::process::id()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_new_disk_is_created_with_its_folder_and_holds_the_chosen_size() {
        let folder = scratch("create");
        drop(std::fs::remove_dir_all(&folder));
        let path = folder.join("nested").join("new.img");

        let created = create_disk(
            &path,
            DiskSize::custom(1),
            rusty_box_bximage::ExistingFilePolicy::CreateNew,
        )
        .expect("create");

        let length = std::fs::metadata(&path).expect("file").len();
        assert_eq!(length, created.bytes);
        // Whole cylinders of 16 heads x 63 sectors x 512 bytes, up to 1 GiB.
        assert!(length <= 1024 * 1024 * 1024 && length > 1_000_000_000, "{length}");
        std::fs::remove_dir_all(&folder).expect("clean up");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_new_disk_does_not_replace_a_file_unless_told_to() {
        let folder = scratch("existing");
        std::fs::create_dir_all(&folder).expect("folder");
        let path = folder.join("taken.img");
        std::fs::write(&path, b"already here").expect("write");

        let refused = create_disk(
            &path,
            DiskSize::custom(1),
            rusty_box_bximage::ExistingFilePolicy::CreateNew,
        );
        assert!(
            matches!(&refused, Err(message) if message.contains("already exists")),
            "{refused:?}"
        );
        assert_eq!(std::fs::read(&path).expect("read"), b"already here");

        create_disk(
            &path,
            DiskSize::custom(1),
            rusty_box_bximage::ExistingFilePolicy::Truncate,
        )
        .expect("replace");
        assert!(std::fs::metadata(&path).expect("file").len() > 1_000_000_000);
        std::fs::remove_dir_all(&folder).expect("clean up");
    }

    #[test]
    fn only_an_enabled_disk_with_a_path_is_attached() {
        assert_eq!(HardDiskState::of(false, "c.img"), HardDiskState::Empty);
        assert_eq!(HardDiskState::of(true, "  "), HardDiskState::Empty);
        assert_eq!(
            HardDiskState::of(true, " c.img "),
            HardDiskState::Attached {
                path: PathBuf::from("c.img")
            }
        );
    }
}
