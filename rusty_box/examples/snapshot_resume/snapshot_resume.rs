//! Snapshot suspend/resume acceptance harness.
//!
//! Proves the user-facing snapshot workflow end to end: boot a real guest
//! from CD for a bounded instruction budget, save a v4 snapshot to disk,
//! then restore it in a FRESH PROCESS with the same media attached and
//! continue executing.
//!
//! Run (defaults target the Windows 7 install ISO):
//!   cargo run --release --example snapshot_resume --features std
//!
//! Environment:
//!   RB_ISO          ISO path (default: C:\Users\olegg\Downloads\Windows.7.SP1.7601.28064.OneSmiLe.iso)
//!   RB_MEM_MIB      Guest/host RAM in MiB (default 2048)
//!   RB_BOOT_INSNS   Instructions before the snapshot (default 1_000_000_000)
//!   RB_RESUME_INSNS Instructions after the restore (default 200_000_000)
//!   RB_SNAPSHOT     Snapshot file (default target/snapshot_resume.rbx)

use rusty_box::{
    cpu::{core_i7_skylake::Corei7SkylakeX, ResetReason},
    emulator::{Emulator, EmulatorConfig},
    gui::NoGui,
};

const DEFAULT_ISO: &str = r"C:\Users\olegg\Downloads\Windows.7.SP1.7601.28064.OneSmiLe.iso";

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_string(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

struct HarnessConfig {
    iso: String,
    mem_mib: u64,
    boot_insns: u64,
    resume_insns: u64,
    snapshot_path: String,
}

impl HarnessConfig {
    fn from_env() -> Self {
        Self {
            iso: env_string("RB_ISO", DEFAULT_ISO),
            mem_mib: env_u64("RB_MEM_MIB", 2048),
            boot_insns: env_u64("RB_BOOT_INSNS", 1_000_000_000),
            resume_insns: env_u64("RB_RESUME_INSNS", 200_000_000),
            snapshot_path: env_string("RB_SNAPSHOT", "target/snapshot_resume.rbx"),
        }
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "full".to_string());
    std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .name("snapshot_resume".to_string())
        .spawn(move || run(&mode))
        .expect("spawn harness thread")
        .join()
        .expect("harness thread panicked");
}

fn run(mode: &str) {
    let config = HarnessConfig::from_env();
    match mode {
        // Full mode drives both phases as separate OS processes so the
        // restore proves a genuinely fresh machine, not warm in-process state.
        "full" => {
            let exe = std::env::current_exe().expect("current exe");
            for phase in ["boot-save", "restore-run"] {
                let status = std::process::Command::new(&exe)
                    .arg(phase)
                    .status()
                    .expect("spawn phase process");
                assert!(status.success(), "phase {phase} failed: {status}");
            }
            println!("PASS: snapshot suspend/resume round trip complete");
        }
        "boot-save" => boot_save(&config),
        "restore-run" => restore_run(&config),
        other => panic!("unknown mode {other:?} (use full | boot-save | restore-run)"),
    }
}

fn build_machine(config: &HarnessConfig) -> Box<Emulator<'static, Corei7SkylakeX>> {
    let workspace_root = workspace_root();
    let bios = std::fs::read(
        workspace_root.join("cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"),
    )
    .expect("read BIOS-bochs-latest");
    let vga_bios = std::fs::read(
        workspace_root.join("cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"),
    )
    .expect("read VGABIOS-lgpl-latest.bin");

    let mem_bytes = usize::try_from(config.mem_mib * 1024 * 1024).expect("memory size");
    let emulator_config = EmulatorConfig {
        guest_memory_size: mem_bytes,
        host_memory_size: mem_bytes,
        ips: 120_000_000,
        pci_enabled: true,
        ..EmulatorConfig::default()
    };

    let mut emu = Emulator::<Corei7SkylakeX>::new(emulator_config).expect("build emulator");
    emu.set_gui(NoGui::new());
    emu.init_memory_and_pc_system().expect("init memory/pc-system");

    let bios_load_addr = !(bios.len() as u64 - 1);
    emu.load_bios(&bios, bios_load_addr).expect("load BIOS");
    emu.load_optional_rom(&vga_bios, 0xC0000).expect("load VGA BIOS");
    emu.init_cpu_and_devices().expect("init CPU/devices");
    emu.configure_memory_in_cmos_from_config();
    // ELTORITO boot codes: 3 = cdrom first.
    emu.configure_boot_sequence(3, 0, 0);
    emu.attach_cdrom(0, 0, &config.iso).expect("attach ISO");
    emu.reset(ResetReason::Hardware).expect("hardware reset");
    emu
}

fn workspace_root() -> std::path::PathBuf {
    let mut dir = std::env::current_dir().expect("cwd");
    loop {
        if dir.join("cpp_orig").exists() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => panic!("workspace root with cpp_orig/ not found"),
        }
    }
}

fn run_instructions(emu: &mut Emulator<'static, Corei7SkylakeX>, budget: u64) -> u64 {
    let mut executed_total = 0u64;
    while executed_total < budget {
        let chunk = (budget - executed_total).min(50_000_000);
        let (executed, shutdown) = emu.step_batch(chunk).expect("step_batch");
        assert!(!shutdown, "guest shut down inside the instruction budget");
        assert!(executed > 0, "guest made no progress");
        executed_total += executed;
    }
    executed_total
}

fn boot_save(config: &HarnessConfig) {
    println!(
        "boot-save: booting {} for {} instructions",
        config.iso, config.boot_insns
    );
    let start = std::time::Instant::now();
    let mut emu = build_machine(config);
    let executed = run_instructions(&mut emu, config.boot_insns);
    println!(
        "boot-save: executed {} instructions in {:?}",
        executed,
        start.elapsed()
    );

    let mut file =
        std::io::BufWriter::new(std::fs::File::create(&config.snapshot_path).expect("create snapshot file"));
    emu.save_snapshot(&mut file).expect("save snapshot");
    use std::io::Write as _;
    file.flush().expect("flush snapshot");
    println!(
        "boot-save: snapshot written to {} ({} bytes)",
        config.snapshot_path,
        std::fs::metadata(&config.snapshot_path)
            .map(|meta| meta.len())
            .unwrap_or(0)
    );
}

fn restore_run(config: &HarnessConfig) {
    println!(
        "restore-run: restoring {} into a fresh machine",
        config.snapshot_path
    );
    let start = std::time::Instant::now();
    let mut emu = build_machine(config);
    let mut file =
        std::io::BufReader::new(std::fs::File::open(&config.snapshot_path).expect("open snapshot file"));
    emu.restore_snapshot(&mut file).expect("restore snapshot");
    println!("restore-run: restored in {:?}", start.elapsed());

    let executed = run_instructions(&mut emu, config.resume_insns);
    println!(
        "restore-run: continued {} instructions in {:?} — PASS",
        executed,
        start.elapsed()
    );
}
