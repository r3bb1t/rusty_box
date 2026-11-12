//! `cargo xtask nested-kvm`: a nested KVM guest on the Arm board, in this
//! terminal, in one command.
//!
//! Alpine Linux runs on the board's processor with EL2 (virtualization
//! on): on the Cortex-A57, the default, its KVM comes up in nVHE mode, and
//! on the Armv9.2 model (`--cpu armv9.2`), which has FEAT_VHE, in VHE mode,
//! each with the in-kernel vGIC; QEMU inside it runs a second Alpine under
//! KVM, on the host's processor as KVM presents it. The serial console is the
//! terminal: once the runner's script has started the nested guest, what is
//! typed goes to the guest. Ctrl-] quits the emulator, Ctrl-A x the nested
//! QEMU.
//!
//! Each step is skipped when its result is already in the work directory,
//! and `--rebuild` runs every one again:
//!   1. Alpine's aarch64 virt ISO, its SHA-512 checked;
//!   2. the kernel's arm64 `Image` and the initramfs, read out of the ISO;
//!   3. QEMU and its dependencies from Alpine's repositories, each index's
//!      signature and each package's checksum checked against the keys in
//!      the ISO's own initramfs;
//!   4. the disk: the ISO, zero-padded to a whole MiB, then an uncompressed
//!      tar of the packages under `qemu/`. The host Alpine boots from the
//!      ISO part (iso9660 reads nothing past its volume) and unpacks the tar
//!      from that MiB offset itself; the nested guest boots from the same
//!      disk;
//!   5. the runner, `linux_virt` (cargo, release), unless `--no-build`;
//!   6. the boot.
//!
//! Nothing beyond cargo is needed: the downloads, checksums, signatures,
//! ISO 9660, gzip and tar are this crate's own.

mod apk;
mod iso9660;

use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha512};

use apk::{ApkKeys, Index, Repository};
use iso9660::IsoImage;

const ALPINE_MIRROR: &str = "https://dl-cdn.alpinelinux.org/alpine/v3.23";
const ARCH: &str = "aarch64";
const ISO_NAME: &str = "alpine-virt-3.23.7-aarch64.iso";
/// The ISO's files the host boots with.
const KERNEL_IN_ISO: &str = "boot/vmlinuz-virt";
const INITRAMFS_IN_ISO: &str = "boot/initramfs-virt";
/// The package the host installs, with its dependencies.
const QEMU_PACKAGE: &str = "qemu-system-aarch64";
/// The serial console, and the modules Alpine's init loads to find and
/// mount its boot media: both guests take it.
const KERNEL_COMMAND_LINE: &str = "console=ttyAMA0 modules=loop,squashfs,sd-mod,usb-storage";
const MIB: u64 = 1 << 20;
const DOWNLOAD_ATTEMPTS: u32 = 3;

/// Emulated instructions a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstructionsPerSecond(pub u64);

/// A size in MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mebibytes(pub u64);

/// How far the boot's script goes before the terminal takes over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootTarget {
    /// The host Alpine's shell, QEMU installed, then the nested guest
    /// started under KVM.
    NestedGuest,
    /// The host Alpine's shell, QEMU installed.
    HostShell,
}

/// The board's processor model, by the runner's `--cpu` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCpu {
    /// The Cortex-A57: Armv8.0, whose KVM runs in nVHE mode.
    CortexA57,
    /// The Armv9.2 model with SME2: FEAT_VHE, whose KVM runs in VHE mode.
    Armv9p2,
}

impl HostCpu {
    const ALL: [Self; 2] = [Self::CortexA57, Self::Armv9p2];

    /// The name the runner's `--cpu` takes.
    fn name(self) -> &'static str {
        match self {
            Self::CortexA57 => "cortex-a57",
            Self::Armv9p2 => "armv9.2",
        }
    }

    /// The model named `name`.
    fn parse(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|cpu| cpu.name() == name)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|cpu| cpu.name()).collect();
                format!("--cpu {name:?}: the models are {}", names.join(", "))
            })
    }
}

