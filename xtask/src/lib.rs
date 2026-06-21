use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use adb_client::{server::ADBServer, server_device::ADBServerDevice, ADBDeviceExt};
use std::net::{Ipv4Addr, SocketAddrV4};

const CMDLINE_TOOLS_VERSION: &str = "14742923";
const ANDROID_PLATFORM: &str = "android-35";
const BUILD_TOOLS_VERSION: &str = "35.0.0";
const NDK_VERSION: &str = "29.0.14206865";
const RUST_TARGET: &str = "aarch64-linux-android";
const ANDROID_PACKAGE: &str = "com.rustybox.android";
const LOCAL_SIGNING_PASSWORD: &str = "android";
const ANDROID_COMPONENT: &str = "com.rustybox.android/android.app.NativeActivity";
const APK_NAME: &str = "RustyBoxAndroid.apk";
const ALPINE_ISO_NAME: &str = "alpine-virt-3.23.3-x86_64.iso";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    Macos,
    Linux,
}

impl HostOs {
    fn current() -> Result<Self, String> {
        match env::consts::OS {
            "windows" => Ok(Self::Windows),
            "macos" => Ok(Self::Macos),
            "linux" => Ok(Self::Linux),
            other => Err(format!(
                "unsupported host OS {other:?}; Android xtask supports Windows, macOS, and Linux"
            )),
        }
    }

    fn exe_suffix(self) -> &'static str {
        match self {
            Self::Windows => ".exe",
            Self::Macos | Self::Linux => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidAction {
    Build,
    Run,
    Screenshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidCommand {
    pub action: AndroidAction,
    pub sdk: Option<PathBuf>,
    pub iso: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub skip_sdk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XtaskCommand {
    Android(AndroidCommand),
}

struct AndroidContext {
    host: HostOs,
    repo: PathBuf,
    home: PathBuf,
    sdk: PathBuf,
    ndk: PathBuf,
    path: std::ffi::OsString,
}

impl AndroidContext {
    fn new(command: &AndroidCommand) -> Result<Self, String> {
        let host = HostOs::current()?;
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or_else(|| "xtask manifest directory has no parent".to_string())?
            .to_path_buf();
        let home = home_dir()?;
        let sdk = command
            .sdk
            .clone()
            .or_else(|| env::var_os("ANDROID_HOME").map(PathBuf::from))
            .or_else(|| env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| home.join("Android").join("Sdk"));
        let ndk = sdk.join("ndk").join(NDK_VERSION);
        let path = android_path(&sdk)?;
        Ok(Self {
            host,
            repo,
            home,
            sdk,
            ndk,
            path,
        })
    }

    fn sdkmanager(&self) -> PathBuf {
        sdkmanager_path(&self.sdk, self.host)
    }

    fn adb_path(&self) -> Option<String> {
        let path = self
            .sdk
            .join("platform-tools")
            .join(format!("adb{}", self.host.exe_suffix()));
        path.exists().then(|| path_for_env(&path))
    }

    fn apk_path(&self) -> PathBuf {
        self.repo
            .join("target")
            .join("release")
            .join("apk")
            .join(APK_NAME)
    }

    fn release_keystore(&self) -> PathBuf {
        self.home
            .join(".android")
            .join("rusty_box_android_xtask_debug.keystore")
    }
}

pub fn main_entry() -> Result<(), String> {
    let command = parse_args(env::args().skip(1))?;
    execute(command)
}

pub fn parse_args<I, S>(args: I) -> Result<XtaskCommand, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        return Err(usage());
    }
    if args[0] != "android" {
        return Err(format!("unknown xtask command {:?}\n{}", args[0], usage()));
    }
    parse_android_args(&args[1..]).map(XtaskCommand::Android)
}

pub fn command_line_tools_url(os: HostOs) -> &'static str {
    match os {
        HostOs::Windows => {
            "https://dl.google.com/android/repository/commandlinetools-win-14742923_latest.zip"
        }
        HostOs::Macos => {
            "https://dl.google.com/android/repository/commandlinetools-mac-14742923_latest.zip"
        }
        HostOs::Linux => {
            "https://dl.google.com/android/repository/commandlinetools-linux-14742923_latest.zip"
        }
    }
}

pub fn sdkmanager_path(sdk: &Path, os: HostOs) -> PathBuf {
    let binary = match os {
        HostOs::Windows => "sdkmanager.bat",
        HostOs::Macos | HostOs::Linux => "sdkmanager",
    };
    sdk.join("cmdline-tools")
        .join("latest")
        .join("bin")
        .join(binary)
}

pub fn cargo_apk_signing_env(keystore: &Path, password: &str) -> Vec<(String, String)> {
    vec![
        (
            "CARGO_APK_RELEASE_KEYSTORE".to_string(),
            path_for_env(keystore),
        ),
        (
            "CARGO_APK_RELEASE_KEYSTORE_PASSWORD".to_string(),
            password.to_string(),
        ),
    ]
}

pub fn cargo_apk_build_args() -> Vec<&'static str> {
    vec![
        "apk",
        "build",
        "-p",
        "rusty_box_android",
        "--lib",
        "--release",
        "--features",
        "embedded-alpine",
    ]
}

