//! Local CI gate suite for the clean-architecture refactor.
//!
//! `cargo xtask ci` runs the same load-bearing matrix as `.github/workflows/ci.yml`
//! plus the DLX Linux headless boot gate, so every refactor phase can be verified
//! with one command before review.
//!
//! `cargo xtask perf-baseline` builds the release perfbench binary and archives it
//! (with the git revision) under `target/perf-baselines/<rev>/` for interleaved
//! A/B comparison against later phases.

use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CiCommand {
    /// Also run the slow steps: full `cargo test -p rusty_box` (integration
    /// tests included) and the release GUI build.
    pub full: bool,
    /// Skip the DLX boot gate (needs the DLX disk image and several minutes).
    pub skip_boot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PerfBaselineCommand {}

/// The stdout marker the DLX headless boot prints when the guest reaches the
/// login prompt (see `rusty_box/examples/dlxlinux/dlxlinux.rs`).
const DLX_LOGIN_MARKER: &str = "*** LOGIN DETECTED ***";
/// Instruction budget that reliably reaches the DLX login prompt headlessly
/// (LILO Enter injected ~130M, `dlx login:` appears ~364M, margin on top).
const DLX_BOOT_BUDGET: &str = "450000000";

fn repo_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "xtask manifest directory has no parent".to_string())
}

struct Step {
    name: &'static str,
    args: &'static [&'static str],
    /// Extra environment variables for the child process.
    envs: &'static [(&'static str, &'static str)],
    /// Substring that must appear on stdout for the step to pass (in addition
    /// to a zero exit status). Steps with a marker run with captured output.
    stdout_marker: Option<&'static str>,
}

