# rusty_box_gui

`rusty_box_gui` is the normal-user Rusty Box launcher. It parses typed CLI flags, optionally merges a TOML config file, validates the resolved boot setup, and starts the existing `rusty_box` emulator APIs.

This crate is intentionally a runner boundary, not an egui application. The `gui-egui` feature only forwards `rusty_box/gui-egui` so future GUI code can be added without changing the CLI/config surface.

## Quick start

Run directly from flags:

```powershell
cargo run -p rusty_box_gui -- `
  --no-config `
  --display headless `
  --bios cpp_orig/bochs/bios/BIOS-bochs-latest `
  --vga-bios cpp_orig/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin `
  --disk dlxlinux/hd10meg.img `
  --disk-chs 306:4:17 `
  --boot disk `
  --memory-mib 32 `
  --ips 15000000 `
  --max-instructions 1000
```

Expected success output ends with:

```text
rusty_box_gui: executed <N> instructions
```

## Configuration file

With no flags, the runner loads `rusty_box.toml` from the current working directory if it exists. Use `--config PATH` to load a specific file, or `--no-config` to skip TOML completely.

Example:

```toml
[emulator]
memory_mib = 32
ips = 15000000
max_instructions = 1000
pci = true
sync_slowdown = false

[display]
backend = "headless"

[rom]
bios = "cpp_orig/bochs/bios/BIOS-bochs-latest"
vga_bios = "cpp_orig/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"

[boot]
order = ["disk"]

[disk]
path = "dlxlinux/hd10meg.img"
chs = { cylinders = 306, heads = 4, sectors_per_track = 17 }
channel = 0
drive = 0

[logging]
level = "warn"
```

CLI flags override TOML values without clearing unrelated TOML fields. For example, this changes memory while still using ROM and disk paths from the file:

```powershell
cargo run -p rusty_box_gui -- --config rusty_box.toml --memory-mib 64 --max-instructions 1000
```

## Resolution and validation

Merge order is:

1. built-in defaults,
2. TOML config,
3. explicit CLI flags.

Defaults:

| Field | Default |
| --- | --- |
| `emulator.memory_mib` | `32` |
| `emulator.host_memory_mib` | `memory_mib` |
| `emulator.memory_block_kib` | `128` |
| `emulator.ips` | `4000000` |
| `emulator.pci` | `true` |
| `emulator.sync_slowdown` | `false` |
| `emulator.max_instructions` | `u64::MAX` |
| `display.backend` | `terminal` |
| `logging.level` | `warn` |
| `disk.channel`, `disk.drive` | `0`, `0` |
| `cdrom.channel`, `cdrom.drive` | `1`, `0` |

Validation highlights:

- BIOS is required.
- Memory sizes and IPS must be non-zero.
- Boot order accepts `disk` and `cdrom`, supports at most three entries, and rejects duplicates.
- Disk boot requires disk media; CD-ROM boot requires CD-ROM media.
- Disk CHS accepts `CYLINDERS:HEADS:SPT` or `CYLINDERS,HEADS,SPT` on the CLI.
- If disk CHS is omitted, the runner auto-detects CHS from a non-empty 512-byte-aligned disk image.
- Disk and CD-ROM ATA slots must be in range and cannot overlap.
- `attach_disk` and `attach_cdrom` currently require UTF-8 paths because the core emulator APIs take `&str`.

## Public API

The crate re-exports the runner surface from `src/lib.rs`:

- `Args`
- `BootDevice`
- `DiskGeometry`
- `DisplayBackend`
- `FileConfig`
- `ResolvedConfig`
- `RunError`
- `RunSummary`
- `run`
- `run_resolved`

Use `run(args)` for normal CLI entrypoints. Use `run_resolved(config)` when another frontend already has a validated `ResolvedConfig`.

## Verification

Useful checks while changing this crate:

```powershell
cargo test -p rusty_box_gui
cargo check -p rusty_box_gui --features gui-egui
cargo doc -p rusty_box_gui --no-deps
```