pub fn adb_start_args() -> Vec<&'static str> {
    vec!["shell", "am", "start", "-n", ANDROID_COMPONENT]
}

fn execute(command: XtaskCommand) -> Result<(), String> {
    match command {
        XtaskCommand::Android(command) => execute_android(command),
    }
}

fn parse_android_args(args: &[String]) -> Result<AndroidCommand, String> {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        return Err(android_usage());
    }

    let action = match args[0].as_str() {
        "build" => AndroidAction::Build,
        "run" => AndroidAction::Run,
        "screenshot" => AndroidAction::Screenshot,
        other => {
            return Err(format!(
                "unknown android action {other:?}\n{}",
                android_usage()
            ))
        }
    };

    let mut sdk = None;
    let mut iso = None;
    let mut screenshot = None;
    let mut skip_sdk = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--sdk" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--sdk requires a path".to_string())?;
                sdk = Some(PathBuf::from(value));
            }
            "--iso" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--iso requires a path".to_string())?;
                iso = Some(PathBuf::from(value));
            }
            "--screenshot" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--screenshot requires a path".to_string())?;
                screenshot = Some(PathBuf::from(value));
            }
            "--skip-sdk" => skip_sdk = true,
            positional if action == AndroidAction::Screenshot && screenshot.is_none() => {
                screenshot = Some(PathBuf::from(positional));
            }
            other => {
                return Err(format!(
                    "unexpected android argument {other:?}\n{}",
                    android_usage()
                ))
            }
        }
        index += 1;
    }

    if action == AndroidAction::Screenshot && screenshot.is_none() {
        screenshot = Some(PathBuf::from("rustybox_android.png"));
    }

    Ok(AndroidCommand {
        action,
        sdk,
        iso,
        screenshot,
        skip_sdk,
    })
}

fn execute_android(command: AndroidCommand) -> Result<(), String> {
    let context = AndroidContext::new(&command)?;

    match command.action {
        AndroidAction::Build => {
            prepare_android_build(&context, &command)?;
            build_apk(&context)?;
        }
        AndroidAction::Run => {
            prepare_android_build(&context, &command)?;
            build_apk(&context)?;
            install_and_launch(&context)?;
            if let Some(path) = command.screenshot.as_deref() {
                thread::sleep(Duration::from_secs(10));
                capture_screenshot(&context, path)?;
            }
        }
        AndroidAction::Screenshot => {
            if !command.skip_sdk {
                ensure_android_sdk(&context)?;
            }
            let path = command
                .screenshot
                .as_deref()
                .expect("defaulted screenshot path");
            capture_screenshot(&context, path)?;
        }
    }

    Ok(())
}