const MATRIX: &[Step] = &[
    Step {
        name: "decoder tests",
        args: &["test", "--release", "-p", "rusty_box_decoder"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "lib tests (default features)",
        args: &["test", "--release", "-p", "rusty_box", "--lib"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "no_std + no_alloc check",
        args: &["check", "--release", "-p", "rusty_box", "--no-default-features"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "no_std + alloc check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box",
            "--no-default-features",
            "--features",
            "alloc",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "bare-metal target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box",
            "--no-default-features",
            "--target",
            "x86_64-unknown-none",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "UEFI example build",
        args: &[
            "build",
            "--release",
            "-p",
            "rusty_box_uefi",
            "--target",
            "x86_64-unknown-uefi",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "wasm target check (lib, alloc)",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box",
            "--no-default-features",
            "--features",
            "alloc",
            "--target",
            "wasm32-unknown-unknown",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "all-features check",
        args: &["check", "--release", "-p", "rusty_box", "--all-features"],
        envs: &[],
        stdout_marker: None,
    },
];

const FULL_MATRIX: &[Step] = &[
    Step {
        name: "full test suite (integration included)",
        args: &["test", "--release", "-p", "rusty_box"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "GUI release build",
        args: &["build", "-p", "rusty_box_gui", "--release"],
        envs: &[],
        stdout_marker: None,
    },
];

const DLX_BOOT_GATE: Step = Step {
    name: "DLX Linux headless boot gate",
    args: &[
        "run",
        "--release",
        "-p",
        "rusty_box",
        "--example",
        "dlxlinux",
        "--features",
        "std",
    ],
    envs: &[
        ("RUSTY_BOX_HEADLESS", "1"),
        ("MAX_INSTRUCTIONS", DLX_BOOT_BUDGET),
    ],
    stdout_marker: Some(DLX_LOGIN_MARKER),
};

fn run_step(root: &PathBuf, step: &Step) -> Result<(), String> {
    let started = Instant::now();
    println!("==> {}", step.name);

    let mut command = Command::new("cargo");
    command.args(step.args).current_dir(root);
    for (key, value) in step.envs {
        command.env(key, value);
    }

    match step.stdout_marker {
        None => {
            let status = command
                .status()
                .map_err(|err| format!("{}: failed to spawn cargo: {err}", step.name))?;
            if !status.success() {
                return Err(format!("{}: cargo exited with {status}", step.name));
            }
        }
        Some(marker) => {
            command.stdout(Stdio::piped()).stderr(Stdio::inherit());
            let output = command
                .output()
                .map_err(|err| format!("{}: failed to spawn cargo: {err}", step.name))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !output.status.success() {
                let tail: Vec<&str> = stdout.lines().rev().take(20).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                return Err(format!(
                    "{}: cargo exited with {}\nlast output:\n{}",
                    step.name,
                    output.status,
                    tail.join("\n")
                ));
            }
            if !stdout.contains(marker) {
                let tail: Vec<&str> = stdout.lines().rev().take(20).collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                return Err(format!(
                    "{}: expected marker {marker:?} not found in output\nlast output:\n{}",
                    step.name,
                    tail.join("\n")
                ));
            }
        }
    }

    println!("<== {} ok ({:.1}s)", step.name, started.elapsed().as_secs_f32());
    Ok(())
}

pub fn execute_ci(command: CiCommand) -> Result<(), String> {
    let root = repo_root()?;
    let started = Instant::now();
    let mut ran = 0usize;

    for step in MATRIX {
        run_step(&root, step)?;
        ran += 1;
    }
    if command.full {
        for step in FULL_MATRIX {
            run_step(&root, step)?;
            ran += 1;
        }
    }
    if !command.skip_boot {
        run_step(&root, &DLX_BOOT_GATE)?;
        ran += 1;
    }

    println!(
        "ci: {ran} steps passed in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

pub fn execute_perf_baseline(_command: PerfBaselineCommand) -> Result<(), String> {
    let root = repo_root()?;

    let rev = {
        let output = Command::new("git")
            .args(["rev-parse", "--short=12", "HEAD"])
            .current_dir(&root)
            .output()
            .map_err(|err| format!("failed to run git rev-parse: {err}"))?;
        if !output.status.success() {
            return Err(format!("git rev-parse exited with {}", output.status));
        }
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    let status = Command::new("cargo")
        .args([
            "build", "--release", "-p", "rusty_box", "--example", "perfbench", "--features", "std",
        ])
        .current_dir(&root)
        .status()
        .map_err(|err| format!("failed to spawn cargo: {err}"))?;
    if !status.success() {
        return Err(format!("perfbench build exited with {status}"));
    }

    let exe = if cfg!(windows) {
        "perfbench.exe"
    } else {
        "perfbench"
    };
    let built = root
        .join("target")
        .join("release")
        .join("examples")
        .join(exe);
    if !built.exists() {
        return Err(format!("built perfbench not found at {}", built.display()));
    }

    let dest_dir = root.join("target").join("perf-baselines").join(&rev);
    std::fs::create_dir_all(&dest_dir)
        .map_err(|err| format!("failed to create {}: {err}", dest_dir.display()))?;
    let dest = dest_dir.join(exe);
    std::fs::copy(&built, &dest)
        .map_err(|err| format!("failed to copy perfbench to {}: {err}", dest.display()))?;

    println!("perf baseline archived: {}", dest.display());
    println!("A/B usage: alternate baseline and candidate binaries >= 10 pairs, compare medians.");
    Ok(())
}

pub fn parse_ci_args(args: &[String]) -> Result<CiCommand, String> {
    let mut command = CiCommand::default();
    for arg in args {
        match arg.as_str() {
            "--full" => command.full = true,
            "--skip-boot" => command.skip_boot = true,
            "--help" | "-h" => return Err(ci_usage()),
            other => return Err(format!("unknown ci option {other:?}\n{}", ci_usage())),
        }
    }
    Ok(command)
}

pub fn ci_usage() -> String {
    [
        "usage: cargo xtask ci [--full] [--skip-boot]",
        "  --full       also run the full integration test suite and GUI release build",
        "  --skip-boot  skip the DLX Linux headless boot gate",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ci_defaults() {
        assert_eq!(parse_ci_args(&[]).unwrap(), CiCommand::default());
    }

    #[test]
    fn parse_ci_flags() {
        let parsed = parse_ci_args(&["--full".into(), "--skip-boot".into()]).unwrap();
        assert_eq!(
            parsed,
            CiCommand {
                full: true,
                skip_boot: true,
            }
        );
    }

    #[test]
    fn parse_ci_rejects_unknown() {
        assert!(parse_ci_args(&["--bogus".into()]).is_err());
    }
}
