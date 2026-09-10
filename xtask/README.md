# xtask

Repository automation for Rusty Box. Run it through the Cargo alias in `.cargo/config.toml` (`xtask = "run --package xtask --"`):

```bash
cargo xtask ci                   # the local gate, run before every commit
cargo xtask ci --skip-boot       # the same, without the DLX boot gate
cargo xtask ci --full            # also the full rusty_box test suite and a GUI release build
cargo xtask perf-baseline        # archive a release perfbench binary for A/B runs
cargo xtask android build        # Android APK, see "Android commands"
cargo xtask --help               # usage for every subcommand
```

`cargo xtask` with no arguments, `--help` or `-h` prints the usage for every subcommand and exits with status 1. `cargo xtask ci --help` prints the `ci` options.

## `cargo xtask ci [--full] [--skip-boot]`

This is the local gate suite, defined in `xtask/src/ci.rs`. It runs its steps in the order below and stops at the first failure, naming the failed step and its exit status. A clean run ends with `ci: N steps passed in <seconds>s`.

| Invocation | Steps |
| --- | --- |
| `cargo xtask ci` | 27 |
| `cargo xtask ci --skip-boot` | 26 |
| `cargo xtask ci --full` | 29 |
| `cargo xtask ci --full --skip-boot` | 28 |

### Prerequisites

- **Rust targets:** `rustup target add x86_64-unknown-none x86_64-unknown-uefi wasm32-unknown-unknown`.
- **The Bochs BIOS ROMs.** The repository does not include them; `/cpp_orig` is gitignored. Two steps embed them at compile time with `include_bytes!`: the UEFI example build and the GUI wasm check. They need them at exactly these paths:
  - `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest`
  - `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`

  The boot gate also finds them there. The step "Download Bochs BIOS ROMs for wasm GUI" in `.github/workflows/ci.yml` shows one way to fetch them.
- **The DLX Linux disk image `dlxlinux/hd10meg.img`**, also gitignored. The UEFI example build embeds it, so you need it even with `--skip-boot`. The boot gate boots it.

### Step 1: doctrine ratchets

This step is an in-process text scan, not a cargo command. It enforces the Safety Doctrine counts (`docs/safety-doctrine.md`) over seven source trees: `rusty_box`, `rusty_box_decoder`, `rusty_box_core`, `rusty_box_devices`, `rusty_box_whp_sys`, `rusty_box_whp` and `rusty_box_whp_engine`. It checks four counts:

- **`unsafe` tokens per crate (R1)**, with comment lines stripped, against `UNSAFE_TOKEN_BASELINES`. A count above its baseline fails the step. A count below it passes, but prints a reminder to lower the baseline in the same commit.
- **`unsafe impl … Send` / `Sync` lines in `rusty_box/src` (R6).** Baseline zero.
- **Files with a blanket `dead_code` allow, per crate**, against `BLANKET_DEAD_CODE_BASELINES`. A file counts if it carries a crate-inner `#![allow(…)]` or `#![expect(…)]` that covers `dead_code`, whether it names the lint directly or through `unused` or `warnings`. A scanned crate with no entry in the table is held at zero. Above the baseline fails; below prints a reminder to tighten it. A targeted `#[allow(dead_code)]` on a single item is not counted.
- **`.unwrap()` / `.expect(` outside test code.** Baseline zero in every scanned crate. The scan skips `#[cfg(test)]` blocks, files named `tests.rs` or `*_tests.rs`, comment lines, and doc comments.

It is a line scan, not a parser. The baseline constants and the reason each one moved are in `xtask/src/ci.rs`.

### Steps 2 to 26: the build and test matrix

Every step is `--release` except the debug-assertions check, which exists to compile the `#[cfg(debug_assertions)]` code that release builds leave out.