fn prepare_android_build(context: &AndroidContext, command: &AndroidCommand) -> Result<(), String> {
    if !command.skip_sdk {
        ensure_android_sdk(context)?;
    }
    copy_alpine_iso(context, command.iso.as_deref())?;
    ensure_rust_tools(context)?;
    ensure_release_keystore(context)?;
    Ok(())
}

fn ensure_android_sdk(context: &AndroidContext) -> Result<(), String> {
    ensure_cmdline_tools(context)?;
    accept_licenses(context)?;
    install_sdk_package(context, "platform-tools")?;
    install_sdk_package(context, &format!("platforms;{ANDROID_PLATFORM}"))?;
    install_sdk_package(context, &format!("build-tools;{BUILD_TOOLS_VERSION}"))?;
    install_sdk_package(context, &format!("ndk;{NDK_VERSION}"))?;
    Ok(())
}

fn ensure_cmdline_tools(context: &AndroidContext) -> Result<(), String> {
    let sdkmanager = context.sdkmanager();
    if sdkmanager.exists() {
        step(&format!(
            "Android command-line tools found: {}",
            sdkmanager.display()
        ));
        return Ok(());
    }

    let zip_path = context.home.join("Downloads").join(format!(
        "commandlinetools-{}-{CMDLINE_TOOLS_VERSION}_latest.zip",
        match context.host {
            HostOs::Windows => "win",
            HostOs::Macos => "mac",
            HostOs::Linux => "linux",
        }
    ));
    let url = command_line_tools_url(context.host);

    step("Downloading Android command-line tools");
    if let Some(parent) = zip_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    if !zip_path.exists() {
        download_file(url, &zip_path)?;
    }

    step("Extracting Android command-line tools");
    let cmdline_dir = context.sdk.join("cmdline-tools");
    let tmp = cmdline_dir.join("_extract");
    let latest = cmdline_dir.join("latest");
    remove_dir_if_exists(&tmp)?;
    remove_dir_if_exists(&latest)?;
    fs::create_dir_all(&tmp).map_err(|error| format!("create {}: {error}", tmp.display()))?;
    extract_zip(&zip_path, &tmp)?;
    fs::create_dir_all(&cmdline_dir)
        .map_err(|error| format!("create {}: {error}", cmdline_dir.display()))?;
    fs::rename(tmp.join("cmdline-tools"), &latest)
        .map_err(|error| format!("move cmdline-tools to {}: {error}", latest.display()))?;
    make_executable(&sdkmanager)?;
    remove_dir_if_exists(&tmp)?;
    Ok(())
}