/// Whether a step whose result exists runs again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assets {
    /// A result already in the work directory is used.
    Reuse,
    /// Every download and every build runs again.
    Rebuild,
}

/// Whether the runner is built before the boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerBuild {
    Build,
    /// The runner already built is used.
    Prebuilt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedKvmCommand {
    /// The board's clock. Its 50M default makes the nested guest crawl: a
    /// nested tick costs tens of thousands of instructions of exits.
    pub ips: InstructionsPerSecond,
    pub cpu: HostCpu,
    pub target: BootTarget,
    pub assets: Assets,
    pub runner: RunnerBuild,
    /// Where the downloads and the disk are kept; `<repo>/target/nested-kvm`
    /// when not given.
    pub dir: Option<PathBuf>,
    pub host_ram: Mebibytes,
    pub nested_ram: Mebibytes,
}

impl Default for NestedKvmCommand {
    fn default() -> Self {
        Self {
            ips: InstructionsPerSecond(1_000_000_000),
            cpu: HostCpu::CortexA57,
            target: BootTarget::NestedGuest,
            assets: Assets::Reuse,
            runner: RunnerBuild::Build,
            dir: None,
            host_ram: Mebibytes(1024),
            nested_ram: Mebibytes(256),
        }
    }
}

pub fn nested_kvm_usage() -> String {
    "usage: cargo xtask nested-kvm [--cpu cortex-a57|armv9.2] [--ips N] [--host-only] [--rebuild]\n\
     \x20                            [--no-build] [--dir PATH] [--host-ram-mib N] [--nested-ram-mib N]\n\
     \x20 --cpu MODEL          the board's processor: cortex-a57 (default; KVM in nVHE mode)\n\
     \x20                      or armv9.2 (KVM in VHE mode)\n\
     \x20 --ips N              emulated instructions a second (default 1000000000)\n\
     \x20 --host-only          stop at the host Alpine's shell, QEMU installed\n\
     \x20 --rebuild            download and build every asset again\n\
     \x20 --no-build           use the runner already built\n\
     \x20 --dir PATH           the downloads and the disk (default <repo>/target/nested-kvm)\n\
     \x20 --host-ram-mib N     the board's RAM (default 1024)\n\
     \x20 --nested-ram-mib N   the nested guest's RAM (default 256)"
        .to_owned()
}

pub fn parse_nested_kvm_args(args: &[String]) -> Result<NestedKvmCommand, String> {
    let mut command = NestedKvmCommand::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let mut value = || {
            index += 1;
            args.get(index)
                .ok_or_else(|| format!("{flag} needs a value\n{}", nested_kvm_usage()))
        };
        match flag {
            "--ips" => command.ips = InstructionsPerSecond(positive(flag, value()?)?),
            "--cpu" => command.cpu = HostCpu::parse(value()?)?,
            "--host-only" => command.target = BootTarget::HostShell,
            "--rebuild" => command.assets = Assets::Rebuild,
            "--no-build" => command.runner = RunnerBuild::Prebuilt,
            "--dir" => command.dir = Some(PathBuf::from(value()?)),
            "--host-ram-mib" => command.host_ram = Mebibytes(positive(flag, value()?)?),
            "--nested-ram-mib" => command.nested_ram = Mebibytes(positive(flag, value()?)?),
            "-h" | "--help" => return Err(nested_kvm_usage()),
            other => {
                return Err(format!(
                    "unknown nested-kvm argument {other:?}\n{}",
                    nested_kvm_usage()
                ))
            }
        }
        index += 1;
    }
    Ok(command)
}

/// `text` as a number above zero.
fn positive(flag: &str, text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(0) => Err(format!("{flag}: 0 is not a size")),
        Ok(n) => Ok(n),
        Err(e) => Err(format!("{flag} {text:?}: {e}")),
    }
}

