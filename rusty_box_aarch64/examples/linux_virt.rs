//! Boot a Linux kernel on the rusty_box AArch64 interpreter the way QEMU's
//! `virt` board does for `-kernel` without firmware, printing its serial
//! console. The loading itself — where the `Image`, initramfs and device
//! tree go, and the register state the kernel is entered with — is the
//! library's ([`rusty_box_aarch64::boot`]); this is the command line around
//! it.
//!
//! ```text
//! cargo run --release -p rusty_box_aarch64 --example linux_virt --features std -- \
//!     --kernel Image --initrd initramfs.cpio.gz \
//!     --until 'login: ' --type 'root\r' --until '# ' \
//!     --type 'cat /etc/alpine-release\r' --until '# '
//! ```
//!
//! The command line defaults to the serial console with early messages;
//! `--append` replaces it. The device tree is QEMU's for this board unless
//! `--dtb` names another, and `--dump-dtb` writes the one the kernel was
//! given.

#[path = "common/mod.rs"]
mod common;

use std::process::ExitCode;
use std::time::Instant;

use common::{Parsed, RunArgs, RUN_USAGE};
use rusty_box_aarch64::boot::{LinuxImage, SERIAL_CONSOLE_COMMAND_LINE};
use rusty_box_aarch64::{MachineBuilder, RamSize, VirtConfig};

struct Args {
    kernel: String,
    initrd: Option<String>,
    append: String,
    dtb: Option<String>,
    dump_dtb: Option<String>,
    ram_mib: u64,
    run: RunArgs,
}

fn usage() -> String {
    format!(
        "usage: linux_virt --kernel <Image> [--initrd <file>] [--append <command line>]\n\
         \x20      [--dtb <file>] [--dump-dtb <file>] [--ram-mib N] {RUN_USAGE}"
    )
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        kernel: String::new(),
        initrd: None,
        append: String::from(SERIAL_CONSOLE_COMMAND_LINE),
        dtb: None,
        dump_dtb: None,
        ram_mib: RamSize::DEFAULT.get() >> 20,
        run: RunArgs::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        if let Parsed::Consumed = a.run.parse_flag(&flag, &mut value)? {
            continue;
        }
        match flag.as_str() {
            "--kernel" => a.kernel = value("--kernel")?,
            "--initrd" => a.initrd = Some(value("--initrd")?),
            "--append" => a.append = value("--append")?,
            "--dtb" => a.dtb = Some(value("--dtb")?),
            "--dump-dtb" => a.dump_dtb = Some(value("--dump-dtb")?),
            "--ram-mib" => a.ram_mib = value("--ram-mib")?.parse().map_err(|e| format!("--ram-mib: {e}"))?,
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
    }
    if a.kernel.is_empty() {
        return Err(usage());
    }
    Ok(a)
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn main() -> ExitCode {
    common::on_machine_thread(boot)
}

fn boot() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let loaded = (|| -> Result<_, String> {
        let kernel = read(&args.kernel)?;
        let initrd = args.initrd.as_deref().map(read).transpose()?;
        let dtb = args.dtb.as_deref().map(read).transpose()?;
        let ram = RamSize::mib(args.ram_mib).ok_or("--ram-mib is not a size the board can map")?;
        Ok((kernel, initrd, dtb, ram, args.run.ips()?))
    })();
    let (kernel, initrd, dtb, ram, ips) = match loaded {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let image = match LinuxImage::parse(&kernel) {
        Ok(image) => image,
        Err(e) => {
            eprintln!("{}: {e}", args.kernel);
            return ExitCode::from(2);
        }
    };
    let config = VirtConfig {
        ips,
        debug_stops: args.run.debug_stops(),
        ..VirtConfig::DEFAULT
    };
    let plugged = common::plug_devices(MachineBuilder::new(config).ram(ram).linux(image).command_line(&args.append), &args.run);
    let mut builder = match plugged {
        Ok(builder) => builder,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if let Some(initrd) = &initrd {
        builder = builder.initrd(initrd);
    }
    if let Some(dtb) = &dtb {
        builder = builder.device_tree(dtb);
    }
    let mut machine = match builder.build() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cannot build the machine: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(path) = &args.dump_dtb {
        if let Err(e) = std::fs::write(path, machine.device_tree()) {
            eprintln!("{path}: {e}");
            return ExitCode::from(2);
        }
    }

    let started = Instant::now();
    let outcome = common::run(&mut machine, &args.run);
    common::report(&machine, &args.run, &outcome, started);
    if let Err(e) = common::close_disk(&mut machine) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    common::exit_code(&args.run, &outcome)
}
