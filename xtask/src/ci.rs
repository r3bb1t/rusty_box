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

/// Doctrine R1/R6 ratchet baselines (docs/safety-doctrine.md). Counts may only
/// DECREASE; a commit that removes unsafe tightens the matching constant in the
/// same commit. An increase fails ci.
const UNSAFE_TOKEN_BASELINES: &[(&str, usize)] = &[
    // 254 -> 249: the memory stub's swap-file `UnsafeCell` is gone, and with it
    // the four `&self`-to-`&mut File` launderings plus the three test pokes.
    // 249 -> 222: the `CpuTlbPin` sidecar is gone — its `UnsafeCell` state, the
    // publication paths that wrote through it, and the `from_raw_parts`
    // reconstructions that rebuilt the pin slice at every scheduler slice.
    // 222 -> 212: the fetch and async-event entry points hold memory as a
    // borrow, so the raw `*mut BxMemC` re-borrows that fed `prefetch`,
    // `serve_icache_miss` and the icache tests are gone.
    // 212 -> 176: `BxCpuC` no longer stores its bus at all. The `mem_bus`,
    // `io_bus` and `pc_system_ptr` pointers, the accessors that laundered
    // each deref, and the wiring that installed and cleared them are gone;
    // an `ExecCtx` holds the machine as borrows instead.
    // 176 -> 116: `run_cpu_batch` and `inject_interrupt` are safe fns. Their
    // bodies' remaining raw wiring is device-side and named by SAFETY blocks
    // that own it; the ~58 `unsafe { … }` wrappers at their call sites, and
    // two `'a: 'static` bounds justified by a deleted helper, are gone.
    // 116 -> 115: the device manager reaches port and MMIO dispatch as a
    // borrow on `ExecCtx`, so `BxDevicesC`'s stored pointer, its set/clear
    // pair and the accessor that laundered each deref are gone.
    // 115 -> 103: the machine holds no pointer to any of its own parts. A port
    // write carries the memory borrow fw_cfg's DMA descriptor needs, the PIC
    // and DMA controllers come off `ExecCtx` instead of aliasing the device
    // manager the slice already borrows, and the SMC drain reads memory
    // through a context. `Emulator` derives `Send`.
    // 103 -> 97: the machine's CPUs live in one store, boot processor at index
    // 0. `BspCpu`'s pointer newtype and its two `Deref` launderings are gone,
    // as is the no-alloc `[*mut BxCpuC; 253]` and the three dereferences that
    // read it. Every build's machine now derives `Send`.
    ("rusty_box/src", 97),
    ("rusty_box_decoder/src", 0),
    // Zero, and structurally so: the crate carries `#![forbid(unsafe_code)]`,
    // which the workspace lint table backs with a `deny` any new crate inherits.
    ("rusty_box_core/src", 0),
    // Zero, and structurally so, for the same reason as core: the crate carries
    // `#![forbid(unsafe_code)]`. A device model has no business with raw memory
    // — it is handed what it may touch.
    ("rusty_box_devices/src", 0),
    // The host-FFI leaf, and the one crate whose baseline is not zero and is
    // not aiming there: calling the WinHvPlatform C API is its whole purpose.
    // What the ratchet enforces here is CONFINEMENT — every one of these lives
    // in `sys/windows.rs`, which is the only file that lifts the workspace's
    // `deny(unsafe_code)`, and each block names the invariant it rests on. A
    // rise means either a new platform call or unsafe that escaped the seam.
    ("rusty_box_whp/src", 30),
];
/// `unsafe impl … Send/Sync` lines in rusty_box/src. Zero, permanently: thread
/// safety is derived from ownership, and `Emulator`'s `const` assertion in
/// emulator/mod.rs fails the build if a field ever takes that away (R6).
const UNSAFE_IMPL_SEND_BASELINE: usize = 0;

