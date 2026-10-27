//! Boot UEFI firmware built for QEMU's `virt` (EDK2 `ArmVirtQemu`) on the
//! rusty_box AArch64 interpreter, printing the firmware's serial console.
//!
//! ```text
//! cargo run --release -p rusty_box_aarch64 --example uefi_virt --features std -- \
//!     --code /usr/share/AAVMF/AAVMF_CODE.no-secboot.fd \
//!     --vars /usr/share/AAVMF/AAVMF_VARS.fd \
//!     --until 'Shell>' --type 'ver\r' --until 'Shell>'
//! ```
//!
//! `--until` and `--type` form a script, run in order: wait for text on the
//! console, then type keys (`\r` is Enter, `\e` Escape). The run succeeds when
//! the whole script completes. `--type` types on the machine's keyboard,
//! which is the virtio keyboard when `--virtio-keyboard` plugs one in;
//! `--type-serial` always types on the serial console's terminal.
//!
//! The variable store is copied, never written back, unless `--persist-vars`
//! names a file to save it to. The device tree is QEMU's for this board
//! unless `--dtb` names another; either way its memory node is set to
//! `--ram-mib`.

#[path = "common/mod.rs"]
mod common;

use std::process::ExitCode;
use std::time::Instant;

use common::{Parsed, RunArgs, RUN_USAGE};
use rusty_box_aarch64::{MachineBuilder, RamSize, VirtConfig};

struct Args {
    code: String,
    vars: Option<String>,
    dtb: Option<String>,
    ram_mib: u64,
    persist_vars: Option<String>,
    run: RunArgs,
}

fn usage() -> String {
    format!(
        "usage: uefi_virt --code <CODE.fd> [--vars <VARS.fd>] [--dtb <file>] [--ram-mib N]\n\
         \x20      [--persist-vars <file>] {RUN_USAGE}"
    )
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        code: String::new(),
        vars: None,
        dtb: None,
        ram_mib: RamSize::DEFAULT.get() >> 20,
        persist_vars: None,
        run: RunArgs::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        if let Parsed::Consumed = a.run.parse_flag(&flag, &mut value)? {
            continue;
        }
        match flag.as_str() {
            "--code" => a.code = value("--code")?,
            "--vars" => a.vars = Some(value("--vars")?),
            "--dtb" => a.dtb = Some(value("--dtb")?),
            "--ram-mib" => {
                a.ram_mib = value("--ram-mib")?
                    .parse()
                    .map_err(|e| format!("--ram-mib: {e}"))?
            }
            "--persist-vars" => a.persist_vars = Some(value("--persist-vars")?),
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
    }
    if a.code.is_empty() {
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
        let code = read(&args.code)?;
        let vars = args.vars.as_deref().map(read).transpose()?;
        let dtb = args.dtb.as_deref().map(read).transpose()?;
        let ram = RamSize::mib(args.ram_mib).ok_or("--ram-mib is not a size the board can map")?;
        Ok((code, vars, dtb, ram, args.run.ips()?))
    })();
    let (code, vars, dtb, ram, ips) = match loaded {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let config = VirtConfig {
        ips,
        cpu: args.run.cpu,
        debug_stops: args.run.debug_stops(),
        ..VirtConfig::DEFAULT
    };
    let mut builder = match common::plug_devices(
        MachineBuilder::new(config).ram(ram).firmware(&code),
        &args.run,
    ) {
        Ok(builder) => builder,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if let Some(vars) = &vars {
        builder = builder.variables(vars);
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

    let started = Instant::now();
    let outcome = common::run(&mut machine, &args.run);
    common::report(&machine, &args.run, &outcome, started);
    if let Some(path) = &args.persist_vars {
        if let Err(e) = std::fs::write(path, machine.variable_store()) {
            eprintln!("{path}: {e}");
        }
    }
    if let Err(e) = common::close_disk(&mut machine) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    common::exit_code(&args.run, &outcome)
}