pub fn execute_nested_kvm(command: NestedKvmCommand) -> Result<(), String> {
    let repo = crate::ci::repo_root()?;
    let work = WorkDir {
        root: command
            .dir
            .clone()
            .unwrap_or_else(|| repo.join("target").join("nested-kvm")),
    };
    fs::create_dir_all(&work.root).map_err(|e| format!("create {}: {e}", work.root.display()))?;
    let agent = ureq::AgentBuilder::new().try_proxy_from_env(true).build();
    let mut iso = IsoBytes {
        path: work.iso(),
        bytes: None,
    };

    fetch_iso(&agent, &work, command.assets)?;
    extract(
        &mut iso,
        KERNEL_IN_ISO,
        &work.image(),
        Extraction::ArmImage,
        command.assets,
    )?;
    extract(
        &mut iso,
        INITRAMFS_IN_ISO,
        &work.initramfs(),
        Extraction::AsIs,
        command.assets,
    )?;
    fetch_packages(&agent, &work, command.assets)?;
    build_disk(&work, &mut iso, command.assets)?;
    let tar_offset = tar_offset(&work)?;
    let runner = runner(&repo, command.runner)?;
    boot(&runner, &runner_args(&work, &command, tar_offset))
}

/// The work directory's files.
struct WorkDir {
    root: PathBuf,
}

impl WorkDir {
    fn iso(&self) -> PathBuf {
        self.root.join(ISO_NAME)
    }

    /// The kernel as the board's Linux boot protocol takes it.
    fn image(&self) -> PathBuf {
        self.root.join("Image")
    }

    fn initramfs(&self) -> PathBuf {
        self.root.join("initramfs-virt")
    }

    fn packages(&self) -> PathBuf {
        self.root.join("apks")
    }

    /// The packages' file names, one a line; written last, so its presence
    /// means every package is there and checked.
    fn package_list(&self) -> PathBuf {
        self.packages().join("packages.txt")
    }

    fn disk(&self) -> PathBuf {
        self.root.join("disk.img")
    }
}

impl Assets {
    /// Whether the step making `result` is skipped.
    fn kept(self, result: &Path) -> bool {
        match self {
            Self::Reuse => result.exists(),
            Self::Rebuild => false,
        }
    }
}

/// The ISO's bytes, read the first time a step needs them.
struct IsoBytes {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl IsoBytes {
    fn get(&mut self) -> Result<&[u8], String> {
        if self.bytes.is_none() {
            self.bytes = Some(read(&self.path)?);
        }
        match &self.bytes {
            Some(bytes) => Ok(bytes),
            None => Err(format!("{} was not read", self.path.display())),
        }
    }
}

/// 1. The ISO, downloaded beside its final name and moved there once its
///    SHA-512 is the one Alpine publishes.
fn fetch_iso(agent: &ureq::Agent, work: &WorkDir, assets: Assets) -> Result<(), String> {
    let iso = work.iso();
    if assets.kept(&iso) {
        return Ok(());
    }
    let url = format!("{ALPINE_MIRROR}/releases/{ARCH}/{ISO_NAME}");
    crate::step(&format!("downloading {ISO_NAME}"));
    let listing = fetch(agent, &format!("{url}.sha512"))?;
    let expected = published_sha512(&listing)?;
    let part = part_path(&iso);
    download(agent, &url, &part)?;
    let actual = sha512_hex(&read(&part)?);
    if actual != expected {
        return Err(format!(
            "{}: SHA-512 {actual}, Alpine publishes {expected}",
            part.display()
        ));
    }
    println!("SHA-512 {actual}: matches");
    rename(&part, &iso)
}

/// The digest of a `sha512sum` listing's first line, in lower case.
fn published_sha512(listing: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(listing).map_err(|e| format!("the SHA-512 listing: {e}"))?;
    let digest = text
        .split_whitespace()
        .next()
        .ok_or("the SHA-512 listing is empty")?
        .to_ascii_lowercase();
    if digest.len() == 128 && digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(digest)
    } else {
        Err(format!("the SHA-512 listing names {digest:?}"))
    }
}