fn accept_licenses(context: &AndroidContext) -> Result<(), String> {
    step("Accepting Android SDK licenses");
    let sdk_root = format!("--sdk_root={}", context.sdk.display());
    let mut command = Command::new(context.sdkmanager());
    apply_android_env(&mut command, context, &[]);
    command
        .arg(sdk_root)
        .arg("--licenses")
        .stdin(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn sdkmanager --licenses: {error}"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "sdkmanager stdin was unavailable".to_string())?;
        for _ in 0..100 {
            stdin
                .write_all(b"y\n")
                .map_err(|error| format!("write license acceptance: {error}"))?;
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("wait for sdkmanager --licenses: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("sdkmanager --licenses failed with status {status}"))
    }
}

fn install_sdk_package(context: &AndroidContext, package: &str) -> Result<(), String> {
    if let Some(marker) = sdk_package_marker(context, package) {
        if marker.exists() {
            step(&format!("Android SDK package already installed: {package}"));
            return Ok(());
        }
    }

    step(&format!("Installing Android SDK package: {package}"));
    let sdk_root = format!("--sdk_root={}", context.sdk.display());
    run_program(
        context.sdkmanager().as_os_str(),
        &[sdk_root, package.to_string()],
        context,
        &[],
        &format!("install Android SDK package {package}"),
    )
}

fn sdk_package_marker(context: &AndroidContext, package: &str) -> Option<PathBuf> {
    let exe = context.host.exe_suffix();
    match package {
        "platform-tools" => Some(context.sdk.join("platform-tools").join(format!("adb{exe}"))),
        p if p == format!("platforms;{ANDROID_PLATFORM}") => Some(
            context
                .sdk
                .join("platforms")
                .join(ANDROID_PLATFORM)
                .join("android.jar"),
        ),
        p if p == format!("build-tools;{BUILD_TOOLS_VERSION}") => Some(
            context
                .sdk
                .join("build-tools")
                .join(BUILD_TOOLS_VERSION)
                .join(format!("aapt2{exe}")),
        ),
        p if p == format!("ndk;{NDK_VERSION}") => Some(
            context
                .sdk
                .join("ndk")
                .join(NDK_VERSION)
                .join("source.properties"),
        ),
        _ => None,
    }
}

fn copy_alpine_iso(context: &AndroidContext, explicit_iso: Option<&Path>) -> Result<(), String> {
    step("Copying Alpine ISO asset");
    let destination = context
        .repo
        .join("rusty_box_android")
        .join("assets")
        .join("alpine.iso");
    let source = match explicit_iso {
        Some(source) if source.exists() => Some(source.to_path_buf()),
        Some(source) => {
            return Err(format!(
                "explicit Alpine ISO path does not exist: {}",
                source.display()
            ));
        }
        None => {
            let downloads = context.home.join("Downloads").join(ALPINE_ISO_NAME);
            if downloads.exists() {
                Some(downloads)
            } else {
                let fallback = context
                    .repo
                    .join("examples")
                    .join("rusty_box_uefi")
                    .join("alpine.iso");
                fallback.exists().then_some(fallback)
            }
        }
    };

    match source {
        Some(source) => {
            if source != destination {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("create {}: {error}", parent.display()))?;
                }
                fs::copy(&source, &destination).map_err(|error| {
                    format!(
                        "copy Alpine ISO from {} to {}: {error}",
                        source.display(),
                        destination.display()
                    )
                })?;
            }
            println!("Copied {}", destination.display());
            Ok(())
        }
        None if destination.exists() => {
            println!("Using existing {}", destination.display());
            Ok(())
        }
        None => Err(format!(
            "missing Alpine ISO; pass --iso PATH or place {ALPINE_ISO_NAME} in {}",
            context.home.join("Downloads").display()
        )),
    }
}

fn ensure_rust_tools(context: &AndroidContext) -> Result<(), String> {
    step("Installing Rust Android target and cargo-apk");
    run_program(
        OsStr::new("rustup"),
        &[
            "target".to_string(),
            "add".to_string(),
            RUST_TARGET.to_string(),
        ],
        context,
        &[],
        "install Rust Android target",
    )?;

    if command_success(
        OsStr::new("cargo"),
        &["apk".to_string(), "--version".to_string()],
        context,
        &[],
    ) {
        println!("cargo-apk already installed");
        return Ok(());
    }

    run_program(
        OsStr::new("cargo"),
        &["install".to_string(), "cargo-apk".to_string()],
        context,
        &[],
        "install cargo-apk",
    )
}