| # | Step | Command |
| --- | --- | --- |
| 2 | core tests (no_std + no_alloc) | `cargo test --release -p rusty_box_core` |
| 3 | core tests (alloc) | `cargo test --release -p rusty_box_core --features alloc` |
| 4 | core tests (std) | `cargo test --release -p rusty_box_core --features std` |
| 5 | core bare-metal target check | `cargo check --release -p rusty_box_core --target x86_64-unknown-none` |
| 6 | devices tests (no_std + no_alloc) | `cargo test --release -p rusty_box_devices` |
| 7 | devices tests (std) | `cargo test --release -p rusty_box_devices --features std` |
| 8 | devices bare-metal target check | `cargo check --release -p rusty_box_devices --features alloc --target x86_64-unknown-none` |
| 9 | WHP sys tests | `cargo test --release -p rusty_box_whp_sys` |
| 10 | WHP leaf tests | `cargo test --release -p rusty_box_whp` |
| 11 | WHP probe builds | `cargo check --release -p rusty_box_whp --examples` |
| 12 | WHP engine tests | `cargo test --release -p rusty_box_whp_engine --all-targets` |
| 13 | decoder tests | `cargo test --release -p rusty_box_decoder` |
| 14 | lib tests (default features) | `cargo test --release -p rusty_box --lib` |
| 15 | no_std + no_alloc check | `cargo check --release -p rusty_box --no-default-features` |
| 16 | no_std + alloc check | `cargo check --release -p rusty_box --no-default-features --features alloc` |
| 17 | bare-metal target check | `cargo check --release -p rusty_box --no-default-features --target x86_64-unknown-none` |
| 18 | UEFI example build | `cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi` |
| 19 | wasm target check (lib, alloc) | `cargo check --release -p rusty_box --no-default-features --features alloc --target wasm32-unknown-unknown` |
| 20 | public API examples | `cargo test --release -p rusty_box --features std --test api_examples` |
| 21 | doc examples | `cargo test --release -p rusty_box --doc` |
| 22 | doctrine compile-fail fixtures | `cargo test --release -p rusty_box --features std --test compile_fail` |
| 23 | GUI wasm target check | `cargo check --release -p rusty_box_gui --target wasm32-unknown-unknown` |
| 24 | GUI host check, tests included | `cargo check --release -p rusty_box_gui --all-targets` |
| 25 | all-features check | `cargo check --release -p rusty_box --all-features` |
| 26 | debug-assertions check (all targets, all features) | `cargo check -p rusty_box --all-targets --all-features` |

Notes on specific steps:

- **WHP steps (9 to 12)** need no hypervisor. The sys and wrapper tests cover layouts and arithmetic. Engine tests that need hardware skip themselves on a host without it. `--all-targets` on step 12 also compiles the engine's example harnesses.
- **Compile-fail fixtures (step 22)** are the doctrine's trybuild tests. Their goldens are rustc's rendered diagnostics, so they change when the quoted code or the toolchain changes. Once you have confirmed the rule still holds, regenerate them with `TRYBUILD=overwrite cargo test --release -p rusty_box --features std --test compile_fail`.

### `--full`: two more steps

These run after the matrix and before the boot gate:

| Step | Command |
| --- | --- |
| full test suite (integration included) | `cargo test --release -p rusty_box` |
| GUI release build | `cargo build -p rusty_box_gui --release` |

### Last step: the DLX Linux headless boot gate

This step is skipped with `--skip-boot`. It runs:

```bash
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=450000000 \
  cargo run --release -p rusty_box --example dlxlinux --features std
```

It passes only if the run exits successfully and its stdout contains `*** LOGIN DETECTED ***`, the marker the example prints when the guest reaches the `dlx login:` prompt. On failure it prints the last 20 lines of output.

### What `ci` does not cover

- It compiles the `rusty_box_gui` tests but does not run them. Run them with `cargo test --release -p rusty_box_gui`.
- No step runs the tests of `rusty_box_bximage` or of `xtask`. Both are compiled: `rusty_box_bximage` as a dependency of the GUI steps, and `xtask` by the `cargo xtask` alias that runs the gate.
- No step builds `examples/rusty_box_web`, `examples/no_alloc_smoke` or `rusty_box_android`.

## `cargo xtask perf-baseline`

This command takes no options. It does three things:

