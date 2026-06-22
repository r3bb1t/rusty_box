# xtask

Cross-platform repository automation for Rusty Box.

Use this crate through the Cargo alias in `.cargo/config.toml`:

```bash
cargo xtask android build
cargo xtask android run
cargo xtask android screenshot rustybox_android.png
```

## Android commands

### `cargo xtask android build`

Prepares the local Android toolchain and builds `target/release/apk/RustyBoxAndroid.apk`.

It performs these steps:

1. Ensures Android command-line tools are installed under `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or `~/Android/Sdk`.
2. Accepts SDK licenses and installs `platform-tools`, `platforms;android-34`, `build-tools;35.0.0`, and `ndk;29.0.14206865` when missing.
3. Runs `rustup target add aarch64-linux-android`.
4. Installs `cargo-apk` if `cargo apk --version` is unavailable.
5. Copies the Alpine ISO into the ignored Android asset path.
6. Generates a local dev signing keystore outside the repo when release signing env vars are not set. The local dev keystore uses the standard non-secret Android password `android`; do not use it for production signing.
7. Runs `cargo apk build -p rusty_box_android --lib --release --features embedded-alpine`.

### `cargo xtask android run`

Runs the build flow, then uses `adb_client` over the local ADB server to install and launch the APK. This avoids the long-running `cargo apk run` logcat stream.

Optional screenshot after launch:

```bash
cargo xtask android run --screenshot rustybox_android.png
```

### `cargo xtask android screenshot [PATH]`

Captures the current connected Android screen through `adb_client`. If `PATH` is omitted, it writes `rustybox_android.png`.

## Options

- `--sdk PATH` sets the Android SDK root for this run.
- `--iso PATH` copies a specific Alpine ISO into `rusty_box_android/assets/alpine.iso` before building.
- `--skip-sdk` skips SDK package installation and license acceptance; use it when the SDK is already prepared.

## Commit-safety rules

The xtask must not write secrets or large local assets into the repository.

- The Alpine ISO destination is gitignored.
- The generated local dev keystore lives under the user's home directory and uses a non-secret test password.
- Custom production signing should use environment variables, not committed config.
- Device operations use `adb_client`; the Android SDK's `adb` binary is only used by the ADB server startup path.
