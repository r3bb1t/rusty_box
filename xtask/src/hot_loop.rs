//! The AArch64 trace loop's host instructions, held against a committed
//! baseline.
//!
//! `VirtMachine::run_until` dispatches every guest instruction, and which
//! of its values LLVM keeps in registers moves with changes far from it: a
//! struct's field order, a new op arm. Timings on a shared host swing by
//! more than such an effect, so the measure is the host instructions
//! executed inside `run_until` (callees included), as callgrind counts them
//! (`--toggle-collect=*run_until*`). The count is deterministic: the same
//! binary on the same work counts the same on every run.
//!
//! The five `aarch64_perfbench` loops (`alu`, `memcpy`, `fnv`, `fp`, and
//! `mmio`, whose every instruction takes the per-instruction path) run
//! plain and hooked, ten counts. A count more than [`RISE_LIMIT_PERCENT`]
//! above its baseline fails the step; one that falls by more says so, and
//! the baseline is refreshed to keep the gain.
//!
//! The counts belong to the code the compiler made for one host target,
//! and the `fp` loop's to whether the host has fused multiply-add (glibc's
//! `fma`, which the loop calls, picks its code by it). The baseline names
//! all three; the step measures only where they match, and is skipped,
//! saying why, where they do not or where callgrind cannot run (valgrind
//! does not exist on Windows). `cargo xtask hot-loop-baseline` measures
//! and rewrites the baseline.

use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    thread,
};

/// The committed baseline, relative to the repository root.
const BASELINE: &str = "xtask/aarch64_hot_loop_baseline.txt";
/// The largest rise over the baseline a count may show, in percent.
const RISE_LIMIT_PERCENT: f64 = 0.5;
/// The loops measured, as `PERFBENCH_WORKLOADS` names them.
const LOOPS: [&str; 5] = ["alu", "memcpy", "fnv", "fp", "mmio"];
/// The observers they run with, as `PERFBENCH_HOOKS` names them.
const HOOKS: [&str; 2] = ["plain", "hooked"];

/// What the counts depend on besides the source: the compiler, the host
/// target it builds for, and the host's fused multiply-add.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Toolchain {
    /// `rustc -vV`'s `commit-hash`, which also fixes its LLVM.
    rustc: String,
    /// `rustc -vV`'s `host`.
    host: String,
    fma: bool,
}

impl fmt::Display for Toolchain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rustc {}, host {}, fma {}",
            self.rustc,
            self.host,
            yes_no(self.fma)
        )
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// One loop's count under one observer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Count {
    workload: &'static str,
    hooks: &'static str,
    instructions: u64,
}

/// A set of counts and the toolchain they were made with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Counts {
    toolchain: Toolchain,
    cells: Vec<Count>,
}

impl Counts {
    fn get(&self, workload: &str, hooks: &str) -> Option<u64> {
        self.cells
            .iter()
            .find(|c| c.workload == workload && c.hooks == hooks)
            .map(|c| c.instructions)
    }

