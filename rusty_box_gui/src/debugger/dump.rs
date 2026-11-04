//! A VM's RAM written to a file, whole and in RAM-store order
//! (`Emulator::read_ram_image`): below 3 GiB the file offset is the physical
//! address; RAM from 4 GiB up follows straight after.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use rusty_box::cpu::Instrumentation;
use rusty_box::emulator::{Emulator, RamOffset, SliceEngine};

use super::control::DumpOutcome;

/// How much RAM is read and written at once.
const DUMP_CHUNK_BYTES: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
enum DumpError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("guest RAM could not be read: {0}")]
    Read(#[from] rusty_box::Error),
    /// The run was told to end while the dump was being written.
    #[error("{}", super::control::NOT_SAVED_VM_STOPPED)]
    Abandoned,
}

/// Writes all of `emu`'s RAM to `path`, giving up between chunks once
/// `abandoned` answers true (the VM is being powered off or the app closed,
/// which must not wait for several GiB). The bytes go to a `.partial` file
/// beside it first, which takes `path`'s name only once it is whole, so a
/// failed dump never leaves a short file under the name asked for.
pub(crate) fn dump_ram<T: Instrumentation, E: SliceEngine<T>, F: Fn() -> bool>(
    emu: &mut Emulator<T, E>,
    path: &Path,
    abandoned: F,
) -> DumpOutcome {
    let partial = partial_path(path);
    let written = write_ram(emu, &partial, abandoned).and_then(|bytes| {
        fs::rename(&partial, path)
            .map(|()| bytes)
            .map_err(DumpError::Io)
    });
    match written {
        Ok(bytes) => DumpOutcome::Saved {
            path: path.to_owned(),
            bytes,
        },
        Err(error) => {
            match fs::remove_file(&partial) {
                Ok(()) => {}
                Err(removal) if removal.kind() == io::ErrorKind::NotFound => {}
                Err(removal) => tracing::warn!(
                    "the partial dump {} could not be removed: {removal}",
                    partial.display()
                ),
            }
            DumpOutcome::Failed {
                path: path.to_owned(),
                reason: error.to_string(),
            }
        }
    }
}

fn write_ram<T: Instrumentation, E: SliceEngine<T>, F: Fn() -> bool>(
    emu: &mut Emulator<T, E>,
    partial: &Path,
    abandoned: F,
) -> Result<u64, DumpError> {
    let total = emu.mem_size() as u64;
    let mut out = BufWriter::new(File::create(partial)?);
    let mut chunk = vec![0u8; DUMP_CHUNK_BYTES];
    let mut offset = 0u64;
    while offset < total {
        if abandoned() {
            return Err(DumpError::Abandoned);
        }
        // `offset` is below `total` here, so what is left is at least 1.
        let len = usize::try_from(total.saturating_sub(offset))
            .map_or(chunk.len(), |left| left.min(chunk.len()));
        let buf = chunk
            .get_mut(..len)
            .ok_or_else(|| io::Error::other("a dump chunk outgrew its buffer"))?;
        emu.read_ram_image(RamOffset(offset), buf)?;
        out.write_all(buf)?;
        offset = offset.saturating_add(len as u64);
    }
    let file = out
        .into_inner()
        .map_err(|error| DumpError::Io(error.into_error()))?;
    file.sync_all()?;
    Ok(total)
}

/// `path` with `.partial` after its name.
fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(".partial");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::emulator::{EmulatorConfig, MachineBuilder, MemorySize};

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rusty-box-gui-dump-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn a_dump_holds_every_byte_of_ram_in_order() {
        let mut machine = MachineBuilder::new(EmulatorConfig {
            memory: MemorySize::bytes(4 * 1024 * 1024),
            ..EmulatorConfig::default()
        })
        .build()
        .expect("machine");
        machine.mem_write(0x1000, b"rusty").expect("write");
        machine.mem_write(0x3F_FFF0, &[1, 2, 3]).expect("write");
        let path = scratch("whole.bin");

        let outcome = dump_ram(&mut machine, &path, || false);

        assert_eq!(
            outcome,
            DumpOutcome::Saved {
                path: path.clone(),
                bytes: 4 * 1024 * 1024
            }
        );
        let image = fs::read(&path).expect("the dump");
        assert_eq!(image.len(), 4 * 1024 * 1024);
        assert_eq!(&image[0x1000..0x1005], b"rusty");
        assert_eq!(&image[0x3F_FFF0..0x3F_FFF3], &[1, 2, 3]);
        assert!(!partial_path(&path).exists(), "no partial file is left");
        fs::remove_file(&path).expect("clean up");
    }

    /// A power-off during a dump does not wait for it: the dump stops at the
    /// next chunk, says why, and leaves no partial file.
    #[test]
    fn a_dump_the_vm_stops_during_gives_up_and_leaves_no_partial_file() {
        let mut machine = MachineBuilder::new(EmulatorConfig::default())
            .build()
            .expect("machine");
        let path = scratch("abandoned.bin");
        match dump_ram(&mut machine, &path, || true) {
            DumpOutcome::Failed { reason, .. } => {
                assert_eq!(reason, super::super::control::NOT_SAVED_VM_STOPPED);
            }
            DumpOutcome::Saved { .. } => panic!("an abandoned dump was saved"),
        }
        assert!(!path.exists());
        assert!(!partial_path(&path).exists());
    }

    #[test]
    fn a_dump_that_cannot_be_written_reports_why_and_leaves_no_partial_file() {
        let mut machine = MachineBuilder::new(EmulatorConfig::default())
            .build()
            .expect("machine");
        let path = scratch("no-such-folder").join("memory.bin");

        match dump_ram(&mut machine, &path, || false) {
            DumpOutcome::Failed {
                path: named,
                reason,
            } => {
                assert_eq!(named, path);
                assert!(!reason.is_empty());
            }
            DumpOutcome::Saved { .. } => panic!("a folder that does not exist took the dump"),
        }
        assert!(!partial_path(&path).exists());
    }
}