fn ensure_release_keystore(context: &AndroidContext) -> Result<Vec<(String, String)>, String> {
    if let (Ok(keystore), Ok(password)) = (
        env::var("CARGO_APK_RELEASE_KEYSTORE"),
        env::var("CARGO_APK_RELEASE_KEYSTORE_PASSWORD"),
    ) {
        if !password.is_empty() {
            step("Using CARGO_APK_RELEASE_KEYSTORE from environment");
            return Ok(cargo_apk_signing_env(Path::new(&keystore), &password));
        }
    }

    let keystore = context.release_keystore();
    let password = LOCAL_SIGNING_PASSWORD.to_string();
    if keystore.exists() {
        step(&format!(
            "Android local dev keystore found: {}",
            keystore.display()
        ));
        return Ok(cargo_apk_signing_env(&keystore, &password));
    }

    step("Generating local Android dev keystore");
    if let Some(parent) = keystore.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let keytool = keytool_program(context.host);
    run_program(
        keytool.as_os_str(),
        &[
            "-genkeypair".to_string(),
            "-v".to_string(),
            "-keystore".to_string(),
            path_for_env(&keystore),
            "-storepass".to_string(),
            password.clone(),
            "-keypass".to_string(),
            password.clone(),
            "-alias".to_string(),
            "rustyboxandroid".to_string(),
            "-keyalg".to_string(),
            "RSA".to_string(),
            "-keysize".to_string(),
            "2048".to_string(),
            "-validity".to_string(),
            "10000".to_string(),
            "-dname".to_string(),
            "CN=Rusty Box Android, O=Rusty Box, C=US".to_string(),
        ],
        context,
        &[],
        "generate Android dev keystore",
    )?;
    Ok(cargo_apk_signing_env(&keystore, &password))
}

fn build_apk(context: &AndroidContext) -> Result<(), String> {
    let signing_env = ensure_release_keystore(context)?;
    step("Building Rusty Box Android APK");
    run_program(
        OsStr::new("cargo"),
        &cargo_apk_build_args()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        context,
        &signing_env,
        "build Rusty Box Android APK",
    )
}

fn install_and_launch(context: &AndroidContext) -> Result<(), String> {
    let mut device = connected_adb_device(context)?;

    step("Installing Rusty Box Android APK");
    if let Err(error) = device.install(&context.apk_path(), None) {
        println!("Install failed; uninstalling existing package and retrying: {error}");
        device
            .uninstall(ANDROID_PACKAGE, None)
            .map_err(|uninstall_error| {
                format!(
                    "install failed ({error}); uninstall {ANDROID_PACKAGE} also failed: {uninstall_error}"
                )
            })?;
        device
            .install(&context.apk_path(), None)
            .map_err(|retry_error| {
                format!("install Rusty Box Android APK after uninstall: {retry_error}")
            })?;
    }

    step("Launching Rusty Box Android");
    run_device_shell(
        &mut device,
        &format!("am force-stop {ANDROID_PACKAGE}"),
        "force-stop Rusty Box Android",
    )?;
    run_device_shell(
        &mut device,
        &format!("am start -n {ANDROID_COMPONENT}"),
        "start Rusty Box Android",
    )
}

fn capture_screenshot(context: &AndroidContext, destination: &Path) -> Result<(), String> {
    let mut device = connected_adb_device(context)?;

    step("Capturing Android screenshot");
    let remote = "/sdcard/rustybox_android_xtask.png";
    run_device_shell(
        &mut device,
        &format!("screencap -p {remote}"),
        "capture Android screenshot",
    )?;
    let mut output = File::create(destination)
        .map_err(|error| format!("create screenshot {}: {error}", destination.display()))?;
    device.pull(&remote, &mut output).map_err(|error| {
        format!(
            "pull Android screenshot to {}: {error}",
            destination.display()
        )
    })
}

fn connected_adb_device(context: &AndroidContext) -> Result<ADBServerDevice, String> {
    step("Checking connected Android devices");
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5037);
    let mut server = ADBServer::new_from_path(address, context.adb_path());
    let devices = server
        .devices()
        .map_err(|error| format!("list Android devices through adb_client: {error}"))?;
    if devices.is_empty() {
        return Err(
            "no Android devices attached; enable USB debugging and accept the RSA prompt"
                .to_string(),
        );
    }
    println!("List of devices attached");
    for device in &devices {
        println!("{device}");
    }
    server
        .get_device()
        .map_err(|error| format!("select Android device through adb_client: {error}"))
}

