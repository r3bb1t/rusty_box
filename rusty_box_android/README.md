# Rusty Box Android

Pure-Rust Android NativeActivity frontend for Rusty Box.

This crate builds an ARM64 APK that embeds a local Alpine ISO at compile time, boots it through the Rust emulator core, renders the shared egui VGA screen, and exposes phone-friendly PS/2 key controls plus the Linux serial console log.

## Quick start

From the repository root:

```bash
cargo xtask android build
cargo xtask android run
```

To capture the connected phone screen:

```bash
cargo xtask android screenshot rustybox_android.png
```

The xtask installs missing Android SDK components, the Rust Android target, and `cargo-apk`; copies `~/Downloads/alpine-virt-3.23.3-x86_64.iso` to `rusty_box_android/assets/alpine.iso`; and signs with a generated local dev keystore under your home directory.

## Local assets and secrets

Do not commit local device/build assets:

- `rusty_box_android/assets/alpine.iso` is ignored and must stay local.
- Android dev signing material is generated under your home directory, not inside the repo. It uses the standard non-secret Android password `android` and is not production signing material.
- If you provide your own production signing material, set `CARGO_APK_RELEASE_KEYSTORE` and `CARGO_APK_RELEASE_KEYSTORE_PASSWORD` in your environment instead of editing files.

## Manual cargo-apk build

Use this only if your Android SDK, `cargo-apk`, keystore env, and Alpine asset are already prepared:

```bash
cargo apk build -p rusty_box_android --lib --release --features embedded-alpine
```

The default feature set intentionally excludes `embedded-alpine`, so normal workspace checks do not require the large ISO file.

## Runtime controls

The APK launches in landscape and shows:

- status row with instruction count, IPS, and run state;
- PS/2 text field and shortcut buttons for BIOS/VGA keyboard input;
- VGA framebuffer from the shared core GUI path;
- `Linux Serial Log (ttyS0)` serial output panel from the guest.
