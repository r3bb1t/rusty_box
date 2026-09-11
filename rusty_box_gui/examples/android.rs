//! The APK's native library. NativeActivity loads it and calls
//! `android_main`, which hands the activity to the shell
//! ([`rusty_box_gui::android::main`]). `cargo xtask android build` packages it;
//! on every other target the library is empty.

// SAFETY: the one `android_main` in this library. android-activity's
// NativeActivity glue calls it by this unmangled name, with the signature
// below, once per process.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: rusty_box_gui::android::AndroidApp) {
    rusty_box_gui::android::main(app);
}