fn run_device_shell(
    device: &mut ADBServerDevice,
    command: &str,
    label: &str,
) -> Result<(), String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = device
        .shell_command(&command, Some(&mut stdout), Some(&mut stderr))
        .map_err(|error| format!("{label}: {error}"))?;
    if !stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&stdout));
    }
    if !stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&stderr));
    }
    match exit {
        Some(0) | None => Ok(()),
        Some(code) => Err(format!("{label}: Android shell exited with status {code}")),
    }
}

fn run_program(
    program: &OsStr,
    args: &[String],
    context: &AndroidContext,
    extra_env: &[(String, String)],
    label: &str,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(&context.repo);
    apply_android_env(&mut command, context, extra_env);
    let status = command
        .status()
        .map_err(|error| format!("{label}: failed to spawn {:?}: {error}", program))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label}: command exited with status {status}"))
    }
}

fn command_success(
    program: &OsStr,
    args: &[String],
    context: &AndroidContext,
    extra_env: &[(String, String)],
) -> bool {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(&context.repo)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_android_env(&mut command, context, extra_env);
    command
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn apply_android_env(
    command: &mut Command,
    context: &AndroidContext,
    extra_env: &[(String, String)],
) {
    command
        .env("ANDROID_HOME", &context.sdk)
        .env("ANDROID_NDK_ROOT", &context.ndk)
        .env("PATH", &context.path);
    for (key, value) in extra_env {
        command.env(key, value);
    }
}

fn android_path(sdk: &Path) -> Result<std::ffi::OsString, String> {
    let mut entries = vec![
        sdk.join("platform-tools"),
        sdk.join("cmdline-tools").join("latest").join("bin"),
        sdk.join("build-tools").join(BUILD_TOOLS_VERSION),
    ];
    if let Some(existing) = env::var_os("PATH") {
        entries.extend(env::split_paths(&existing));
    }
    env::join_paths(entries).map_err(|error| format!("build Android PATH: {error}"))
}

fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME or USERPROFILE must be set".to_string())
}