    fn render(&self) -> String {
        let mut text = String::from(
            "# Host instructions inside VirtMachine::run_until for the aarch64_perfbench\n\
             # loops (callgrind, --toggle-collect=*run_until*), held by `cargo xtask ci`'s\n\
             # \"aarch64 hot-loop host instructions\" step. `cargo xtask hot-loop-baseline`\n\
             # rewrites this file: see docs/aarch64-automation.md.\n",
        );
        text.push_str(&format!("rustc {}\n", self.toolchain.rustc));
        text.push_str(&format!("host {}\n", self.toolchain.host));
        text.push_str(&format!("fma {}\n", yes_no(self.toolchain.fma)));
        for cell in &self.cells {
            text.push_str(&format!(
                "{} {} {}\n",
                cell.workload, cell.hooks, cell.instructions
            ));
        }
        text
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut rustc = None;
        let mut host = None;
        let mut fma = None;
        let mut cells = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["rustc", hash] => rustc = Some(hash.to_string()),
                ["host", triple] => host = Some(triple.to_string()),
                ["fma", "yes"] => fma = Some(true),
                ["fma", "no"] => fma = Some(false),
                [workload, hooks, count] => {
                    let workload = LOOPS
                        .iter()
                        .copied()
                        .find(|w| w == workload)
                        .ok_or_else(|| format!("unknown loop {workload:?}"))?;
                    let hooks = HOOKS
                        .iter()
                        .copied()
                        .find(|h| h == hooks)
                        .ok_or_else(|| format!("unknown observer {hooks:?}"))?;
                    let instructions = count
                        .parse::<u64>()
                        .map_err(|e| format!("count {count:?}: {e}"))?;
                    cells.push(Count {
                        workload,
                        hooks,
                        instructions,
                    });
                }
                _ => return Err(format!("unreadable line {line:?}")),
            }
        }
        let toolchain = Toolchain {
            rustc: rustc.ok_or("no `rustc` line")?,
            host: host.ok_or("no `host` line")?,
            fma: fma.ok_or("no `fma` line")?,
        };
        let counts = Self { toolchain, cells };
        for workload in LOOPS {
            for hooks in HOOKS {
                if counts.get(workload, hooks).is_none() {
                    return Err(format!("no count for {workload} {hooks}"));
                }
            }
        }
        Ok(counts)
    }
}

/// valgrind is not installed or does not run, so nothing can be counted:
/// why, as the attempt to run it reported.
#[derive(Debug)]
struct NoValgrind(String);

impl fmt::Display for NoValgrind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "valgrind does not run on this host ({}); callgrind counts the \
             instructions, and valgrind does not exist on Windows",
            self.0
        )
    }
}

fn valgrind_runs() -> Result<(), NoValgrind> {
    match Command::new("valgrind")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(NoValgrind(format!(
            "`valgrind --version` exited with {status}"
        ))),
        Err(err) => Err(NoValgrind(err.to_string())),
    }
}

fn host_has_fma() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn toolchain(root: &Path) -> Result<Toolchain, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .current_dir(root)
        .output()
        .map_err(|err| format!("failed to run rustc -vV: {err}"))?;
    if !output.status.success() {
        return Err(format!("rustc -vV exited with {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|value| value.trim().to_string())
            .ok_or_else(|| format!("rustc -vV printed no `{name}` line"))
    };
    Ok(Toolchain {
        rustc: field("commit-hash:")?,
        host: field("host:")?,
        fma: host_has_fma(),
    })
}

fn target_dir(root: &Path) -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                root.join(dir)
            }
        }
        None => root.join("target"),
    }
}

/// Build the perfbench with `cargo`, a command in the repository root whose
/// environment is the caller's to set.
fn build_perfbench(root: &Path, mut cargo: Command) -> Result<PathBuf, String> {
    let status = cargo
        .args([
            "build",
            "--release",
            "-p",
            "rusty_box_aarch64",
            "--features",
            "std",
            "--example",
            "aarch64_perfbench",
        ])
        .current_dir(root)
        .status()
        .map_err(|err| format!("failed to spawn cargo: {err}"))?;
    if !status.success() {
        return Err(format!("aarch64_perfbench build exited with {status}"));
    }
    let exe = format!("aarch64_perfbench{}", env::consts::EXE_SUFFIX);
    let built = target_dir(root).join("release").join("examples").join(exe);
    if !built.exists() {
        return Err(format!("built aarch64_perfbench not found at {}", built.display()));
    }
    Ok(built)
}

/// Callgrind's total for one run: the `summary:` (or `totals:`) line of its
/// output file.
fn callgrind_total(file: &Path) -> Result<u64, String> {
    let text = fs::read_to_string(file)
        .map_err(|err| format!("failed to read {}: {err}", file.display()))?;
    text.lines()
        .find_map(|line| {
            line.strip_prefix("summary:")
                .or_else(|| line.strip_prefix("totals:"))
        })
        .and_then(|rest| rest.split_whitespace().next())
        .ok_or_else(|| format!("{} holds no summary line", file.display()))?
        .parse::<u64>()
        .map_err(|err| format!("{}: summary: {err}", file.display()))
}

