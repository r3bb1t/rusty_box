# rusty_box_gui

`rusty_box_gui` is the Rusty Box launcher for people who want to run a VM rather than program against the library. One crate has two front ends:

- **Desktop (native).** Typed CLI flags are merged over an optional TOML file and validated. The guest then runs on either the software interpreter or the Windows Hypervisor Platform. The default front end is a VMware-style egui shell, "Rusty Box Workstation". `terminal` and `headless` display backends are also available.
- **Browser (`wasm32`).** The same egui pages run over `eframe::WebRunner`, behind a menu bar and toolbar of their own, with no host filesystem access.

The shell borrows VMware's layout for familiarity. It is not VMware, and Rusty Box's own constraints still apply.

[docs/getting-started.md](../docs/getting-started.md) walks through the config file and the panels at more length.

## Before you start

- **ROMs.** You need a Bochs BIOS ROM, and optionally a VGA BIOS ROM. Neither is in this repository. The in-tree tools expect a Bochs source checkout at `cpp_orig/` (gitignored), which provides `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` and `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`. `.github/workflows/ci.yml` downloads the same two files from the Bochs repository. The desktop build takes any path. The browser build embeds exactly these two files at compile time.
- **Boot media.** You need an ISO for the CD-ROM or a raw flat disk image. The launcher can also create a blank disk for you (see [Disk images](#disk-images)).
- **Release builds.** A debug build of the emulator is too slow to be usable, so every command below passes `--release`.

## Desktop

```powershell
cargo run --release -p rusty_box_gui
```

With neither `--config` nor `--no-config`, the launcher loads `rusty_box.toml` from the current directory, or failing that from its parent directory. So running from `rusty_box_gui/` finds the file at the repository root.

The file is gitignored and not shipped. Write one from the [example](#configuration-file), or pass `--bios`: a BIOS path is always required. Without one the launcher exits with `BIOS path is required; pass --bios PATH or set rom.bios in TOML`.

- `--config PATH` (or `-f PATH`) loads a specific file.
- `--no-config` skips TOML entirely.

Everything can also be given as flags:

```powershell
cargo run --release -p rusty_box_gui -- `
  --no-config `
  --display egui `
  --bios C:/path/BIOS-bochs-latest `
  --vga-bios C:/path/VGABIOS-lgpl-latest.bin `
  --cdrom C:/path/alpine-virt.iso `
  --boot cdrom `
  --memory-mib 256 `
  --cpus 2 `
  --ips 15000000
```

### The shell

The egui shell runs `eframe` on the main thread and the emulator on a worker thread with a 1500 MiB stack. The window has four parts:

- **Sidebar tree.** Every VM profile is a row, and the selected one opens to its four pages: **Summary**, **Console**, **Hardware** and **Images**. The `+` button duplicates the selected profile, and a "Search VMs" field filters the list. Profiles last for the session. The first one, "Rusty Box", is built from the resolved configuration.
- **VM bar** above the page. It carries the selected VM's name, its state, and the verbs that change the state:
  - `▶ Power on` is enabled while the VM is stopped. `■ Power off` and `↻ Restart` are enabled only while it runs.
  - On the Console page only, the bar adds `Ctrl+Alt+Del`, `Capture mouse` / `Release mouse` and `Show serial` / `Hide serial`. When the window is too narrow, these fold into the `…` menu.
  - `…` always holds `About Rusty Box Workstation` and `Quit`.
  - `☰` hides the sidebar, so the Console can scale wider.
- **Status strip** along the bottom. It shows the state, the engine, the memory and CPU count, and the measured instruction rate (`--- IPS` when none is published). It adds `Restart queued` while a restart is pending.
- **Page.** Startup and runtime errors appear as notices at the top of the page.

The pages:

- **Summary** shows the state and engine and the profile's name, which you can edit here. It lists the memory, processors, boot order, CD/DVD and disk. Its tiles are `Power On VM`, `Create Disk Image` and `Hardware Settings`. `Delete Profile` is enabled only while the VM is stopped and at least one other profile remains.
- **Console** shows the guest's display, or "This VM is powered off". The serial pane shows the serial log, has an input line (`Send`, or Enter, works only while the VM runs), and a `Copy Log` button.
- **Hardware** edits the selected profile. Changes can be made only while the VM is powered off, and apply at the next power-on:

  | Pane | Settings |
  | --- | --- |
  | Memory | Guest memory, host memory, memory block size |
  | Processors | Sockets, cores per socket, threads per core (with the logical total and a warning above the SMP limit); IPS target; sync slowdown; **Engine** (Interpreter or Windows Hypervisor, see [Execution engines](#execution-engines)); max instructions (0 means unlimited) |
  | Devices | Enable PCI; boot order, which you can reorder, remove from, and add attached devices to |
  | Hard Disk | Enable, path (with a file browser), ATA channel and drive, an optional CHS override; `Create disk image` opens the Images page |
  | CD/DVD | Enable, ISO path (with a file browser), ATA channel and drive, "Boot CD/DVD first" |
  | Display | BIOS and VGA BIOS paths, log level, a pre-boot display resolution, and "Register VGA on PCI" (experimental: exposes the adapter as PCI 1234:1111 for Linux `bochs-drm`) |

  `Save settings to config file` writes the selected profile to the TOML file that was loaded. If no file was loaded, including under `--no-config`, it writes `rusty_box.toml` in the current directory. It rewrites the whole file, so comments in it are lost. It writes the CPU topology as `cpu_sockets` / `cpu_cores` / `cpu_threads` rather than `cpus`. It omits `sync_realtime`, `smp_quantum`, `max_instructions`, `cpuid_freq` and `pci_vga` when they are at their defaults. The engine and CPU-capability choices have no TOML keys, so they are not saved.
- **Images** creates disk images (see [Disk images](#disk-images)).

## Execution engines

`--engine interpreter|whp` selects what runs the guest's instructions. In the shell, the same choice is Hardware › Processors › Engine.

- **`interpreter`** (the default) is this port's own CPU. It runs everywhere.
- **`whp`** runs the guest on the Windows Hypervisor Platform. It needs a Windows build with the `hv-whp` feature, which is on by default, and a host with the platform enabled. If the platform is absent, the run is refused with `this host has no Windows Hypervisor Platform, so --engine whp cannot run`, and the error names the fix: `dism /Online /Enable-Feature /FeatureName:HypervisorPlatform`. The shell lists the "Windows Hypervisor" engine only in the builds that carry it: Windows, with `hv-whp`, without `guest-trace`.

A machine on the hypervisor differs from an interpreter machine in these ways:

- Nothing counts the guest's instructions. So any `--max-instructions` / `max_instructions` limit is refused, not approximated. The status strip shows no instruction rate, and a headless run ends with `rusty_box_gui: the guest ran on the hypervisor, which does not count instructions`.
- CPU capabilities are always `host-shared`, whatever `--cpu-capabilities` says (see the next section).
- Device timers run on host time instead of the instruction count.

The WHP path is compiled only on Windows, with `hv-whp`, and without `guest-trace`. In any other build, `--engine whp` is refused before any file is read or created, with `this build has no hypervisor engine, so --engine whp cannot run`; the error names `--engine interpreter` as the alternative.

The engine is chosen per run. It has no TOML key and is not saved.

## CPU capabilities

`--cpu-capabilities preset|host-shared` selects the processor the guest is offered:

- **`preset`** (the default) is this port's own processor model, whole. It is the same on every host, so a run is reproducible.
- **`host-shared`** narrows that model to what the host can also carry. It drops AVX-512 unless the host's `CPUID.(EAX=0DH,ECX=0)` reports the opmask and both ZMM state components. It drops AVX unless the host reports YMM state. It always drops `MONITOR`/`MWAIT`, which a hypervisor partition does not offer.

This is a CLI-only setting, with no TOML key. The WHP engine forces `host-shared`.

## Disk images

### Images page (desktop)

- Creates flat hard disks and floppy images with `rusty_box_bximage`. The images are sparse only on filesystems that leave the unwritten region unallocated, such as ext4 and APFS. On NTFS the whole image is allocated.
- The path field has a file browser, a native save dialog that suggests `c.img` or `floppy.img`. Dropping a file onto the page puts its path in the field.
- Hard-disk sizes take the bximage syntax: `10M`, `512M`, `20G`, or a bare number, which means MiB. The size must be at least 10 MiB, and it is rounded down to whole 16-head, 63-sector cylinders. Fewer than 2^24 cylinders are allowed, so 8064 GiB or more is refused.
- Floppies come in the ten bximage formats, from 160 KB to 2.88 MB.
- "Overwrite existing file" is off by default. Without it, an existing file is refused.
- A new hard disk is attached to the selected profile if that VM is stopped. If it is running, you are told to stop it first. A floppy is created but not attached: the shell has no floppy drive to attach it to.

### Images page (browser)

- Downloads zero-filled hard disk and floppy images, built in memory with the bximage writer functions.
- Hard disks are capped at 64 MiB (final image size). Use the desktop build for anything larger.
- Downloaded images are not attached to the browser VM.

### Creating a disk at startup

The launcher can provision a blank boot disk before the emulator starts:

```powershell
cargo run --release -p rusty_box_gui -- `
  --no-config `
  --display egui `
  --bios C:/path/BIOS-bochs-latest `
  --create-disk C:/tmp/c.img `
  --create-disk-size 20G `
  --boot disk
```

The equivalent TOML:

```toml
[disk]
channel = 0
drive = 0

[disk.create]
path = "C:/tmp/c.img"
size = "20G"
overwrite = false
```

What provisioning does depends on `overwrite`:

- **Without `overwrite`** (the default), provisioning is idempotent. If no file is at the path, a raw flat disk with 512-byte sectors is created there, 20G if no size is given. An existing file that is non-empty and a multiple of 512 bytes is reused untouched. Anything else fails startup.
- **With `overwrite`** (`--overwrite-created-disk` or `disk.create.overwrite = true`), the file is recreated empty at every launch, whatever it holds. In the egui shell that happens at the session's first power-on, and again at the next power-on if that run ended in an error.

The disk geometry comes from `rusty_box_bximage`: 16 heads, 63 sectors per track.

## Browser

Prerequisites:

```powershell
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

The ROMs must be at the `cpp_orig/bochs/bochs/bios/` paths listed in [Before you start](#before-you-start), because the browser build embeds them. The wasm linker's 8 MiB stack comes from `[target.wasm32-unknown-unknown]` in `.cargo/config.toml`.

Run the browser shell:

```powershell
cd rusty_box_gui
trunk serve --release --port 8080
```

Then open `http://localhost:8080`. `trunk build --release` writes the site to `rusty_box_gui/dist/`, which is gitignored.

Browser builds use no TOML, no CLI flags, no native file dialogs, and no host filesystem. What the browser shell offers:

- **Menu bar.** `File` (`Boot OS Image`, `Create Disk Image`), `Edit` (`Clear Library Search`), `VM` (`Reset Browser VM`), `Help` (`About Rusty Box Workstation`), then the Home, Console, Hardware and Images pages.
- **Toolbar.** `▶ Boot OS Image` before a browser VM exists, and `Console` after launch. `↻ Reset Browser VM` clears the browser VM. `▣ Hardware` and `+ New Image` jump to their pages. The `Library` and `Serial` checkboxes show or hide the library sidebar and the serial pane.
- **Home.** `Boot OS Image` opens a file picker for `.iso` or `.img` files and attaches the chosen file as a bootable CD/DVD. The "Boot DLX sample" tile is disabled, because this build does not bundle DLX. `Create Disk Image` opens the Images page.
- **Hardware.** Memory (1 to 4096 MB, default 128) and the processor count (default 1) can be changed before boot; reset the browser VM to change them again. Devices, Hard Disk, CD/DVD and Display are read-only information. CD/DVD shows the uploaded file's name and size.

## Headless run

```powershell
cargo run --release -p rusty_box_gui -- `
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

On the interpreter, a successful run ends with:

```text
rusty_box_gui: executed <N> instructions
```

On `--engine whp`, drop `--max-instructions`, which that engine refuses. The run then ends with `rusty_box_gui: the guest ran on the hypervisor, which does not count instructions`.

## Configuration file

An example native config:

```toml
[emulator]
memory_mib = 32
cpus = 2
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

CLI flags override TOML values without clearing unrelated TOML fields. For example, this changes the memory size and still uses the ROM and CD-ROM paths from the file:

```powershell
cargo run --release -p rusty_box_gui -- --config rusty_box.toml --memory-mib 64
```

Unknown keys are rejected. Relative paths in the TOML file resolve against the file's own directory, while paths given on the command line resolve against the current directory.

Every key, with its default and matching flag:

| Key | Meaning | Default | CLI flag |
| --- | --- | --- | --- |
| `emulator.memory_mib` | Guest memory, MiB | `32` | `--memory-mib` |
| `emulator.host_memory_mib` | Host RAM backing guest memory, MiB; a larger guest gets an overflow file for the rest | `memory_mib` | `--host-memory-mib` |
| `emulator.memory_block_kib` | Memory allocation block size, KiB | `128` | `--memory-block-kib` |
| `emulator.cpus` | Processor count, as N sockets × 1 core × 1 thread | `1` | `--cpus` |
| `emulator.cpu_sockets`, `cpu_cores`, `cpu_threads` | Topology. Any one present overrides `cpus`, and missing ones count as 1 | unset | `--cpu-sockets`, `--cpu-cores`, `--cpu-threads` |
| `emulator.ips` | Instructions-per-second target | `4000000` | `--ips` |
| `emulator.pci` | PCI bus | `true` | `--pci` / `--no-pci` |
| `emulator.sync_slowdown` | Sync-slowdown pacing | `false` | `--sync-slowdown` / `--no-sync-slowdown` |
| `emulator.sync_realtime` | Advance PIT/ACPI timers on wall-clock time (Bochs `clock: sync=realtime`) | `false` | `--sync-realtime` |
| `emulator.smp_quantum` | SMP scheduling quantum, 1-32 (Bochs `cpu: quantum=`) | `16` | `--smp-quantum` |
| `emulator.max_instructions` | Stop after this many instructions | unlimited | `--max-instructions` |
| `emulator.cpuid_freq` | How CPUID leaves 0x15/0x16 report frequency: `"hardware"`, `"none"` or `"ips"` (Bochs `cpu: cpuid_freq=`) | `"none"` | `--cpuid-freq` |
| `display.backend` | `"egui"`, `"terminal"` or `"headless"` | `egui` (`terminal` without the `gui-egui` feature) | `--display` |
| `display.width`, `display.height` | Pre-boot VBE mode. Both are needed and must be non-zero; this raises the VBE ceiling so the guest can select the mode | unset | none |
| `display.bpp` | Bits per pixel of that mode (8/16/24/32) | `32` | none |
| `display.pci_vga` | Register the VGA on PCI for Linux `bochs-drm` (experimental) | `false` | none |
| `rom.bios` | System BIOS ROM (required) | none | `--bios` |
| `rom.vga_bios` | VGA BIOS ROM | none | `--vga-bios` |
| `boot.order` | One or both of `"disk"`, `"cdrom"`, in order, with no repeats | inferred, see below | `--boot` |
| `disk.path` | Existing raw disk image | none | `--disk` |
| `disk.chs` | `{ cylinders = …, heads = …, sectors_per_track = … }` | auto-detected | `--disk-chs` |
| `disk.channel`, `disk.drive` | ATA slot of the disk | `0`, `0` | none |
| `disk.create.path` | Disk to provision at startup | none | `--create-disk` |
| `disk.create.size` | Its size, in bximage syntax | `"20G"` | `--create-disk-size` |
| `disk.create.overwrite` | Recreate the file empty at every launch, discarding what it holds | `false` | `--overwrite-created-disk` |
| `cdrom.path` | ISO image | none | `--cdrom` |
| `cdrom.channel`, `cdrom.drive` | ATA slot of the CD-ROM | `1`, `0` | none |
| `logging.level` | `trace`, `debug`, `info`, `warn` or `error` | `warn` | `--log-level` |

Release builds compile out `debug` and `trace` output: the workspace builds `tracing` with `release_max_level_info`.

[docs/getting-started.md](../docs/getting-started.md) explains the pacing settings (IPS, sync slowdown, max instructions).

## Command-line flags

`rusty_box_gui --help` lists every flag, and `--version` prints the version. The flags that have no TOML key:

| Flag | Meaning |
| --- | --- |
| `-f`, `--config PATH` | Load this TOML file (conflicts with `--no-config`) |
| `--no-config` | Load no TOML file |
| `--engine interpreter\|whp` | Execution engine (default `interpreter`) |
| `--cpu-capabilities preset\|host-shared` | Processor offered to the guest (default `preset`) |

All other flags mirror a TOML key in the table above. Some rules for combining them:

- `--boot` takes one or both devices, comma-separated or repeated (`--boot cdrom,disk`).
- `--disk-chs` takes `CYLINDERS:HEADS:SPT` or `CYLINDERS,HEADS,SPT`.
- `--cpu-sockets`, `--cpu-cores` and `--cpu-threads` conflict with `--cpus`.
- `--create-disk`, `--create-disk-size` and `--overwrite-created-disk` conflict with `--disk` and `--disk-chs`.
- `--pci` conflicts with `--no-pci`, and `--sync-slowdown` with `--no-sync-slowdown`.
- `--sync-realtime` can only turn realtime sync on.
- `--display egui` exists only in builds with the `gui-egui` feature.

## Resolution and validation

Settings are merged in this order, later winning:

1. Built-in defaults.
2. The TOML file.
3. Explicit CLI flags.

When no boot order is given, it is inferred. It is `disk` if a disk is configured, otherwise `cdrom` if a CD-ROM is. If neither is configured, the order is empty. An empty order is allowed only for the egui display, where media can be attached from the Hardware page; with any other display it is an error.

Validation rules:

- A BIOS is required.
- `memory_mib`, `host_memory_mib`, `memory_block_kib` and `ips` must be non-zero.
- `smp_quantum` must be 1-32, and `cpuid_freq` must be one of its three values.
- The CPU topology is checked against the emulator's per-component and SMP limits.
- A boot order holds at most three entries, with no duplicates. Disk boot needs a disk, and CD-ROM boot needs a CD-ROM.
- Disk CHS values must be non-zero. When CHS is omitted, it is auto-detected from the image: the image must be non-empty and a multiple of 512 bytes. Detection assumes 16 heads and 63 sectors per track, rounds the cylinder count up, and requires fewer than 2^24 cylinders.
- The disk and CD-ROM must each name a valid ATA slot (channel 0 or 1, drive 0 or 1), and they cannot share one.
- A VGA BIOS file must be a non-zero multiple of 512 bytes.
- Disk and CD-ROM paths must be valid UTF-8, because `MachineBuilder::disk_file` / `cdrom_file` take `&str`. A non-UTF-8 path is rejected with `path must be valid UTF-8 for current emulator media API`.
- `disk.create` cannot be combined with `disk.path` or `disk.chs` in the TOML file. A CLI disk option picks one mode explicitly: `--disk` wins over a TOML `disk.create`, and `--create-disk` wins over a TOML `disk.path`. A creation size or overwrite flag with no creation path is an error.
- A created disk must be at least 10 MiB and have fewer than 2^24 cylinders, so under 8064 GiB. It uses the same geometry rules as the Images page.

## Cargo features

| Feature | Default | Effect |
| --- | --- | --- |
| `gui-egui` | yes | The egui desktop shell and browser build. Without it (`--no-default-features`), the native binary is a CLI runner whose default display is `terminal`. |
| `hv-whp` | yes | The Windows Hypervisor Platform engine. Its dependency is declared only for Windows targets, so on other targets the feature adds nothing. |
| `guest-trace` | no | Diagnostic build that records guest process starts and exits, stderr writes, mounts, signals and CPU exceptions to `guest_trace.log` (override the path with `RUSTY_BOX_GUEST_TRACE_LOG`). Single-CPU configurations only; it slows emulation and compiles out the WHP path. |
| `windows-gui-subsystem` | no | On Windows, builds the binary without a console window. `.github/workflows/gui-artifacts.yml` uses it when it builds the GUI artifacts. |

## Public API

`src/lib.rs` re-exports these on every target:

- `Args`
- `BootDevice`
- `DiskGeometry`
- `DisplayBackend`
- `FileConfig`
- `ResolvedConfig`
- `RunError`

On native targets it also re-exports `run`, `run_resolved` and `RunSummary`.

The `args`, `config` and `error` modules are public, as are `runner` (native only) and `app` (with `gui-egui`). `config::Engine` and `config::CpuCapabilities` are reachable through `config`. Browser targets start the shell through `app::WebShellApp`.

## Automation

`eframe` is built with its `inspection` feature. Setting `EGUI_INSPECTION=1` when launching the desktop shell exposes its UI to egui inspection clients such as `egui-mcp`, on `127.0.0.1:5719` by default. Setting it to a `host:port` value chooses the address instead.

## Verification

Checks to run while changing this crate:

```powershell
cargo test --release -p rusty_box_gui
cargo test --release -p rusty_box_bximage
cargo check --release -p rusty_box_gui --all-targets
cargo check --release -p rusty_box_gui --no-default-features
cargo check --release -p rusty_box_gui --target wasm32-unknown-unknown
```

The wasm check needs the ROMs at the `cpp_orig/` paths above. Before every commit, run the whole gate, `cargo xtask ci`. It compiles this crate's tests and its wasm build, but does not run the tests (see `xtask/README.md`).
