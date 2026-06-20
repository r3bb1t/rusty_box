# rusty_box_gui

`rusty_box_gui` is the normal-user Rusty Box launcher. Native builds parse typed CLI flags, merge optional TOML config, validate media, and start the existing `rusty_box` emulator APIs. The default egui frontend is a modern VMware-style Rusty Box Workstation shell; browser builds use the same shell structure with a WebRunner runtime.

RustyBox GUI is not VMware; it uses VMware-like organization for familiarity while retaining Rusty Box emulator constraints.

## Desktop

Run the native graphical frontend with the local `rusty_box.toml`:

```powershell
cargo run -p rusty_box_gui
```

With no flags, the runner loads `rusty_box.toml` from the current working directory if it exists. Use `--config PATH` to load a specific file, or `--no-config` to skip TOML completely.

The desktop egui shell runs `eframe` on the main thread and the emulator on a large-stack worker thread. It includes a Library sidebar, toolbar, Home, Hardware, Images, and Console pages. The Images page uses native `Browse...` save dialogs and accepts drag/drop to fill the target image path.

Power controls are state-aware: `Power On` starts the selected Library profile, `Restart VM` and `Power Off` only affect a running VM, and startup errors are surfaced in the shell. Hardware panes edit guest memory, CPU/IPS, boot device, disk/CD-ROM attachment, and ROM paths before launch. The Library can duplicate, rename, select, and delete VM profiles while the VM is stopped, and the toolbar `Library` checkbox can hide the sidebar so the Console can scale wider. The Console can show the serial log and send serial input lines while a VM is running.

Direct flags still work:

```powershell
cargo run -p rusty_box_gui -- `
  --no-config `
  --display egui `
  --bios C:/path/BIOS-bochs-latest `
  --vga-bios C:/path/VGABIOS-lgpl-latest.bin `
  --cdrom C:/path/alpine-virt.iso `
  --boot cdrom `
  --memory-mib 256 `
  --ips 15000000
```

## Browser

Prerequisites:

```powershell
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

Run the browser shell:

```powershell
cd rusty_box_gui
trunk serve --release --port 8080
```

Open `http://localhost:8080`. Browser builds do not use TOML, CLI host paths, native file dialogs, or `std::fs` image creation. The toolbar shows `Boot ISO` before a browser VM exists and `Console` after launch; `Reset Browser VM` clears the browser VM. The Home page offers ISO/image upload, and the Library records the uploaded media name and size. The toolbar `Library` checkbox hides the sidebar for a wider console. Browser Hardware panes are read-only because memory/device settings are fixed for the browser runtime.

## Disk Images

Desktop Images page:

- creates sparse flat hard disks and floppy images on the host filesystem,
- uses `rusty_box_bximage` for geometry and output metadata,
- supports an overwrite checkbox, disabled by default,
- attaches newly created hard disks to the selected stopped profile; running VMs must be stopped first,
- reports floppy image creation without attaching it because floppy drive emulation is not wired in the shell.

Browser Images page:

- downloads zero-filled hard disk and floppy images,
- uses the same bximage writer backend with an in-memory cursor,
- caps hard disk downloads at 64 MiB; use the desktop app for sparse large disks,
- does not attach downloaded images to the running browser VM.

Hard disk sizes accept `10M`, `512M`, `20G`, or bare numbers as MiB.

Startup disk creation is also available before native emulator startup:

```powershell
cargo run -p rusty_box_gui -- `
  --no-config `
  --display egui `
  --bios C:/path/BIOS-bochs-latest `
  --create-disk C:/tmp/c.img `
  --create-disk-size 20G `
  --boot disk
```

Equivalent TOML:

```toml
[disk]
channel = 0
drive = 0

[disk.create]
path = "C:/tmp/c.img"
size = "20G"
overwrite = false
```

## Headless smoke

```powershell
cargo run -p rusty_box_gui -- `
  --no-config `
  --display headless `
  --bios C:/path/BIOS-bochs-latest `
  --vga-bios C:/path/VGABIOS-lgpl-latest.bin `
  --cdrom C:/path/alpine-virt.iso `
  --boot cdrom `
  --memory-mib 32 `
  --ips 15000000 `
  --max-instructions 1000
```

Expected success output ends with:

```text
rusty_box_gui: executed <N> instructions
```

## Configuration file

Example native config:

```toml
[emulator]
memory_mib = 32
ips = 15000000
pci = true
sync_slowdown = false

[display]
backend = "egui"

[rom]
bios = "C:/path/BIOS-bochs-latest"
vga_bios = "C:/path/VGABIOS-lgpl-latest.bin"

[boot]
order = ["cdrom"]

[cdrom]
path = "C:/path/alpine-virt.iso"
channel = 1
drive = 0

[logging]
level = "warn"
```

CLI flags override TOML values without clearing unrelated TOML fields. For example, this changes memory while still using ROM and CD-ROM paths from the file:

```powershell
cargo run -p rusty_box_gui -- --config rusty_box.toml --memory-mib 64
```

## Resolution and validation

Merge order:

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
| `display.backend` | `egui` with default features, `terminal` with `--no-default-features` |
| `logging.level` | `warn` |
| `disk.channel`, `disk.drive` | `0`, `0` |
| `cdrom.channel`, `cdrom.drive` | `1`, `0` |

Validation highlights:

- BIOS is required for native emulator startup.
- Memory sizes and IPS must be non-zero.
- Boot order accepts `disk` and `cdrom`, supports at most three entries, and rejects duplicates.
- Disk boot requires disk media; CD-ROM boot requires CD-ROM media.
- Disk CHS accepts `CYLINDERS:HEADS:SPT` or `CYLINDERS,HEADS,SPT` on the CLI.
- If disk CHS is omitted, the runner auto-detects CHS from a non-empty 512-byte-aligned disk image.
- Disk and CD-ROM ATA slots must be in range and cannot overlap.
- `attach_disk` and `attach_cdrom` currently require UTF-8 paths because the core emulator APIs take `&str`.
- Startup-created disks are raw flat 512-byte-sector hard disks.
- Disk creation accepts human sizes like `20G` or `512M`; bare numbers mean MiB.
- `disk.create` conflicts with `disk.path` and `disk.chs` unless a CLI disk option explicitly selects another mode.
- Created-disk overwrite is opt-in through `--overwrite-created-disk` or `disk.create.overwrite = true`.
- Current GUI-created boot disks are capped by the Rusty Box CHS field at 31 GiB or smaller.

## Public API

The crate re-exports the native runner surface from `src/lib.rs` on desktop targets:

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

Browser targets expose the egui shell through `app::WebShellApp`.

## Verification

Useful checks while changing this crate:

```powershell
cargo test -p rusty_box_bximage
cargo test -p rusty_box_gui
cargo test -p rusty_box_gui --no-default-features
cargo check -p rusty_box_gui
cargo doc -p rusty_box_gui --no-deps
cargo check -p rusty_box_gui --target wasm32-unknown-unknown
```
