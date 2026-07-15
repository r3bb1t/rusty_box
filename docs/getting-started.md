# Getting Started with Rusty Box

This guide walks through setting up the GUI launcher (`rusty_box_gui`), writing a
config file, booting a real OS from an ISO, and understanding the settings whose
meaning is not obvious — pacing, CPU topology, disk provisioning, and display
options. It reflects how the launcher actually behaves; every knob documented
here exists in the code.

## What you need

- **Rust toolchain** (stable) — build with `cargo`.
- **The repository** — the BIOS and VGA BIOS ROMs ship in-repo under
  `cpp_orig/bochs/bochs/bios/`, so there is nothing extra to download for
  firmware.
- **A guest image** — a bootable ISO (e.g. an Ubuntu Server installer) and/or a
  hard-disk image. The launcher can create a blank disk image for you.

## Quick start

From the repository root:

```bash
cargo run --release -p rusty_box_gui
```

The launcher reads `rusty_box.toml` from the current directory (override with
`--config <path>`, or skip the file entirely with `--no-config`). A minimal
config that boots an installer ISO and installs to a fresh 12 GiB disk:

```toml
[emulator]
memory_mib = 2048
ips = 120000000
pci = true

[rom]
bios = "cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"
vga_bios = "cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"

[boot]
order = ["disk", "cdrom"]

[disk.create]
path = "my-disk.img"
size = "12G"
overwrite = false

[cdrom]
path = "my-installer.iso"
```

Press **Power On** in the GUI. On the first boot the disk is blank (no boot
signature), so the BIOS falls through to the CD and starts the installer; after
the install writes a bootloader to the disk, the *same* boot order boots the
installed system instead of looping the installer. That is why
`order = ["disk", "cdrom"]` is the right default for an install workflow.

## The config file, section by section

All sections and keys are optional; unknown keys are rejected (typos fail
loudly rather than being silently ignored).

### `[emulator]`