1. Builds the `perfbench` example: `cargo build --release -p rusty_box --example perfbench --features std`. The source is `rusty_box/examples/perfbench/perfbench.rs`.
2. Copies the binary to `target/perf-baselines/<rev>/`, where `<rev>` is `git rev-parse --short=12 HEAD`. The directory is named after `HEAD`, so a build of an uncommitted tree is filed under the last commit's revision.
3. Prints the archived path.

Use it for interleaved A/B runs: alternate the baseline and candidate binaries for at least 10 pairs and compare the medians. `perfbench` runs a synthetic guest workload and needs no ROMs or disk images. The `PERFBENCH_MODE`, `PERFBENCH_INSN`, `PERFBENCH_CPUS` and `PERFBENCH_QUANTUM` environment variables select what it runs (see the environment-variable table in [`rusty_box/examples/README.md`](../rusty_box/examples/README.md)).

## Android commands

The `android` subcommand builds and deploys the `rusty_box_android` crate. That crate is slated for removal; when it goes, this subcommand and this section go with it.

```bash
cargo xtask android build
cargo xtask android run
cargo xtask android screenshot rustybox_android.png
```

### `cargo xtask android build`

This prepares the local Android toolchain and builds `target/release/apk/RustyBoxAndroid.apk`, in this order:

1. **Android SDK**, skipped with `--skip-sdk`. Finds the SDK under `--sdk`, `ANDROID_HOME`, `ANDROID_SDK_ROOT` or `~/Android/Sdk`, in that order. Downloads the command-line tools if they are missing. Accepts the SDK licenses. Installs `platform-tools`, `platforms;android-34`, `build-tools;35.0.0` and `ndk;29.0.14206865` when missing.
2. **Alpine ISO.** Copies the ISO into the gitignored asset path `rusty_box_android/assets/alpine.iso`. The source is `--iso PATH`, which fails if the path does not exist. Without `--iso`, it uses `~/Downloads/alpine-virt-3.23.3-x86_64.iso`, then `examples/rusty_box_uefi/alpine.iso`, and otherwise keeps an existing asset.
3. **Rust tools.** Runs `rustup target add aarch64-linux-android`. Installs `cargo-apk` if `cargo apk --version` fails.
4. **Signing keystore.** If `CARGO_APK_RELEASE_KEYSTORE` and a non-empty `CARGO_APK_RELEASE_KEYSTORE_PASSWORD` are set, they are used. Otherwise it uses `~/.android/rusty_box_android_xtask_debug.keystore`, generating it with `keytool` (from `JAVA_HOME/bin` when present) if it does not exist. That local dev keystore uses the standard non-secret Android password `android`; do not use it for production signing.
5. **Build.** Runs `cargo apk build -p rusty_box_android --lib --release --features embedded-alpine`.

### `cargo xtask android run`

Runs the build flow above. It then uses `adb_client`, over the local ADB server on port 5037, to install the APK and launch it. If the install fails, it uninstalls the old package and retries. This avoids the long-running `cargo apk run` logcat stream.

To take a screenshot 10 seconds after launch:

```bash
cargo xtask android run --screenshot rustybox_android.png
```

### `cargo xtask android screenshot [PATH]`

Captures the connected device's screen through `adb_client`. If `PATH` is omitted, it writes `rustybox_android.png`. It installs any missing SDK packages first, unless you pass `--skip-sdk`.

### Options

- `--sdk PATH` sets the Android SDK root for this run.
- `--iso PATH` copies a specific Alpine ISO into `rusty_box_android/assets/alpine.iso` before building.
- `--skip-sdk` skips SDK package installation and license acceptance. Use it when the SDK is already prepared.
- `--screenshot PATH` (`run` only) captures the screen after launch.

### Commit-safety rules

The xtask must not write secrets or large local assets into the repository:

- The Alpine ISO destination is gitignored.
- The generated local dev keystore lives under the user's home directory and uses a non-secret test password.
- Custom production signing uses environment variables, not committed config.
- Device operations use `adb_client`. The Android SDK's `adb` binary is used only to start the ADB server.