fn sha512_hex(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What a file of the ISO becomes in the work directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extraction {
    /// The file itself.
    AsIs,
    /// The arm64 `Image` inside an EFI zboot kernel.
    ArmImage,
}

/// 2. A file of the ISO, by its Rock Ridge path, as `extraction` asks.
fn extract(
    iso: &mut IsoBytes,
    in_iso: &str,
    destination: &Path,
    extraction: Extraction,
    assets: Assets,
) -> Result<(), String> {
    if assets.kept(destination) {
        return Ok(());
    }
    crate::step(&format!("taking {in_iso} out of the ISO"));
    let file = IsoImage::open(iso.get()?)
        .and_then(|image| image.file(in_iso))
        .map_err(|e| format!("{ISO_NAME}: {e}"))?;
    let contents = match extraction {
        Extraction::AsIs => file,
        Extraction::ArmImage => zboot_image(&file).map_err(|e| format!("{in_iso}: {e}"))?,
    };
    write_atomically(destination, &contents)?;
    println!("{}: {} bytes", destination.display(), contents.len());
    Ok(())
}

/// The EFI zboot header (Linux `drivers/firmware/efi/libstub/zboot-header.S`):
/// a PE file whose bytes 4..8 are `zimg`, then the compressed payload's
/// offset and size (little-endian), and at 24 the compression's name.
const ZBOOT_MAGIC: &[u8] = b"zimg";
const ZBOOT_PAYLOAD_OFFSET_AT: usize = 8;
const ZBOOT_PAYLOAD_SIZE_AT: usize = 12;
const ZBOOT_COMPRESSION_AT: usize = 24;
const ZBOOT_COMPRESSION_LEN: usize = 8;
/// The arm64 `Image` header's magic (Linux `Documentation/arch/arm64/booting.rst`).
const ARM64_IMAGE_MAGIC_AT: usize = 56;
const ARM64_IMAGE_MAGIC: &[u8] = b"ARM\x64";

/// The kernel's arm64 `Image` out of an EFI zboot `vmlinuz`, whose payload
/// is gzip: the board's Linux boot protocol takes a plain `Image`.
fn zboot_image(vmlinuz: &[u8]) -> Result<Vec<u8>, String> {
    if vmlinuz.get(..2) != Some(b"MZ".as_slice()) || vmlinuz.get(4..8) != Some(ZBOOT_MAGIC) {
        return Err("not an EFI zboot image".into());
    }
    let word = |at: usize| -> Result<usize, String> {
        vmlinuz
            .get(at..at + 4)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_le_bytes)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| "the zboot header is cut short".to_owned())
    };
    let offset = word(ZBOOT_PAYLOAD_OFFSET_AT)?;
    let size = word(ZBOOT_PAYLOAD_SIZE_AT)?;
    let compression = vmlinuz
        .get(ZBOOT_COMPRESSION_AT..ZBOOT_COMPRESSION_AT + ZBOOT_COMPRESSION_LEN)
        .ok_or("the zboot header is cut short")?;
    let compression = String::from_utf8_lossy(compression);
    let compression = compression.trim_end_matches('\0');
    if compression != "gzip" {
        return Err(format!("the payload is {compression}, only gzip is read"));
    }
    let payload = vmlinuz
        .get(offset..offset.saturating_add(size))
        .ok_or("the zboot payload runs past the file")?;
    let mut image = Vec::new();
    flate2::bufread::GzDecoder::new(payload)
        .read_to_end(&mut image)
        .map_err(|e| format!("the zboot payload: {e}"))?;
    if image.get(ARM64_IMAGE_MAGIC_AT..ARM64_IMAGE_MAGIC_AT + 4) != Some(ARM64_IMAGE_MAGIC) {
        return Err("the payload is not an arm64 Image".into());
    }
    Ok(image)
}

