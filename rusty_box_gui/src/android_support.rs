//! The Android front end's host-independent parts: the file browser's view of
//! a directory, the platform's safe area converted to egui points, and the
//! placement of the files the APK carries. The module compiles for Android,
//! which uses it, and for the host's tests, which pin it.

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
}