fn count_one(binary: &Path, out_dir: &Path, workload: &'static str, hooks: &'static str) -> Result<Count, String> {
    let file = out_dir.join(format!("callgrind.{workload}.{hooks}.out"));
    let output = Command::new("valgrind")
        .arg("--tool=callgrind")
        .arg(format!("--callgrind-out-file={}", file.display()))
        .arg("--toggle-collect=*run_until*")
        .arg(binary)
        .env("PERFBENCH_WORKLOADS", workload)
        .env("PERFBENCH_HOOKS", hooks)
        .env("PERFBENCH_SCALE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("failed to spawn valgrind: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(10).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "{workload} {hooks}: valgrind exited with {}\n{}",
            output.status,
            tail.join("\n")
        ));
    }
    Ok(Count {
        workload,
        hooks,
        instructions: callgrind_total(&file)?,
    })
}

/// Build the perfbench with `cargo` and count the ten cells, as many at
/// once as the host has threads.
fn measure(root: &Path, toolchain: Toolchain, cargo: Command) -> Result<Counts, String> {
    let binary = build_perfbench(root, cargo)?;
    let out_dir = target_dir(root).join("hot-loop");
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;
    let jobs: Vec<(&'static str, &'static str)> = LOOPS
        .iter()
        .flat_map(|&w| HOOKS.iter().map(move |&h| (w, h)))
        .collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Result<Count, String>>> = Mutex::new(Vec::new());
    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(jobs.len());
    println!("counting {} runs under callgrind, {workers} at a time", jobs.len());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(workload, hooks)) = jobs.get(index) else {
                    break;
                };
                let result = count_one(&binary, &out_dir, workload, hooks);
                match results.lock() {
                    Ok(mut results) => results.push(result),
                    Err(poisoned) => poisoned.into_inner().push(result),
                }
            });
        }
    });
    let results = match results.into_inner() {
        Ok(results) => results,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut cells = Vec::with_capacity(jobs.len());
    for result in results {
        cells.push(result?);
    }
    cells.sort_by_key(|c| {
        (
            LOOPS.iter().position(|w| *w == c.workload),
            HOOKS.iter().position(|h| *h == c.hooks),
        )
    });
    Ok(Counts { toolchain, cells })
}

fn read_baseline(root: &Path) -> Result<Counts, String> {
    let path = root.join(BASELINE);
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Counts::parse(&text).map_err(|err| format!("{}: {err}", path.display()))
}

/// The change from `base` to `now`, in percent.
fn change_percent(base: u64, now: u64) -> f64 {
    (now as f64 - base as f64) / base as f64 * 100.0
}

/// What a comparison found.
#[derive(Debug, Default, PartialEq)]
struct Verdict {
    /// The cells that rose by more than the limit.
    risen: Vec<String>,
    /// The cells that fell by more than the limit.
    fallen: Vec<String>,
}

fn compare(baseline: &Counts, measured: &Counts) -> Result<Verdict, String> {
    let mut verdict = Verdict::default();
    println!(
        "{:<8} {:<7} {:>16} {:>16} {:>9}",
        "loop", "hooks", "baseline", "now", "change"
    );
    for cell in &measured.cells {
        let base = baseline
            .get(cell.workload, cell.hooks)
            .ok_or_else(|| format!("the baseline has no {} {}", cell.workload, cell.hooks))?;
        let change = change_percent(base, cell.instructions);
        println!(
            "{:<8} {:<7} {:>16} {:>16} {:>+8.3}%",
            cell.workload, cell.hooks, base, cell.instructions, change
        );
        let name = format!("{} {} {change:+.3}%", cell.workload, cell.hooks);
        if change > RISE_LIMIT_PERCENT {
            verdict.risen.push(name);
        } else if change < -RISE_LIMIT_PERCENT {
            verdict.fallen.push(name);
        }
    }
    Ok(verdict)
}