/// 3. QEMU and every package it depends on, each checked against its
///    repository's signed index.
fn fetch_packages(agent: &ureq::Agent, work: &WorkDir, assets: Assets) -> Result<(), String> {
    let list = work.package_list();
    if assets.kept(&list) {
        return Ok(());
    }
    crate::step("downloading QEMU's packages");
    let directory = work.packages();
    crate::remove_dir_if_exists(&directory)?;
    fs::create_dir_all(&directory).map_err(|e| format!("create {}: {e}", directory.display()))?;
    let keys: ApkKeys = apk::initramfs_keys(&read(&work.initramfs())?)?;
    println!("keys: {}", keys.names().join(", "));
    let mut index = Index::default();
    for repository in Repository::ALL {
        let url = format!(
            "{ALPINE_MIRROR}/{}/{ARCH}/APKINDEX.tar.gz",
            repository.name()
        );
        let signed =
            apk::verify_index(&fetch(agent, &url)?, &keys).map_err(|e| format!("{url}: {e}"))?;
        println!(
            "{}: index signed by {}, verified",
            repository.name(),
            signed.signer
        );
        index.add(&signed.text, repository)?;
    }
    let wanted = index.resolve(&[QEMU_PACKAGE])?;
    println!("{} packages", wanted.len());
    let mut names = String::new();
    for package in &wanted {
        let file_name = package.file_name();
        let url = format!(
            "{ALPINE_MIRROR}/{}/{ARCH}/{file_name}",
            package.repository.name()
        );
        let bytes = fetch(agent, &url)?;
        apk::check_package(package, &bytes)?;
        write_atomically(&directory.join(&file_name), &bytes)?;
        names.push_str(&file_name);
        names.push('\n');
    }
    println!("all {} checksums match their signed index", wanted.len());
    write_atomically(&list, names.as_bytes())
}

/// The MiB at which the disk's package tar starts: the ISO's size, rounded
/// up to a whole MiB.
fn tar_offset(work: &WorkDir) -> Result<u64, String> {
    let iso = work.iso();
    let length = fs::metadata(&iso)
        .map_err(|e| format!("{}: {e}", iso.display()))?
        .len();
    Ok(length.div_ceil(MIB))
}

/// 4. The disk: the ISO, zeros to the next MiB, a tar of the packages.
fn build_disk(work: &WorkDir, iso: &mut IsoBytes, assets: Assets) -> Result<(), String> {
    let disk = work.disk();
    if assets.kept(&disk) {
        return Ok(());
    }
    crate::step("building disk.img");
    let list = read(&work.package_list())?;
    let list = String::from_utf8(list).map_err(|e| format!("the package list: {e}"))?;
    let iso_bytes = iso.get()?;
    let iso_length = iso_bytes.len() as u64;
    let padding = iso_length.div_ceil(MIB) * MIB - iso_length;
    let part = part_path(&disk);
    let writing = |e: io::Error| format!("write {}: {e}", part.display());
    let mut file = BufWriter::new(File::create(&part).map_err(writing)?);
    file.write_all(iso_bytes).map_err(writing)?;
    let zeros = io::copy(&mut io::repeat(0).take(padding), &mut file).map_err(writing)?;
    if zeros != padding {
        return Err(format!(
            "{}: {zeros} bytes of padding written, not {padding}",
            part.display()
        ));
    }
    let mut tar = tar::Builder::new(file);
    tar.mode(tar::HeaderMode::Deterministic);
    let packages = work.packages();
    tar.append_dir("qemu", &packages).map_err(writing)?;
    for name in list.lines().filter(|line| !line.is_empty()) {
        tar.append_path_with_name(packages.join(name), format!("qemu/{name}"))
            .map_err(writing)?;
    }
    let file = tar.into_inner().map_err(writing)?;
    let file = file.into_inner().map_err(|e| writing(e.into_error()))?;
    file.sync_all().map_err(writing)?;
    rename(&part, &disk)?;
    println!(
        "{}: the ISO, then the packages' tar at {} MiB",
        disk.display(),
        iso_length.div_ceil(MIB)
    );
    Ok(())
}

