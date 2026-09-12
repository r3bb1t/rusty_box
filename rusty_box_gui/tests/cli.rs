use clap::Parser;
use rusty_box_bximage::ImageSize;
use rusty_box_gui::{Args, DiskGeometry, DisplayBackend};

#[test]
fn parses_core_runner_flags() {
    let args = Args::try_parse_from([
        "rusty_box_gui",
        "--no-config",
        "--display",
        "headless",
        "--bios",
        "bios.bin",
        "--disk",
        "disk.img",
        "--disk-chs",
        "306:4:17",
        "--boot",
        "disk",
        "--memory-mib",
        "64",
        "--ips",
        "15000000",
        "--max-instructions",
        "1000",
    ])
    .unwrap();

    assert!(args.no_config);
    assert_eq!(args.display, Some(DisplayBackend::Headless));
    assert_eq!(args.bios.unwrap().as_os_str(), "bios.bin");
    assert_eq!(args.disk.unwrap().as_os_str(), "disk.img");
    assert_eq!(
        args.disk_chs,
        Some(DiskGeometry {
            cylinders: 306,
            heads: 4,
            sectors_per_track: 17,
        })
    );
    assert_eq!(args.memory_mib, Some(64));
    assert_eq!(args.ips, Some(15_000_000));
    assert_eq!(args.max_instructions, Some(1000));
}

#[test]
fn config_and_no_config_conflict() {
    let result = Args::try_parse_from(["rusty_box_gui", "--config", "a.toml", "--no-config"]);

    assert!(result.is_err());
}

#[test]
fn pci_negation_is_explicit() {
    let args = Args::try_parse_from(["rusty_box_gui", "--no-pci"]).unwrap();

    assert!(args.no_pci);
    assert!(!args.pci);
}

#[test]
fn parses_create_disk_flags() {
    let args = Args::try_parse_from([
        "rusty_box_gui",
        "--create-disk",
        "c.img",
        "--create-disk-size",
        "20G",
        "--overwrite-created-disk",
    ])
    .unwrap();

    assert_eq!(args.create_disk.unwrap().as_os_str(), "c.img");
    assert_eq!(args.create_disk_size.unwrap().0, ImageSize::gib(20));
    assert!(args.overwrite_created_disk);
}

#[test]
fn create_disk_conflicts_with_existing_disk() {
    let result = Args::try_parse_from([
        "rusty_box_gui",
        "--disk",
        "existing.img",
        "--create-disk",
        "c.img",
    ]);

    assert!(result.is_err());
}

#[test]
fn create_disk_size_conflicts_with_disk_chs() {
    let result = Args::try_parse_from([
        "rusty_box_gui",
        "--create-disk-size",
        "20G",
        "--disk-chs",
        "20:16:63",
    ]);

    assert!(result.is_err());
}

#[cfg(feature = "gui-egui")]
#[test]
fn parses_egui_display_backend_when_feature_enabled() {
    let args = Args::try_parse_from(["rusty_box_gui", "--display", "egui"]).unwrap();

    assert_eq!(args.display, Some(DisplayBackend::Egui));
}

#[cfg(not(feature = "gui-egui"))]
#[test]
fn rejects_egui_display_backend_without_feature() {
    assert!(Args::try_parse_from(["rusty_box_gui", "--display", "egui"]).is_err());
}

/// A scratch folder, removed when the test ends.
struct ScratchDir {
    root: std::path::PathBuf,
}

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rusty-box-gui-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create the scratch folder");
        Self { root }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            eprintln!("could not remove {}: {error}", self.root.display());
        }
    }
}

/// A scratch folder with a planted `rusty_box.toml` and an empty `child`
/// folder under it, removed when the test ends.
struct PlantedConfig {
    dir: ScratchDir,
}

impl PlantedConfig {
    fn new(tag: &str) -> Self {
        let dir = ScratchDir::new(tag);
        std::fs::create_dir_all(dir.root.join("child")).expect("create the child folder");
        std::fs::write(
            dir.root.join("rusty_box.toml"),
            "[rom]\nbios = \"planted.bin\"\n\n[cdrom]\npath = \"planted.iso\"\n",
        )
        .expect("plant rusty_box.toml");
        Self { dir }
    }

    fn root(&self) -> &std::path::Path {
        &self.dir.root
    }
}

/// Runs the launcher from `dir` with no config file named, and returns what
/// it printed on stderr. A planted file that were read would name a BIOS the
/// launcher then fails to read; unread, the command line names no BIOS.
fn launch_from(dir: &std::path::Path) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rusty_box_gui"))
        .args(["--display", "headless"])
        .current_dir(dir)
        .output()
        .expect("run rusty_box_gui");
    assert!(!output.status.success(), "the launcher started without a BIOS");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_rusty_box_toml_in_the_working_directory_is_not_read() {
    let planted = PlantedConfig::new("cwd-config");

    let stderr = launch_from(planted.root());

    assert!(stderr.contains("BIOS path is required"), "stderr: {stderr}");
}

#[test]
fn a_rusty_box_toml_in_the_parent_directory_is_not_read() {
    let planted = PlantedConfig::new("parent-config");

    let stderr = launch_from(&planted.root().join("child"));

    assert!(stderr.contains("BIOS path is required"), "stderr: {stderr}");
}

/// How long the launcher gets to refuse before the test gives up on it. The
/// refusal returns in milliseconds; a launcher still running after this has
/// opened the shell instead.
#[cfg(feature = "gui-egui")]
const REFUSAL_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Waits for `child` to exit within `limit`. A child still running after
/// that is killed and the test fails, saying whether the kill succeeded.
#[cfg(feature = "gui-egui")]
fn wait_bounded(
    child: &mut std::process::Child,
    limit: std::time::Duration,
) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => match child.kill() {
                Ok(()) => panic!("the launcher did not exit within {limit:?}; it was killed"),
                Err(error) => panic!(
                    "the launcher did not exit within {limit:?}, and killing it failed: {error}"
                ),
            },
            Err(error) => panic!("could not poll the launcher: {error}"),
        }
    }
}

/// A run flag with no machine to apply it to is refused before the shell
/// opens, naming the flag, rather than dropped on the way to the library.
/// The library folder is derived from the environment given here, so a
/// regressed refusal that reached the shell would open on a scratch library,
/// never the user's own, and would be killed at the bound rather than waited
/// on for ever.
#[cfg(feature = "gui-egui")]
#[test]
fn a_run_flag_without_a_machine_is_refused_before_the_shell_opens() {
    let scratch = ScratchDir::new("run-flag-refusal");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rusty_box_gui"))
        .args(["--engine", "whp", "--log-level", "debug"])
        .env("APPDATA", &scratch.root)
        .env("XDG_DATA_HOME", &scratch.root)
        .env("HOME", &scratch.root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run rusty_box_gui");

    let status = wait_bounded(&mut child, REFUSAL_LIMIT);
    let output = child
        .wait_with_output()
        .expect("collect the launcher's output");

    assert!(!status.success(), "the launcher opened with a machine-less run flag");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`--engine`, `--log-level` without a machine"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("--config FILE"), "stderr: {stderr}");
}