/// Files carrying a crate-inner `#![allow(… dead_code …)]`, which switches the
/// lint off for the whole file — and, in a `mod.rs`, for every module beneath
/// it. Ratcheted for the same reason as unsafe: not because the items it hides
/// are wrong, but because while it is on, nothing tells you a NEW one appeared.
///
/// Measured with `RUSTFLAGS="--force-warn dead_code"`, these hide ~2470 items.
/// Most are Bochs-parity constants and helpers ported ahead of their callers —
/// `tables.rs` alone accounts for ~141, and `geforce.rs` ~23 in a file that is
/// deliberately kept — so the backlog needs per-item judgement and cannot be
/// swept. The count going down is what makes that judgement happen file by
/// file; replace a blanket allow with targeted ones that name their provenance.
// 73 -> 70: `memory/mod.rs` (whose inner attribute covered the whole memory
// subtree, `residency.rs` included), `memory/memory_stub.rs`, and `cpu/tlb.rs`.
// Between them they hid a callerless `Tlb::pinned_alloc_offset` left over from
// the deleted pin sidecar, a 4 KiB never-read `apic_scratch` buffer, and a
// forwarder with no callers.
const BLANKET_DEAD_CODE_BASELINE: usize = 70;

/// Count occurrences of a bare `unsafe` token per crate, comment lines
/// stripped, against the ratchet baselines.
fn doctrine_ratchets(root: &PathBuf) -> Result<(), String> {
    let started = Instant::now();
    println!("==> doctrine ratchets");

    fn is_ident(b: u8) -> bool {
        b == b'_' || b.is_ascii_alphanumeric()
    }
    /// Occurrences of `needle` as a whole word in `line`, ignoring anything
    /// after `//`. Good enough for a monotone ratchet; not a parser.
    fn word_count(line: &str, needle: &str) -> usize {
        let code = line.split("//").next().unwrap_or("");
        let bytes = code.as_bytes();
        let mut n = 0;
        let mut at = 0;
        while let Some(pos) = code[at..].find(needle) {
            let start = at + pos;
            let end = start + needle.len();
            let left_ok = start == 0 || !is_ident(bytes[start - 1]);
            let right_ok = end >= bytes.len() || !is_ident(bytes[end]);
            if left_ok && right_ok {
                n += 1;
            }
            at = end;
        }
        n
    }
    /// Whether `line` is a crate-inner attribute switching `dead_code` off for
    /// a whole file. Matches the `#![allow(…)]` form only: a targeted
    /// `#[allow(dead_code)]` on one item is the shape this ratchet is pushing
    /// the tree towards, so it must not be counted.
    fn is_blanket_dead_code_allow(line: &str) -> bool {
        let trimmed = line.trim_start();
        trimmed.starts_with("#![allow") && trimmed.contains("dead_code")
    }

    fn scan_dir(
        dir: &std::path::Path,
        unsafe_tokens: &mut usize,
        unsafe_impl_send: &mut usize,
        blanket_dead_code: &mut usize,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|err| format!("doctrine ratchets: read_dir {}: {err}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|err| format!("doctrine ratchets: dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                scan_dir(&path, unsafe_tokens, unsafe_impl_send, blanket_dead_code)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).map_err(|err| {
                    format!("doctrine ratchets: read {}: {err}", path.display())
                })?;
                // One per FILE, not per line: a file may carry several inner
                // attributes, and what is being counted is files whose dead
                // code is invisible.
                if text.lines().any(is_blanket_dead_code_allow) {
                    *blanket_dead_code += 1;
                }
                for line in text.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    *unsafe_tokens += word_count(line, "unsafe");
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("unsafe impl")
                        && (code.contains("Send") || code.contains("Sync"))
                    {
                        *unsafe_impl_send += 1;
                    }
                }
            }
        }
        Ok(())
    }

    let mut total_impl_send = 0usize;
    let mut total_blanket_dead_code = 0usize;
    for (rel, baseline) in UNSAFE_TOKEN_BASELINES {
        let mut tokens = 0usize;
        let mut impl_send = 0usize;
        let mut blanket_dead_code = 0usize;
        scan_dir(&root.join(rel), &mut tokens, &mut impl_send, &mut blanket_dead_code)?;
        if *rel == "rusty_box/src" {
            total_impl_send = impl_send;
            total_blanket_dead_code = blanket_dead_code;
        }
        if tokens > *baseline {
            return Err(format!(
                "doctrine ratchets: {rel} has {tokens} `unsafe` tokens, baseline is {baseline} \
                 (R1: unsafe only ratchets down — remove the new unsafe or justify + re-baseline \
                 in this commit)"
            ));
        }
        if tokens < *baseline {
            println!(
                "    {rel}: {tokens} unsafe tokens (< baseline {baseline} — tighten \
                 UNSAFE_TOKEN_BASELINES in this commit)"
            );
        }
    }
    if total_impl_send > UNSAFE_IMPL_SEND_BASELINE {
        return Err(format!(
            "doctrine ratchets: {total_impl_send} `unsafe impl Send/Sync` lines, baseline is \
             {UNSAFE_IMPL_SEND_BASELINE} (R6: Send derives, never promised)"
        ));
    }
    if total_blanket_dead_code > BLANKET_DEAD_CODE_BASELINE {
        return Err(format!(
            "doctrine ratchets: {total_blanket_dead_code} files carry a blanket \
             `#![allow(… dead_code …)]`, baseline is {BLANKET_DEAD_CODE_BASELINE} — a new one \
             hides every unused item in its file (and, from a mod.rs, in every module below it). \
             Put the allow on the item that needs it and say why."
        ));
    }
    if total_blanket_dead_code < BLANKET_DEAD_CODE_BASELINE {
        println!(
            "    rusty_box/src: {total_blanket_dead_code} blanket dead_code allows \
             (< baseline {BLANKET_DEAD_CODE_BASELINE} — tighten BLANKET_DEAD_CODE_BASELINE \
             in this commit)"
        );
    }

    println!(
        "<== doctrine ratchets ok ({:.1}s)",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

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
    // The core crate is proven on its own, in every configuration it ships,
    // BEFORE anything that depends on it: a foundation that only compiles as
    // part of its consumer is not a foundation. Its default build is the
    // strictest one (no_std, no alloc), so the additive features are checked
    // for what they add rather than for making it work.
    Step {
        name: "core tests (no_std + no_alloc)",
        args: &["test", "--release", "-p", "rusty_box_core"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "core tests (alloc)",
        args: &["test", "--release", "-p", "rusty_box_core", "--features", "alloc"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "core tests (std)",
        args: &["test", "--release", "-p", "rusty_box_core", "--features", "std"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "core bare-metal target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box_core",
            "--target",
            "x86_64-unknown-none",
        ],
        envs: &[],
        stdout_marker: None,
    },
    // The device models, under the same rule and for the same reason: their
    // whole claim is that they need no host and no execution engine, and a
    // build that proves it has to happen without one in the room.
    Step {
        name: "devices tests (no_std + no_alloc)",
        args: &["test", "--release", "-p", "rusty_box_devices"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "devices tests (std)",
        args: &["test", "--release", "-p", "rusty_box_devices", "--features", "std"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "devices bare-metal target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box_devices",
            "--features",
            "alloc",
            "--target",
            "x86_64-unknown-none",
        ],
        envs: &[],
        stdout_marker: None,
    },
    // The Windows Hypervisor Platform leaf. Its tests need no hypervisor —
    // they cover the bitfield layouts transcribed from the SDK, which is
    // exactly the part a reader cannot check by eye — so this step runs
    // everywhere, including on a host where `hypervisor_present()` is false.
    // Building the probe too, because an example outside the gate is an
    // example that rots.
    Step {
        name: "WHP leaf tests",
        args: &["test", "--release", "-p", "rusty_box_whp"],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        name: "WHP probe builds",
        args: &["check", "--release", "-p", "rusty_box_whp", "--examples"],
        envs: &[],
        stdout_marker: None,
    },
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
        // The lib tests reach into the crate's own internals; this compiles
        // and runs the public API the way a consumer writes it, which is the
        // only thing that catches a surface that is correct inside and
        // unusable outside.
        name: "public API examples",
        args: &[
            "test",
            "--release",
            "-p",
            "rusty_box",
            "--features",
            "std",
            "--test",
            "api_examples",
        ],
        envs: &[],
        stdout_marker: None,
    },
    Step {
        // The GUI's browser front end is the only consumer of several
        // `#[cfg(target_arch = "wasm32")]` code paths, so no other step in
        // this suite compiles them. One such path went stale unnoticed.
        name: "GUI wasm target check",
        args: &[
            "check",
            "--release",
            "-p",
            "rusty_box_gui",
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

    doctrine_ratchets(&root)?;
    ran += 1;

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