/// 5. The runner, built unless `build` says it is already.
fn runner(repo: &Path, build: RunnerBuild) -> Result<PathBuf, String> {
    let target = match env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => repo.join(dir),
        None => repo.join("target"),
    };
    let runner = target
        .join("release")
        .join("examples")
        .join(format!("linux_virt{}", env::consts::EXE_SUFFIX));
    match build {
        RunnerBuild::Build => {
            crate::step("building the runner");
            let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
            let status = Command::new(cargo)
                .args([
                    "build",
                    "--release",
                    "-p",
                    "rusty_box_aarch64",
                    "--features",
                    "std",
                    "--example",
                    "linux_virt",
                ])
                .current_dir(repo)
                .status()
                .map_err(|e| format!("run cargo: {e}"))?;
            if !status.success() {
                return Err(format!("building the runner failed: {status}"));
            }
        }
        RunnerBuild::Prebuilt => {}
    }
    if runner.is_file() {
        Ok(runner)
    } else {
        Err(format!(
            "no runner at {} (drop --no-build)",
            runner.display()
        ))
    }
}

/// The runner's command line: the processor `command.cpu` names with EL2
/// booting the ISO's kernel, and a console script that logs in, installs
/// QEMU from the disk's tar and, for [`BootTarget::NestedGuest`], starts
/// the nested guest. `\r` in a typed string is the runner's own escape.
fn runner_args(work: &WorkDir, command: &NestedKvmCommand, tar_offset: u64) -> Vec<OsString> {
    let install = format!(
        "mkdir -p /tmp/q && dd if=/dev/vda bs=1M skip={tar_offset} 2>/dev/null | tar -x -C /tmp/q \
         && apk add --quiet --no-progress --no-network --force-non-repository /tmp/q/qemu/*.apk \
         && echo QEMU-READY\\r"
    );
    let mut args: Vec<OsString> = vec![
        "--cpu".into(),
        command.cpu.name().into(),
        "--virtualization".into(),
        "--ips".into(),
        command.ips.0.to_string().into(),
        "--kernel".into(),
        work.image().into(),
        "--initrd".into(),
        work.initramfs().into(),
        "--append".into(),
        KERNEL_COMMAND_LINE.into(),
        "--disk".into(),
        work.disk().into(),
        "--ram-mib".into(),
        command.host_ram.0.to_string().into(),
        "--interactive".into(),
        "--until".into(),
        "login: ".into(),
        "--type-serial".into(),
        "root\\r".into(),
        "--until".into(),
        "# ".into(),
        "--type-serial".into(),
        install.into(),
        "--until".into(),
        "QEMU-READY".into(),
        "--until".into(),
        "# ".into(),
    ];
    match command.target {
        BootTarget::NestedGuest => {
            let nested = format!(
                "qemu-system-aarch64 -M virt,gic-version=2 -accel kvm -cpu host -m {} \
                 -nographic -no-reboot -kernel /media/vda/{KERNEL_IN_ISO} \
                 -initrd /media/vda/{INITRAMFS_IN_ISO} \
                 -drive if=virtio,format=raw,readonly=on,file=/dev/vda \
                 -append '{KERNEL_COMMAND_LINE}'\\r",
                command.nested_ram.0
            );
            args.push("--type-serial".into());
            args.push(nested.into());
        }
        BootTarget::HostShell => {}
    }
    args
}

/// 6. The boot, on this terminal; the runner's exit status is this
///    command's.
fn boot(runner: &Path, args: &[OsString]) -> Result<(), String> {
    crate::step("booting (the first prompt takes a minute or two)");
    println!("Ctrl-] quits the emulator; Ctrl-A x quits the nested QEMU");
    let status = Command::new(runner)
        .args(args)
        .status()
        .map_err(|e| format!("run {}: {e}", runner.display()))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => {
            eprintln!("the runner exited with status {code}");
            std::process::exit(code)
        }
        None => Err(format!("the runner ended without an exit code: {status}")),
    }
}

