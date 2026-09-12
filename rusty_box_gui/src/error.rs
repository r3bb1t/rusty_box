use crate::args::BootDevice;
use std::{io, path::PathBuf};

/// Every place an engine is chosen, named by each message that refuses one.
/// A shell user cannot pass a flag, so a refusal naming the flag alone would
/// send them nowhere.
const ENGINE_CHOSEN_BY: &str = "chosen by `--engine`, a VM file's `emulator.engine`, or the \
                                shell's Hardware › Processors › Engine";

/// `flags`, each in backticks, joined with ", ".
fn quoted_flags(flags: &[&str]) -> String {
    flags
        .iter()
        .map(|flag| format!("`{flag}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("failed to read config {}: {source}", path.display())]
    ConfigRead { path: PathBuf, source: io::Error },

    #[error("failed to parse TOML config {}: {source}", path.display())]
    ConfigParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error(
        "BIOS path is required; pass --bios PATH, set rom.bios in the VM file, or set the BIOS \
         path under Hardware › Display"
    )]
    MissingBios,

    /// A run flag — `--engine`, `--cpu-capabilities` or `--log-level` — on a
    /// command line that names no machine. Each is one VM's own setting, so
    /// the command line is refused with the flags it set rather than opened
    /// on the library with them dropped.
    #[error(
        "{} without a machine: this command line sets how a machine runs but names none. Pass \
         --config FILE or --bios/--cdrom/--disk…, or set it per VM (in the shell: Hardware › \
         Processors › Engine, Hardware › Display › Log level; in the VM file: `emulator.engine`, \
         `emulator.cpu_capabilities`, `logging.level`)",
        quoted_flags(flags)
    )]
    RunFlagsNeedAMachine { flags: Vec<&'static str> },

    #[error("{field} must be greater than zero")]
    ZeroValue { field: &'static str },

    #[error("{field} is too large for this platform")]
    ValueOverflow { field: &'static str },

    #[error("invalid CPU topology: {message}")]
    InvalidCpuTopology { message: String },

    #[error("smp_quantum must be 1-32 (Bochs cpu: quantum=N), got {value}")]
    InvalidSmpQuantum { value: u32 },

    #[error("cpuid_freq must be hardware|none|ips (Bochs cpu: cpuid_freq=), got {value}")]
    InvalidCpuidFreq { value: String },

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

    #[error("the platform gave the app no storage directory for its ROMs and settings")]
    NoAppStorage,

    #[error("failed to place the carried {kind} at {}: {source}", path.display())]
    StageFile {
        kind: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    #[error(transparent)]
    Library(#[from] LibraryError),

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

    #[error("existing disk image {} is not a usable flat image ({len} bytes, not a non-zero multiple of 512); set disk.create.overwrite = true to replace it, or delete it", path.display())]
    InvalidExistingDiskImage { path: PathBuf, len: u64 },

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

    #[error(
        "this host has no Windows Hypervisor Platform, so the `whp` engine ({}) cannot run. \
         Enable it with: dism /Online /Enable-Feature /FeatureName:HypervisorPlatform",
        ENGINE_CHOSEN_BY
    )]
    NoHypervisor,

    /// The `whp` engine asked of a build that carries no hypervisor path. The
    /// path exists only on Windows, with `hv-whp` and without `guest-trace`;
    /// the gate is the one `runner.rs` compiles the engine under.
    #[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
    #[error(
        "this build has no hypervisor engine, so the `whp` engine ({}) cannot run. The engine is \
         built only for Windows, with the `hv-whp` feature and without `guest-trace`. Choose \
         `interpreter` there, or run a build that carries the engine",
        ENGINE_CHOSEN_BY
    )]
    NoHypervisorEngine,

    #[error(
        "max_instructions = {max_instructions} cannot be honoured by the `whp` engine ({}): a \
         processor on the hypervisor retires instructions the host does not count, so the limit \
         would end the run at a guess. Remove the limit or choose `interpreter` there",
        ENGINE_CHOSEN_BY
    )]
    InstructionBudgetOnHypervisor { max_instructions: u64 },

    #[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
    #[error("the machine on the hypervisor could not carry on: {source}")]
    Hypervisor {
        #[source]
        source: rusty_box_whp_engine::FastMachineFault,
    },

    #[error(transparent)]
    Emulator(#[from] rusty_box::Error),
}

