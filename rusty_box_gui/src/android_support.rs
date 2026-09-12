//! The Android front end's host-independent parts: the file browser's view of
//! a directory, the platform's safe area converted to egui points, the
//! placement of the files the APK carries, and the first VM an empty library
//! is given. The module compiles for Android, which uses it, and for the
//! host's tests, which pin it.

use crate::config::{load_toml_file, resolve_config_in, CdromToml, FileConfig, DEFAULT_CONFIG_FILE};
use crate::error::RunError;
use crate::library::{VmLibrary, VmStem, DEFAULT_VM_NAME};
use crate::{BootDevice, DisplayBackend};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// A browser row: a directory to open or a file to choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EntryKind {
    Directory,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) kind: EntryKind,
}

/// The files a browse offers besides directories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileFilter {
    /// Every regular file.
    Any,
    /// Regular files with this extension, compared without case.
    Extension(&'static str),
}

impl FileFilter {
    fn admits(self, path: &Path) -> bool {
        match self {
            Self::Any => true,
            Self::Extension(wanted) => path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted)),
        }
    }
}

/// The directories and admitted files in `dir`, directories first, each group
/// ordered by name. Links are followed, because Android's shared storage is
/// reached through them (`/sdcard`); an entry whose target cannot be read is
/// left out, since the browser could not open it either.
pub(crate) fn list_directory(dir: &Path, filter: FileFilter) -> io::Result<Vec<DirectoryEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => EntryKind::Directory,
            Ok(metadata) if metadata.is_file() && filter.admits(&path) => EntryKind::File,
            Ok(_) | Err(_) => continue,
        };
        entries.push(DirectoryEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            kind,
        });
    }
    entries.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

/// A rectangle in the platform's physical pixels, as Android reports a
/// window's content rect: `right` and `bottom` lie just outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

/// The part of `viewport` that `content` covers, in egui points. `None` when
/// `content` is empty, when the scale is not a positive finite number, or
/// when the two do not overlap; the caller then keeps the whole viewport.
pub(crate) fn content_rect_in_points(
    viewport: egui::Rect,
    pixels_per_point: f32,
    content: PixelRect,
) -> Option<egui::Rect> {
    if !pixels_per_point.is_finite()
        || pixels_per_point <= 0.0
        || content.right <= content.left
        || content.bottom <= content.top
    {
        return None;
    }
    let rect = egui::Rect::from_min_max(
        egui::pos2(
            content.left as f32 / pixels_per_point,
            content.top as f32 / pixels_per_point,
        ),
        egui::pos2(
            content.right as f32 / pixels_per_point,
            content.bottom as f32 / pixels_per_point,
        ),
    )
    .intersect(viewport);
    (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}

/// Puts `bytes` at `dir/name` and returns that path. A file already holding
/// exactly these bytes is left as it is; anything else there is replaced. The
/// bytes go to a sibling first and are renamed into place, so the named file
/// is never a partial copy.
pub(crate) fn stage_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(name);
    if holds_exactly(&path, bytes)? {
        return Ok(path);
    }
    let partial = dir.join(format!("{name}.partial"));
    fs::write(&partial, bytes)?;
    fs::rename(&partial, &path)?;
    Ok(path)
}

/// Whether the file at `path` holds exactly `bytes`. A missing file does not.
fn holds_exactly(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if usize::try_from(file.metadata()?.len()).ok() != Some(bytes.len()) {
        return Ok(false);
    }
    let mut chunk = vec![0u8; 64 * 1024];
    let mut offset = 0;
    while offset < bytes.len() {
        let want = (bytes.len() - offset).min(chunk.len());
        let read = file.read(&mut chunk[..want])?;
        if read == 0 || chunk[..read] != bytes[offset..offset + read] {
            return Ok(false);
        }
        offset += read;
    }
    Ok(true)
}

/// The files the APK carries, placed in the app's storage, and what a phone
/// gives a VM made from them alone.
pub(crate) struct CarriedMachine {
    /// What a VM made from the carried files alone is called.
    pub(crate) name: &'static str,
    pub(crate) bios: PathBuf,
    pub(crate) vga_bios: PathBuf,
    /// The CD such a VM boots first: the Alpine ISO in a build with
    /// `embedded-alpine`, none otherwise.
    pub(crate) cdrom: Option<PathBuf>,
    pub(crate) memory_mib: u32,
    pub(crate) ips: u32,
}