/// The body of `url`, tried [`DOWNLOAD_ATTEMPTS`] times.
fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    with_attempts(url, || {
        let response = agent.get(url).call().map_err(|e| e.to_string())?;
        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    })
}

/// The body of `url` written to `destination`, tried [`DOWNLOAD_ATTEMPTS`]
/// times.
fn download(agent: &ureq::Agent, url: &str, destination: &Path) -> Result<(), String> {
    with_attempts(url, || {
        let response = agent.get(url).call().map_err(|e| e.to_string())?;
        let mut file = File::create(destination)
            .map_err(|e| format!("create {}: {e}", destination.display()))?;
        let written = io::copy(&mut response.into_reader(), &mut file)
            .map_err(|e| format!("write {}: {e}", destination.display()))?;
        println!("{}: {written} bytes", destination.display());
        Ok(())
    })
}

fn with_attempts<T, F>(url: &str, mut attempt: F) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    let mut tried = 1;
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(e) if tried < DOWNLOAD_ATTEMPTS => {
                eprintln!("{url}: {e}; trying again");
                tried += 1;
            }
            Err(e) => return Err(format!("download {url}: {e}")),
        }
    }
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// `path` with `.part` appended: where a result is made before it takes
/// its name.
fn part_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// `bytes` at `path`, which exists only once they are all written.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let part = part_path(path);
    fs::write(&part, bytes).map_err(|e| format!("write {}: {e}", part.display()))?;
    rename(&part, path)
}