/// What the VM library (`crate::library`) fails at. Defined here, with no
/// `#[cfg]`, so that `RunError::Library` exists in every build: a public
/// error has the same variants in every configuration (safety doctrine R0,
/// shape stability across features), even though the module producing this
/// one is native-only.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("failed to create the VM library folder {}: {source}", path.display())]
    CreateDir { path: PathBuf, source: io::Error },

    #[error("failed to read the VM library {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },

    #[error("failed to write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },

    #[error("failed to delete {}: {source}", path.display())]
    Delete { path: PathBuf, source: io::Error },

    #[error("{} is not a file of the VM library in {}", path.display(), dir.display())]
    OutsideLibrary { path: PathBuf, dir: PathBuf },

    #[error("{text:?} is not a VM file stem: one file name, no folder, not starting with a dot")]
    InvalidStem { text: String },

    #[error("failed to make {} absolute: {source}", path.display())]
    AbsolutePath { path: PathBuf, source: io::Error },

    #[error("failed to serialize VM {name}: {source}")]
    Serialize {
        name: String,
        source: toml::ser::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The places an engine is chosen, every one of which a refusal names:
    /// a shell user cannot pass a flag, so a message naming the flag alone
    /// would send them nowhere.
    const ENGINE_SOURCES: [&str; 3] = [
        "--engine",
        "emulator.engine",
        "Hardware › Processors › Engine",
    ];

    #[test]
    fn every_engine_refusal_names_each_place_an_engine_is_chosen() {
        #[cfg(not(all(not(feature = "guest-trace"), feature = "hv-whp", windows)))]
        let refusals = [
            RunError::NoHypervisor,
            RunError::InstructionBudgetOnHypervisor {
                max_instructions: 1_000,
            },
            RunError::NoHypervisorEngine,
        ];
        #[cfg(all(not(feature = "guest-trace"), feature = "hv-whp", windows))]
        let refusals = [
            RunError::NoHypervisor,
            RunError::InstructionBudgetOnHypervisor {
                max_instructions: 1_000,
            },
        ];

        for refusal in refusals {
            let text = refusal.to_string();
            for source in ENGINE_SOURCES {
                assert!(text.contains(source), "{text:?} does not name {source}");
            }
            assert!(!text.contains("--engine whp"), "{text:?} names only the flag");
        }
    }

    #[test]
    fn a_run_flag_refusal_names_every_flag_and_where_a_machine_comes_from() {
        let text = RunError::RunFlagsNeedAMachine {
            flags: vec!["--engine", "--cpu-capabilities", "--log-level"],
        }
        .to_string();

        for flag in ["`--engine`", "`--cpu-capabilities`", "`--log-level`"] {
            assert!(text.contains(flag), "{text:?} does not name {flag}");
        }
        assert!(text.contains("--config FILE"), "{text:?} does not say how to name a machine");
        assert!(text.contains("Hardware › Processors › Engine"));
        assert!(text.contains("Hardware › Display › Log level"));
        // The shell has no processor-capabilities control; the VM file does.
        assert!(text.contains("emulator.cpu_capabilities"), "{text:?}");
    }

    #[test]
    fn a_missing_bios_is_reported_by_its_first_words_and_points_at_the_shell_setting() {
        let text = RunError::MissingBios.to_string();

        assert!(text.starts_with("BIOS path is required"), "{text:?}");
        assert!(text.contains("Hardware › Display"), "{text:?}");
    }
}