impl CarriedMachine {
    /// `file` with the carried ROMs and the phone's sizes where it names
    /// none, the display a phone has, and — when a CD is carried and the file
    /// names none — that CD, booted first unless the file orders the boot
    /// itself.
    fn fill(&self, mut file: FileConfig) -> FileConfig {
        if file.rom.bios.is_none() {
            file.rom.bios = Some(self.bios.clone());
        }
        if file.rom.vga_bios.is_none() {
            file.rom.vga_bios = Some(self.vga_bios.clone());
        }
        if file.emulator.memory_mib.is_none() {
            file.emulator.memory_mib = Some(self.memory_mib);
        }
        if file.emulator.ips.is_none() {
            file.emulator.ips = Some(self.ips);
        }
        file.display.backend = Some(DisplayBackend::Egui);
        let carried_cd = self.cdrom.as_ref().filter(|_| file.cdrom.is_none());
        if let Some(cdrom) = carried_cd {
            file.cdrom = Some(CdromToml {
                path: Some(cdrom.clone()),
                channel: None,
                drive: None,
            });
            if file.boot.order.is_empty() {
                file.boot.order = vec![BootDevice::Cdrom];
            }
        }
        file
    }
}

/// Whether the library holds no file and so is given its first VM at this
/// launch. A library that cannot be listed is given none: the shell lists it
/// itself and shows that error, so nothing is said twice.
pub(crate) fn needs_first_vm(library: &VmLibrary) -> bool {
    match library.is_empty() {
        Ok(empty) => empty,
        Err(error) => {
            tracing::warn!("the VM library could not be listed, so it is given no VM: {error}");
            false
        }
    }
}

/// Gives the empty library its first VM and records it as the one the shell
/// opens on. The VM is the one the settings a build before the library saved
/// in `rusty_box.toml` under `storage` describe, `carried` filling in what
/// the file leaves out; with no such file, a VM made from `carried` alone.
/// The file is left where it is: the library never reads it again. Returns
/// what could not be done, in one
/// message for the shell to show — the settings did not import, so the VM
/// was made from the carried files instead; no VM could be made; the VM was
/// made but the record of it was not written, so the next launch will not
/// open on it — and `None` when everything was done.
pub(crate) fn seed_first_vm(
    library: &VmLibrary,
    storage: &Path,
    carried: &CarriedMachine,
) -> Option<String> {
    let mut problems = Vec::new();
    let saved = storage.join(DEFAULT_CONFIG_FILE);
    let imported = if saved.is_file() {
        let imported = load_toml_file(&saved).and_then(|file| {
            let name = file.vm.name.clone().filter(|name| !name.trim().is_empty());
            let name = name.as_deref().unwrap_or(DEFAULT_VM_NAME);
            add_first_vm(library, name, carried.fill(file), storage)
        });
        match imported {
            Ok(stem) => Some(stem),
            Err(error) => {
                problems.push(format!(
                    "The settings saved in {} were not imported: {error}.",
                    saved.display()
                ));
                None
            }
        }
    } else {
        None
    };
    let made = match imported {
        Some(stem) => Ok(stem),
        None => add_first_vm(
            library,
            carried.name,
            carried.fill(FileConfig::default()),
            library.dir(),
        ),
    };
    match made {
        Ok(stem) => {
            if let Err(error) = library.remember_selected(&stem) {
                problems.push(format!(
                    "The next launch will not open on the VM that was made: {error}."
                ));
            }
        }
        Err(error) => {
            problems.push(format!(
                "No VM was made from the files the APK carries: {error}."
            ));
        }
    }
    if problems.is_empty() {
        return None;
    }
    let notice = problems.join(" ");
    tracing::warn!("{notice}");
    Some(notice)
}