/// The ci step: measure and compare, or say why it is skipped. The
/// perfbench is built with `cargo`, a command whose environment is the
/// gate's, so the build reuses what the gate's other steps built.
pub fn check(root: &Path, cargo: Command) -> Result<(), String> {
    let baseline = read_baseline(root)?;
    if let Err(why) = valgrind_runs() {
        println!("skipped: {why}");
        return Ok(());
    }
    let toolchain = toolchain(root)?;
    if toolchain != baseline.toolchain {
        println!(
            "skipped: the baseline was counted with {}, and this host builds with {}; \
             counts from another compiler or host are not comparable. \
             `cargo xtask hot-loop-baseline` counts this one (docs/aarch64-automation.md)",
            baseline.toolchain, toolchain
        );
        return Ok(());
    }
    let measured = measure(root, toolchain, cargo)?;
    let verdict = compare(&baseline, &measured)?;
    if !verdict.fallen.is_empty() {
        println!(
            "fell by more than {RISE_LIMIT_PERCENT}%: {}; `cargo xtask hot-loop-baseline` \
             keeps the gain",
            verdict.fallen.join(", ")
        );
    }
    if verdict.risen.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "host instructions inside run_until rose by more than {RISE_LIMIT_PERCENT}%: {}",
            verdict.risen.join(", ")
        ))
    }
}

/// `cargo xtask hot-loop-baseline`: count the cells on this host and write
/// them as the baseline, building in the environment it was run in.
pub fn refresh(root: &Path) -> Result<(), String> {
    valgrind_runs().map_err(|why| format!("cannot count: {why}"))?;
    let toolchain = toolchain(root)?;
    let measured = measure(root, toolchain, Command::new("cargo"))?;
    if let Ok(baseline) = read_baseline(root) {
        if baseline.toolchain == measured.toolchain {
            compare(&baseline, &measured)?;
        }
    }
    let path = root.join(BASELINE);
    fs::write(&path, measured.render())
        .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    println!("baseline written: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(alu_plain: u64) -> Counts {
        let mut cells = Vec::new();
        for workload in LOOPS {
            for hooks in HOOKS {
                cells.push(Count {
                    workload,
                    hooks,
                    instructions: if (workload, hooks) == ("alu", "plain") {
                        alu_plain
                    } else {
                        1_000_000
                    },
                });
            }
        }
        Counts {
            toolchain: Toolchain {
                rustc: "abc".to_string(),
                host: "x86_64-unknown-linux-gnu".to_string(),
                fma: true,
            },
            cells,
        }
    }

    #[test]
    fn a_baseline_reads_back_as_written() {
        let written = counts(8_470_198_632);
        assert_eq!(Counts::parse(&written.render()), Ok(written));
    }

    #[test]
    fn a_baseline_missing_a_cell_is_refused() {
        let text = counts(1_000_000).render().replace("fp hooked 1000000\n", "");
        assert!(Counts::parse(&text).is_err());
    }

    #[test]
    fn only_a_rise_past_the_limit_fails() {
        let base = counts(1_000_000);
        let at_limit = compare(&base, &counts(1_005_000)).unwrap();
        assert!(at_limit.risen.is_empty() && at_limit.fallen.is_empty());
        let past = compare(&base, &counts(1_005_001)).unwrap();
        assert_eq!(past.risen.len(), 1);
        let fallen = compare(&base, &counts(994_999)).unwrap();
        assert!(fallen.risen.is_empty());
        assert_eq!(fallen.fallen.len(), 1);
    }

    #[test]
    fn the_summary_line_is_the_total() {
        let dir = env::temp_dir().join(format!("hot_loop_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cg.out");
        fs::write(&file, "version: 1\nevents: Ir\nfn=x\n0x10 5\nsummary: 8470198632\ntotals: 8470198632\n").unwrap();
        assert_eq!(callgrind_total(&file), Ok(8_470_198_632));
        fs::remove_dir_all(&dir).unwrap();
    }
}
