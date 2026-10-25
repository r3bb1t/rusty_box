//! A VM's RAM written to a file, whole and in the machine's RAM order. A
//! PC's is its RAM store's (`Emulator::read_ram_image`): below 3 GiB the
//! file offset is the physical address; RAM from 4 GiB up follows straight
//! after. An Arm `virt` board's RAM is one run from its base, 0x4000_0000, so
//! the file offset is the distance from there.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use rusty_box::cpu::Instrumentation;
use rusty_box::emulator::{Emulator, RamOffset, SliceEngine};

use super::control::DumpOutcome;

/// How much RAM is read and written at once.
const DUMP_CHUNK_BYTES: usize = 1 << 20;

/// A byte's place in a machine's RAM image, in the order its dump file keeps
/// (R4: an offset, not a guest address).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ImageOffset(pub(crate) u64);

/// A machine's RAM as a dump reads it: how much there is, and reads of it
/// in the order the dump file keeps.
pub(crate) trait RamImage {
    /// Why a read failed.
    type Error: std::fmt::Display;

    /// The RAM's size in bytes.
    fn ram_len(&self) -> u64;

    /// Fills `buf` from the RAM, starting at `at`.
    fn read_ram(&mut self, at: ImageOffset, buf: &mut [u8]) -> Result<(), Self::Error>;
}

/// A PC's image is its RAM store's order, [`RamOffset`]'s.
impl<T: Instrumentation, E: SliceEngine<T>> RamImage for Emulator<T, E> {
    type Error = rusty_box::Error;

    fn ram_len(&self) -> u64 {
        self.mem_size() as u64
    }

    fn read_ram(&mut self, at: ImageOffset, buf: &mut [u8]) -> Result<(), Self::Error> {
        self.read_ram_image(RamOffset(at.0), buf)
    }
}

/// An Arm board's image is its one run of RAM, from the RAM's base.
impl<S, I> RamImage for rusty_box_aarch64::VirtMachine<S, I>
where
    S: rusty_box_aarch64::GuestStorage,
    I: rusty_box_aarch64::Instrumentation,
{
    type Error = VirtRamOutOfRange;

    fn ram_len(&self) -> u64 {
        self.ram().len() as u64
    }

    fn read_ram(&mut self, at: ImageOffset, buf: &mut [u8]) -> Result<(), Self::Error> {
        let source = usize::try_from(at.0)
            .ok()
            .and_then(|start| self.ram().get(start..start.checked_add(buf.len())?))
            .ok_or(VirtRamOutOfRange { at })?;
        buf.copy_from_slice(source);
        Ok(())
    }
}

/// A read past the end of an Arm machine's RAM.
#[derive(Debug, thiserror::Error)]
#[error("RAM image offset {:#x} is past the end of the machine's RAM", at.0)]
pub(crate) struct VirtRamOutOfRange {
    at: ImageOffset,
}

#[derive(Debug, thiserror::Error)]
enum DumpError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("guest RAM could not be read: {0}")]
    Read(String),
    /// The run was told to end while the dump was being written.
    #[error("{}", super::control::NOT_SAVED_VM_STOPPED)]
    Abandoned,
}

/// Writes all of `machine`'s RAM to `path`, giving up between chunks once
/// `abandoned` answers true (the VM is being powered off or the app closed,
/// which must not wait for several GiB). The bytes go to a `.partial` file
/// beside it first, which takes `path`'s name only once it is whole, so a
/// failed dump never leaves a short file under the name asked for.
pub(crate) fn dump_ram<M: RamImage, F: Fn() -> bool>(
    machine: &mut M,
    path: &Path,
    abandoned: F,
) -> DumpOutcome {
    let partial = partial_path(path);
    let written = write_ram(machine, &partial, abandoned).and_then(|bytes| {
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

fn write_ram<M: RamImage, F: Fn() -> bool>(
    machine: &mut M,
    partial: &Path,
    abandoned: F,
) -> Result<u64, DumpError> {
    let total = machine.ram_len();
    let mut out = BufWriter::new(File::create(partial)?);
    let mut chunk = vec![0u8; DUMP_CHUNK_BYTES];
    let mut offset = 0u64;
    while offset < total {
        if abandoned() {
            return Err(DumpError::Abandoned);
        }
        let len = usize::try_from(total - offset)
            .map_or(DUMP_CHUNK_BYTES, |left| left.min(DUMP_CHUNK_BYTES));
        let buf = &mut chunk[..len];
        machine
            .read_ram(ImageOffset(offset), buf)
            .map_err(|error| DumpError::Read(error.to_string()))?;
        out.write_all(buf)?;
        offset += len as u64;
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

        let outcome = dump_ram(machine.as_mut(), &path, || false);

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
        match dump_ram(machine.as_mut(), &path, || true) {
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

        match dump_ram(machine.as_mut(), &path, || false) {
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