/// Adds the VM `file` describes under `name`, its relative paths resolved
/// against `base`, and returns the stem of its new file.
fn add_first_vm(
    library: &VmLibrary,
    name: &str,
    file: FileConfig,
    base: &Path,
) -> Result<VmStem, RunError> {
    let config = resolve_config_in(file, base)?;
    Ok(library.create(name, &config)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusty_box_gui_android_{tag}_{}",
            std::process::id()
        ));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("clear scratch dir");
        }
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn a_listing_puts_directories_first_and_offers_only_admitted_files() {
        let root = scratch_dir("listing");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("create nested dir");
        fs::write(root.join("alpine.iso"), b"iso").expect("write first iso");
        fs::write(root.join("BOOT.ISO"), b"iso").expect("write second iso");
        fs::write(root.join("notes.txt"), b"text").expect("write other file");

        let entries = list_directory(&root, FileFilter::Extension("iso")).expect("list");

        assert_eq!(
            entries,
            vec![
                DirectoryEntry {
                    name: "nested".to_owned(),
                    path: nested,
                    kind: EntryKind::Directory,
                },
                DirectoryEntry {
                    name: "BOOT.ISO".to_owned(),
                    path: root.join("BOOT.ISO"),
                    kind: EntryKind::File,
                },
                DirectoryEntry {
                    name: "alpine.iso".to_owned(),
                    path: root.join("alpine.iso"),
                    kind: EntryKind::File,
                },
            ]
        );
        fs::remove_dir_all(&root).expect("remove scratch dir");
    }

    #[test]
    fn the_any_filter_offers_every_file() {
        let root = scratch_dir("any");
        fs::write(root.join("disk.img"), b"img").expect("write image");
        fs::write(root.join("bios.bin"), b"rom").expect("write rom");

        let names: Vec<String> = list_directory(&root, FileFilter::Any)
            .expect("list")
            .into_iter()
            .map(|entry| entry.name)
            .collect();

        assert_eq!(names, ["bios.bin", "disk.img"]);
        fs::remove_dir_all(&root).expect("remove scratch dir");
    }

    #[test]
    fn a_content_rect_in_pixels_becomes_points_inside_the_viewport() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 548.0));
        let content = PixelRect {
            left: 0,
            top: 48,
            right: 2496,
            bottom: 1096,
        };

        let rect = content_rect_in_points(viewport, 2.0, content).expect("a usable rect");

        assert_eq!(rect.min, egui::pos2(0.0, 24.0));
        assert_eq!(rect.max, egui::pos2(1248.0, 548.0));
    }

    #[test]
    fn an_empty_content_rect_or_an_unusable_scale_gives_no_rect() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        let empty = PixelRect {
            left: 10,
            top: 10,
            right: 10,
            bottom: 50,
        };
        let usable = PixelRect {
            left: 0,
            top: 0,
            right: 50,
            bottom: 50,
        };

        assert_eq!(content_rect_in_points(viewport, 1.0, empty), None);
        assert_eq!(content_rect_in_points(viewport, 0.0, usable), None);
        assert_eq!(content_rect_in_points(viewport, f32::NAN, usable), None);
    }

    #[test]
    fn staging_writes_once_and_replaces_different_bytes_of_the_same_length() {
        let root = scratch_dir("stage");

        let path = stage_file(&root, "BIOS-bochs-latest", b"first").expect("stage");
        assert_eq!(fs::read(&path).expect("read staged"), b"first");

        let modified = fs::metadata(&path).expect("stat").modified().expect("mtime");
        let again = stage_file(&root, "BIOS-bochs-latest", b"first").expect("stage again");
        assert_eq!(again, path);
        assert_eq!(
            fs::metadata(&path).expect("stat").modified().expect("mtime"),
            modified,
            "identical bytes must leave the staged file untouched"
        );

        stage_file(&root, "BIOS-bochs-latest", b"other").expect("restage");
        assert_eq!(fs::read(&path).expect("read restaged"), b"other");
        assert!(
            !root.join("BIOS-bochs-latest.partial").exists(),
            "no partial copy may remain"
        );
        fs::remove_dir_all(&root).expect("remove scratch dir");
    }

    /// What the APK carries, as placed under `storage`, with the Alpine CD.
    fn carried_machine(storage: &Path) -> CarriedMachine {
        let carried = storage.join("carried");
        CarriedMachine {
            name: "Alpine",
            bios: carried.join("BIOS-bochs-latest"),
            vga_bios: carried.join("VGABIOS-lgpl-latest.bin"),
            cdrom: Some(carried.join("alpine.iso")),
            memory_mib: 256,
            ips: 300_000_000,
        }
    }

    fn phone_library(storage: &Path) -> VmLibrary {
        VmLibrary::open(storage.join("vms")).expect("open the library")
    }

    #[test]
    fn saved_settings_become_the_first_vm_with_carried_files_where_they_name_none() {
        let storage = scratch_dir("seed_saved");
        fs::write(
            storage.join(DEFAULT_CONFIG_FILE),
            "[emulator]\nmemory_mib = 96\n\n[cdrom]\npath = \"other.iso\"\n",
        )
        .expect("write the saved settings");
        let library = phone_library(&storage);
        let machine = carried_machine(&storage);
        assert!(needs_first_vm(&library));

        let notice = seed_first_vm(&library, &storage, &machine);

        assert_eq!(notice, None);
        let contents = library.load().expect("load");
        assert_eq!(contents.vms.len(), 1);
        let vm = &contents.vms[0];
        assert_eq!(vm.name, DEFAULT_VM_NAME);
        assert_eq!(vm.config.memory_mib, 96);
        assert_eq!(vm.config.ips, machine.ips);
        assert_eq!(vm.config.bios, machine.bios);
        assert_eq!(vm.config.vga_bios, Some(machine.vga_bios.clone()));
        assert_eq!(vm.config.display, DisplayBackend::Egui);
        // The file's own CD is kept, resolved against the file's folder.
        assert_eq!(
            vm.config.cdrom.as_ref().map(|cdrom| cdrom.path.clone()),
            Some(storage.join("other.iso"))
        );
        assert_eq!(library.last_selected(), Some(vm.stem.clone()));
        assert!(
            storage.join(DEFAULT_CONFIG_FILE).is_file(),
            "the saved settings stay where they are"
        );
        assert!(!needs_first_vm(&library));
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }

    #[test]
    fn with_no_saved_settings_the_first_vm_is_made_from_the_carried_files() {
        let storage = scratch_dir("seed_carried");
        let library = phone_library(&storage);
        let machine = carried_machine(&storage);

        let notice = seed_first_vm(&library, &storage, &machine);

        assert_eq!(notice, None);
        let contents = library.load().expect("load");
        assert_eq!(contents.vms.len(), 1);
        let vm = &contents.vms[0];
        assert_eq!(vm.name, "Alpine");
        assert_eq!(vm.config.memory_mib, 256);
        assert_eq!(vm.config.ips, 300_000_000);
        assert_eq!(vm.config.bios, machine.bios);
        assert_eq!(vm.config.vga_bios, Some(machine.vga_bios.clone()));
        assert_eq!(vm.config.display, DisplayBackend::Egui);
        assert_eq!(
            vm.config.cdrom.as_ref().map(|cdrom| cdrom.path.clone()),
            machine.cdrom
        );
        assert_eq!(vm.config.boot_order, [BootDevice::Cdrom]);
        assert_eq!(library.last_selected(), Some(vm.stem.clone()));
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }

    #[test]
    fn a_build_without_a_cd_seeds_a_vm_of_the_default_name_with_none() {
        let storage = scratch_dir("seed_no_cd");
        let library = phone_library(&storage);
        let machine = CarriedMachine {
            name: DEFAULT_VM_NAME,
            cdrom: None,
            ..carried_machine(&storage)
        };

        let notice = seed_first_vm(&library, &storage, &machine);

        assert_eq!(notice, None);
        let contents = library.load().expect("load");
        assert_eq!(contents.vms[0].name, DEFAULT_VM_NAME);
        assert_eq!(contents.vms[0].config.cdrom, None);
        assert!(contents.vms[0].config.boot_order.is_empty());
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }

    #[test]
    fn a_library_that_holds_a_file_or_cannot_be_listed_needs_no_first_vm() {
        let storage = scratch_dir("needs_none");
        let library = phone_library(&storage);
        fs::write(library.dir().join("bad.toml"), "memory_mib = [").expect("write");
        assert!(!needs_first_vm(&library));

        fs::remove_dir_all(library.dir()).expect("remove the folder under the library");
        assert!(!needs_first_vm(&library));
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }

    #[test]
    fn saved_settings_that_do_not_load_are_reported_and_the_carried_vm_made_instead() {
        let storage = scratch_dir("seed_bad_saved");
        fs::write(storage.join(DEFAULT_CONFIG_FILE), "memory_mib = [").expect("write");
        let library = phone_library(&storage);

        let notice = seed_first_vm(&library, &storage, &carried_machine(&storage))
            .expect("what could not be done");

        assert!(notice.contains("were not imported"), "{notice:?}");
        assert!(notice.contains(DEFAULT_CONFIG_FILE), "{notice:?}");
        let contents = library.load().expect("load");
        assert_eq!(contents.vms.len(), 1);
        assert_eq!(contents.vms[0].name, "Alpine");
        assert_eq!(library.last_selected(), Some(contents.vms[0].stem.clone()));
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }

    #[test]
    fn a_first_vm_whose_record_cannot_be_written_is_kept_and_reported() {
        let storage = scratch_dir("seed_no_record");
        let library = phone_library(&storage);
        fs::create_dir(library.dir().join(".last")).expect("a folder where the record goes");

        let notice = seed_first_vm(&library, &storage, &carried_machine(&storage))
            .expect("what could not be done");

        assert!(notice.contains("next launch"), "{notice:?}");
        assert_eq!(library.load().expect("load").vms.len(), 1);
        assert_eq!(library.last_selected(), None);
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }

    #[test]
    fn a_first_vm_that_cannot_be_made_is_reported() {
        let storage = scratch_dir("seed_no_vm");
        let library = phone_library(&storage);
        fs::create_dir(library.dir().join("alpine.toml")).expect("a folder in the file's way");

        let notice = seed_first_vm(&library, &storage, &carried_machine(&storage))
            .expect("what could not be done");

        assert!(notice.contains("No VM was made"), "{notice:?}");
        assert!(library.load().expect("load").vms.is_empty());
        fs::remove_dir_all(&storage).expect("remove scratch dir");
    }
}