| Key | Meaning |
|-----|---------|
| `memory_mib` | Guest RAM in MiB. 2048 gives an installer (squashfs unpack + apt) enough headroom; 1024 runs a live/installer session but is tight for the actual install phase. |
| `host_memory_mib` | Host-side allocation cap for guest RAM. Usually set equal to `memory_mib`. |
| `memory_block_kib` | Memory block granularity. 128 is a good default. |
| `cpus` | Flat logical CPU count (legacy form; equals `cpus × 1 × 1` topology). |
| `cpu_sockets` / `cpu_cores` / `cpu_threads` | Explicit topology (see [CPU topology](#cpu-topology)). Mutually exclusive with `cpus`. |
| `ips` | Guest-clock calibration — **not a speed limit**. See [Pacing](#pacing-ips-sync-slowdown-max-instructions). |
| `pci` | Enable the PCI bus (i440FX/PIIX3). Required for IDE bus-master DMA (fast disk/CD I/O) and for `pci_vga`. Leave `true`. |
| `sync_slowdown` | Throttle emulation so guest time ≈ real time. See [Pacing](#pacing-ips-sync-slowdown-max-instructions). |
| `max_instructions` | `0` = run forever. Any other value stops the VM after exactly that many instructions — useful for benchmarks/CI, **looks like a freeze** if you set it by accident in interactive use. |

### `[display]`

| Key | Meaning |
|-----|---------|
| `backend` | `"egui"` (windowed GUI), `"terminal"` (text-mode in your terminal), or `"headless"` (no display — benchmarks, CI). |
| `width` / `height` / `bpp` | Pre-boot VBE preferred mode. Raises the display capability ceiling so the guest *may* select this resolution (GRUB `gfxpayload`, vesafb, KMS). It does not force the mode — the guest decides. `bpp` is 8/16/24/32. |
| `pci_vga` | **Experimental.** Registers the VGA adapter as a PCI device (`1234:1111`), which lets Linux's `bochs-drm` driver bind and switch the console to a KMS framebuffer at the preferred resolution. Off by default. Caveat: when `bochs-drm` takes over, the text console goes dark until the framebuffer console comes up — during a slow boot phase this can look like a hang. Confirm your config boots with it off before turning it on. |

### `[rom]`

`bios` and `vga_bios` point at the Bochs firmware images. The in-repo paths
(`cpp_orig/bochs/bochs/bios/...`) are the ones to use; there is no reason to
change these unless you are experimenting with custom firmware.

### `[boot]`

`order` is a list of up to three entries from `"disk"` and `"cdrom"`, tried in
order. See the quick-start note on why `["disk", "cdrom"]` handles the whole
install-then-boot lifecycle without edits.

### `[disk]` — attach an existing image

```toml
[disk]
path = "my-disk.img"
channel = 0          # ATA channel 0 or 1
drive = 0            # master (0) or slave (1)
chs = { cylinders = 1024, heads = 16, sectors_per_track = 63 }  # optional override
```

Disk geometry is auto-detected from the file size when `chs` is omitted; only
override it for images with unusual layout expectations.

### `[disk.create]` — auto-provision a blank disk

```toml
[disk.create]
path = "my-disk.img"
size = "12G"
overwrite = false
```

Creates the image on first launch. Two things people trip over:

- The image is a **pre-allocated flat file**: a `12G` image consumes 12 GiB of
  host disk immediately, and the allocation can take a while — the launcher
  shows a notice, but the window may look idle until it finishes.
- `overwrite = false` (the default) means an existing, valid image is **reused,
  never recreated** — your installed OS survives relaunches. Set
  `overwrite = true` only when you deliberately want a factory-reset disk on
  every launch.

### `[cdrom]`

```toml
[cdrom]
path = "ubuntu-26.04-live-server-amd64.iso"
channel = 1
drive = 0
```

Put the CD on channel 1 when the disk is on channel 0 — each ATA
channel/drive slot can hold only one device, and keeping them on separate
channels matches the classic PC layout the guest expects.

### `[logging]`

`level` is one of `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`.
`"warn"` is the quiet default choice; `"info"` surfaces one-shot device
lifecycle messages (PCI BAR assignments, VGA mode changes).

**Caveat:** release builds compile out `debug` and `trace` statements entirely
(`release_max_level_info`), so setting `level = "debug"` on a release binary
shows nothing extra. Debug-level logging requires a debug build, which is far
slower — prefer `info` and the one-shot messages.

## The GUI panels

The Hardware pages edit the same settings as the TOML, per-session:

- **Processors** — topology, IPS target, sync slowdown, max instructions
  (details below). Editable only while powered off.
- **Display** — backend resolution picker and the "Register VGA on PCI"
  checkbox (`pci_vga`).
- **Disks / Images** — attach or create disk images, CD selection, CHS
  override, boot order.

**"Save settings to config file"** writes the current settings back to
`rusty_box.toml`. Warning: it serializes a fresh file — **hand-written comments
in your TOML are lost**. If you maintain a commented config, edit it by hand
and treat Save as a tool for throwaway setups.

## Pacing: IPS, sync slowdown, max instructions

This is the least obvious part of the whole setup.

**`ips` calibrates the guest clock; it does not limit speed.** The emulator
declares "one virtual second = `ips` instructions." The guest's sense of time
then runs at `real_throughput ÷ ips` of real time:

- `ips` **at or slightly above** your machine's real sustained throughput →
  the guest clock runs at (or slightly slower than) real time. Correct.
- `ips` **too low** → the guest clock runs *faster* than real time. Timers and
  timeouts in the guest expire early: systemd's 90-second unit timeouts and
  D-Bus's 25-second call timeouts fire spuriously and services "fail" during
  boot for no visible reason. If you see a cascade of red `FAILED` units,
  raise `ips`.
- `ips` **absurdly high** → guest-side sleeps cost proportionally more real
  time; an idle guest feels sluggish.

To pick a value: watch the IPS readout in the status bar during a CPU-heavy
phase (e.g. kernel boot) and set `ips` a little above the peak you observe.
On a machine that peaks around 106M, `120000000` is right.

**`sync_slowdown`** makes the emulator *sleep* to pin guest time to real time
instead of running as fast as it can. Turn it on for interactive sessions
where wall-clock-accurate timing matters (cursor blink rates, media, games).
Leave it off for installs and boots — you want those as fast as possible.

**`max_instructions`** = 0 for normal use. A non-zero value silently stops the
VM when the budget runs out — that is a benchmarking/CI feature.

## CPU topology

`sockets × cores × threads` is the hardware hierarchy the guest sees via CPUID
and the MP/ACPI tables — exactly like real hardware: sockets are physical CPU
packages (a 2-socket config is a dual-CPU server board), cores are independent
execution units per package, threads are SMT/hyper-threading per core.

Two practical notes:

- The emulator interleaves all logical CPUs on **one host thread**. More vCPUs
  gives zero extra speed — it splits the same instruction budget and adds
  scheduling overhead. The split only changes what the guest *believes*, which
  matters to its scheduler (thread/core/socket affinity, NUMA assumptions) and
  to per-socket software licensing.
- Multi-CPU guest support is still being stabilized. Until then, `1 × 1 × 1`
  is the recommended configuration; choose more only to exercise the SMP code
  paths themselves.

## Input: keyboard and mouse

- **Keyboard** goes to the guest whenever the VM is running and the display
  has focus (while the mouse is captured, *all* keys go to the guest,
  including chords like Ctrl+C).
- **Mouse**: click the display to capture it — after that, relative motion,
  buttons, and the wheel are forwarded to the guest's PS/2 mouse. Release
  capture from the toolbar toggle.
- **Ctrl+Alt+Del** has a dedicated menu action (the host OS would otherwise
  intercept it).

## Performance expectations

- Expect on the order of ~100M instructions/second on a fast desktop core.
- Disk and CD I/O go through IDE bus-master DMA (requires `pci = true`).
  If a Linux guest prints `BMDMA: BAR4 is zero, falling back to PIO` in dmesg,
  PCI is disabled in your config and all I/O is running an order of magnitude
  slower than it should.
- Some boot phases are CPU-bound userspace work (e.g. an installer generating
  its APT cache, initramfs unpacking) — the screen sits on one line for a
  minute or two while the IPS readout stays high. That is progress, not a
  hang.

## Troubleshooting

**"It looks frozen."** Check, in order:
1. Is the **IPS readout** in the status bar still high and changing? Then the
   guest is running — likely a CPU-bound phase. Give it a minute.
2. Did you set **`max_instructions`** to something non-zero? The VM stops
   silently when the budget is exhausted.
3. Is **`pci_vga = true`**? The console goes dark between `bochs-drm` binding
   and the framebuffer console coming up. Retest with it off to compare.

**Services fail during boot with timeouts** (`FAILED` cascade, D-Bus errors):
`ips` is set below your machine's real throughput, so guest time runs fast and
timeouts expire early. Raise `ips` above the observed IPS peak.

**Disk image errors at startup**: an existing file at the `[disk.create]`
path that is not a valid image (empty or not sector-aligned) is rejected
rather than silently overwritten. Delete or move it, or point `path`
elsewhere.

**Rebuild fails with "access denied" on the executable** (Windows): a running
VM has the `.exe` locked. Power off / close the launcher before
`cargo build`.

**Typo in the config**: unknown keys fail the launch with a parse error
naming the key — check the spelling against the tables above.

## Command-line flags

Everything in the TOML has a CLI counterpart that overrides it, useful for
one-off runs without editing the file:

```bash
# Headless benchmark run, capped at 15 billion instructions
cargo run --release -p rusty_box_gui -- \
  --config rusty_box.toml --display headless --max-instructions 15000000000

# Terminal-mode boot of a different ISO
cargo run --release -p rusty_box_gui -- --cdrom other.iso --display terminal

# Ignore the config file entirely
cargo run --release -p rusty_box_gui -- --no-config --bios <path> ...
```

Key flags: `--config`, `--no-config`, `--display`, `--boot disk,cdrom`,
`--disk` / `--disk-chs`, `--create-disk` / `--create-disk-size` /
`--overwrite-created-disk`, `--cdrom`, `--memory-mib`, `--ips`,
`--max-instructions`, `--cpus` or `--cpu-sockets`/`--cpu-cores`/`--cpu-threads`,
`--pci` / `--no-pci`, `--sync-slowdown` / `--no-sync-slowdown`, `--log-level`.