fn rename(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(|e| format!("rename {} to {}: {e}", from.display(), to.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_options_boot_the_nested_guest_at_a_gigahertz() {
        let command = parse_nested_kvm_args(&[]).expect("parse");
        assert_eq!(command, NestedKvmCommand::default());
        assert_eq!(command.ips, InstructionsPerSecond(1_000_000_000));
        assert_eq!(command.cpu, HostCpu::CortexA57);
        assert_eq!(command.target, BootTarget::NestedGuest);
        assert_eq!(command.host_ram, Mebibytes(1024));
        assert_eq!(command.nested_ram, Mebibytes(256));
    }

    #[test]
    fn every_option_is_read() {
        let command = parse_nested_kvm_args(&args(&[
            "--ips",
            "300000000",
            "--cpu",
            "armv9.2",
            "--host-only",
            "--rebuild",
            "--no-build",
            "--dir",
            "scratch",
            "--host-ram-mib",
            "2048",
            "--nested-ram-mib",
            "512",
        ]))
        .expect("parse");
        assert_eq!(
            command,
            NestedKvmCommand {
                ips: InstructionsPerSecond(300_000_000),
                cpu: HostCpu::Armv9p2,
                target: BootTarget::HostShell,
                assets: Assets::Rebuild,
                runner: RunnerBuild::Prebuilt,
                dir: Some(PathBuf::from("scratch")),
                host_ram: Mebibytes(2048),
                nested_ram: Mebibytes(512),
            }
        );
    }

    #[test]
    fn bad_options_are_refused() {
        assert!(parse_nested_kvm_args(&args(&["--ips"])).is_err());
        assert!(parse_nested_kvm_args(&args(&["--ips", "0"])).is_err());
        assert!(parse_nested_kvm_args(&args(&["--host-ram-mib", "lots"])).is_err());
        assert!(parse_nested_kvm_args(&args(&["--nested"])).is_err());
        assert!(parse_nested_kvm_args(&args(&["--cpu"])).is_err());
        assert!(parse_nested_kvm_args(&args(&["--cpu", "max"])).is_err());
    }

    /// The runner gets the processor `--cpu` names, with EL2.
    #[test]
    fn the_runner_boots_the_processor_asked_for_with_virtualization() {
        let work = WorkDir {
            root: PathBuf::from("w"),
        };
        for (cpu, name) in [
            (HostCpu::CortexA57, "cortex-a57"),
            (HostCpu::Armv9p2, "armv9.2"),
        ] {
            let command = NestedKvmCommand {
                cpu,
                ..NestedKvmCommand::default()
            };
            let args = joined(&runner_args(&work, &command, 77));
            let at = args.iter().position(|a| a == "--cpu").expect("--cpu");
            assert_eq!(args[at + 1], name);
            assert!(args.iter().any(|a| a == "--virtualization"));
            assert_eq!(HostCpu::parse(name), Ok(cpu));
        }
    }

    fn joined(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_script_unpacks_the_tar_at_its_offset_and_starts_the_nested_guest() {
        let work = WorkDir {
            root: PathBuf::from("w"),
        };
        let command = NestedKvmCommand::default();
        let args = joined(&runner_args(&work, &command, 77));
        let install = args
            .iter()
            .find(|a| a.contains("QEMU-READY\\r"))
            .expect("the install step");
        assert!(install.contains("dd if=/dev/vda bs=1M skip=77 "));
        let last = args.last().expect("a last step");
        assert!(last.starts_with("qemu-system-aarch64 "));
        assert!(last.contains("-m 256 "));
        assert!(last.contains("file=/dev/vda "));
        assert!(last.ends_with("usb-storage'\\r"));
        assert!(!last.contains('\r'));
        let ips = args.iter().position(|a| a == "--ips").expect("--ips");
        assert_eq!(args[ips + 1], "1000000000");
    }

    #[test]
    fn host_only_stops_at_the_host_shell() {
        let work = WorkDir {
            root: PathBuf::from("w"),
        };
        let command = NestedKvmCommand {
            target: BootTarget::HostShell,
            ..NestedKvmCommand::default()
        };
        let args = joined(&runner_args(&work, &command, 77));
        assert_eq!(args.last().map(String::as_str), Some("# "));
        assert!(!args.iter().any(|a| a.contains("qemu-system-aarch64")));
    }

    fn zboot(payload_kind: &[u8], image: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(image).expect("compress");
        let payload = encoder.finish().expect("finish");
        let mut file = vec![0u8; 64];
        file[..2].copy_from_slice(b"MZ");
        file[4..8].copy_from_slice(ZBOOT_MAGIC);
        file[8..12].copy_from_slice(&64u32.to_le_bytes());
        file[12..16].copy_from_slice(&u32::try_from(payload.len()).expect("fits").to_le_bytes());
        file[24..24 + payload_kind.len()].copy_from_slice(payload_kind);
        file.extend_from_slice(&payload);
        file.extend_from_slice(b"PE trailer");
        file
    }

    #[test]
    fn the_image_comes_out_of_a_gzip_zboot_kernel() {
        let mut image = vec![0u8; 4096];
        image[ARM64_IMAGE_MAGIC_AT..ARM64_IMAGE_MAGIC_AT + 4].copy_from_slice(ARM64_IMAGE_MAGIC);
        image[100] = 0x5a;
        assert_eq!(zboot_image(&zboot(b"gzip", &image)).expect("image"), image);
        assert!(zboot_image(&zboot(b"zstd22", &image)).is_err());
        assert!(zboot_image(&zboot(b"gzip", &[0u8; 4096])).is_err());
        assert!(zboot_image(&image).is_err());
    }

    #[test]
    fn the_published_digest_is_the_listing_first_word() {
        let digest = "E7".repeat(64);
        let listing = format!("{digest}  {ISO_NAME}\n");
        assert_eq!(
            published_sha512(listing.as_bytes()).expect("digest"),
            digest.to_ascii_lowercase()
        );
        assert!(published_sha512(b"abc  file\n").is_err());
        assert!(published_sha512(b"").is_err());
    }
}