fn path_for_env(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn step(message: &str) {
    println!("\n==> {message}");
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .call()
        .map_err(|error| format!("download {url}: {error}"))?;
    let mut reader = response.into_reader();
    let mut file = File::create(destination)
        .map_err(|error| format!("create {}: {error}", destination.display()))?;
    io::copy(&mut reader, &mut file)
        .map_err(|error| format!("write {}: {error}", destination.display()))?;
    Ok(())
}

fn extract_zip(zip_path: &Path, destination: &Path) -> Result<(), String> {
    let file =
        File::open(zip_path).map_err(|error| format!("open {}: {error}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("read zip {}: {error}", zip_path.display()))?;
    archive.extract(destination).map_err(|error| {
        format!(
            "extract {} to {}: {error}",
            zip_path.display(),
            destination.display()
        )
    })
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove {}: {error}", path.display())),
    }
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata =
            fs::metadata(path).map_err(|error| format!("metadata {}: {error}", path.display()))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("chmod {}: {error}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn keytool_program(host: HostOs) -> PathBuf {
    if let Some(java_home) = env::var_os("JAVA_HOME") {
        let candidate = PathBuf::from(java_home)
            .join("bin")
            .join(format!("keytool{}", host.exe_suffix()));
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(format!("keytool{}", host.exe_suffix()))
}

fn usage() -> String {
    format!(
        "Usage:\n  cargo xtask android build [--sdk PATH] [--iso PATH] [--skip-sdk]\n  cargo xtask android run [--sdk PATH] [--iso PATH] [--skip-sdk] [--screenshot PATH]\n  cargo xtask android screenshot [PATH] [--sdk PATH] [--skip-sdk]"
    )
}

fn android_usage() -> String {
    usage()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn command_line_tools_urls_are_platform_specific() {
        assert_eq!(
            command_line_tools_url(HostOs::Windows),
            "https://dl.google.com/android/repository/commandlinetools-win-14742923_latest.zip"
        );
        assert_eq!(
            command_line_tools_url(HostOs::Macos),
            "https://dl.google.com/android/repository/commandlinetools-mac-14742923_latest.zip"
        );
        assert_eq!(
            command_line_tools_url(HostOs::Linux),
            "https://dl.google.com/android/repository/commandlinetools-linux-14742923_latest.zip"
        );
    }

    #[test]
    fn sdkmanager_path_uses_host_script_name() {
        let sdk = PathBuf::from("/tmp/android-sdk");
        assert_eq!(
            sdkmanager_path(&sdk, HostOs::Windows),
            sdk.join("cmdline-tools")
                .join("latest")
                .join("bin")
                .join("sdkmanager.bat")
        );
        assert_eq!(
            sdkmanager_path(&sdk, HostOs::Linux),
            sdk.join("cmdline-tools")
                .join("latest")
                .join("bin")
                .join("sdkmanager")
        );
    }

    #[test]
    fn parse_android_run_screenshot_command() {
        let command = parse_args(["android", "run", "--screenshot", "screen.png"])
            .expect("parse android run");
        assert_eq!(
            command,
            XtaskCommand::Android(AndroidCommand {
                action: AndroidAction::Run,
                sdk: None,
                iso: None,
                screenshot: Some(PathBuf::from("screen.png")),
                skip_sdk: false,
            })
        );
    }

    #[test]
    fn android_build_uses_commit_safe_signing_env_names() {
        let env = cargo_apk_signing_env(&PathBuf::from("keystore.jks"), "test-value");
        assert_eq!(
            env,
            vec![
                (
                    "CARGO_APK_RELEASE_KEYSTORE".to_string(),
                    "keystore.jks".to_string(),
                ),
                (
                    "CARGO_APK_RELEASE_KEYSTORE_PASSWORD".to_string(),
                    "test-value".to_string(),
                ),
            ]
        );
    }

    #[test]
    fn apk_build_args_are_release_embedded_alpine() {
        assert_eq!(
            cargo_apk_build_args(),
            vec![
                "apk",
                "build",
                "-p",
                "rusty_box_android",
                "--lib",
                "--release",
                "--features",
                "embedded-alpine",
            ]
        );
        assert_eq!(
            adb_start_args(),
            vec![
                "shell",
                "am",
                "start",
                "-n",
                "com.rustybox.android/android.app.NativeActivity",
            ]
        );
    }
    #[test]
    fn copy_alpine_iso_rejects_missing_explicit_path_without_fallback() {
        let root = std::env::temp_dir().join(format!(
            "rusty_box_xtask_iso_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before Unix epoch")
                .as_nanos()
        ));
        let repo = root.join("repo");
        let home = root.join("home");
        let fallback = repo
            .join("examples")
            .join("rusty_box_uefi")
            .join("alpine.iso");
        let destination = repo
            .join("rusty_box_android")
            .join("assets")
            .join("alpine.iso");
        fs::create_dir_all(fallback.parent().expect("fallback parent"))
            .expect("create fallback dir");
        fs::write(&fallback, b"fallback").expect("write fallback ISO");
        fs::create_dir_all(destination.parent().expect("destination parent"))
            .expect("create destination dir");
        fs::write(&destination, b"existing").expect("write existing destination ISO");

        let explicit = root.join("missing").join("explicit.iso");
        let context = AndroidContext {
            host: HostOs::Linux,
            repo,
            home,
            sdk: root.join("sdk"),
            ndk: root.join("ndk"),
            path: std::ffi::OsString::new(),
        };

        let error = copy_alpine_iso(&context, Some(&explicit))
            .expect_err("missing explicit ISO should not use fallback or destination");

        assert!(
            error.contains(&explicit.display().to_string()),
            "error {error:?} did not mention explicit path {}",
            explicit.display()
        );
        assert_eq!(
            fs::read(&destination).expect("read destination ISO"),
            b"existing".to_vec()
        );
        fs::remove_dir_all(&root).expect("clean up test directory");
    }
}
