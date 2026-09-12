# Getting Started with Rusty Box

This guide covers the `rusty_box_gui` launcher: where its firmware comes from,
how to write a config file, how to boot an OS from an ISO, how the window is
organised, and the settings whose meaning is not obvious (pacing, CPU
topology, disk provisioning, display options, and the choice of engine). Every
flag, key and control named here exists in `rusty_box_gui/src/`.

## What you need

- **A Rust toolchain** (stable). Build with `cargo`, in release mode.
- **The Bochs firmware ROMs.** They are **not** in this repository: the
  launcher reads a system BIOS and a VGA BIOS from files you name. See
  [Getting the firmware](#getting-the-firmware).
- **A guest image**, meaning a bootable ISO (an installer, say) and/or a
  hard-disk image. The launcher can create a blank disk image for you.

### Getting the firmware

Both ROMs come from the Bochs source tree,
<https://github.com/bochs-emu/Bochs>:

| ROM | Path in the Bochs repository |
|---|---|
| System BIOS | `bochs/bios/BIOS-bochs-latest` |
| VGA BIOS | `bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` |

The simplest way to get them is to clone Bochs into `cpp_orig/bochs` at the
root of this repository, which is where the examples in this guide, and the
repository's other tools, expect it to be:

```bash
git clone https://github.com/bochs-emu/Bochs cpp_orig/bochs
```

That clone gives you `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` and
`cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`. `/cpp_orig`
is in `.gitignore`, so the clone never shows up as a change to this
repository. Any other copy of the two files works just as well.

**The launcher does not search for them.** You name them yourself, with
`--bios` / `--vga-bios` on the command line, with `bios` / `vga_bios` in the
`[rom]` section of the config file, or on the shell's Hardware › Display
pane:

- A system BIOS is required. Without one, a machine cannot run: a launch
  fails with `BIOS path is required; pass --bios PATH, set rom.bios in the
  VM file, or set the BIOS path under Hardware › Display`. In the shell, a
  VM with no BIOS path, or with no hard disk or CD/DVD attached, can still
  be edited and kept in the library; its power-on is refused with a notice
  naming what is missing, such as `Set a BIOS path under Hardware › Display
  before powering on.` or `Attach a hard disk or CD/DVD before powering
  on.`
- The VGA BIOS is optional as far as the launcher is concerned, but if you
  leave it out the machine is built with no VGA BIOS ROM. A VGA BIOS file
  must be a non-zero multiple of 512 bytes.
- A relative path in the config file is resolved against the config file's
  own directory. A relative path on the command line is resolved against the
  current directory.

The browser (wasm) build of `rusty_box_gui` works differently: it embeds both
ROMs at compile time from exactly the two `cpp_orig/bochs/...` paths above,
so building for the browser needs the clone in place.

## Quick start

From the repository root:

```bash
cargo run --release -p rusty_box_gui -- --config rusty_box.toml
```

The shell keeps its VMs in a library folder (`%APPDATA%\rusty_box\vms` on
Windows; see [rusty_box_gui/README.md](../rusty_box_gui/README.md#desktop)
for the other platforms) and lists every VM in it at every launch. To start
from a file, pass it with `--config <path>` (or `-f <path>`): it opens as a
temporary VM, marked "(unsaved)", and **Keep in library** on its Summary page
adds it to the library for good. A `rusty_box.toml` in the current or parent
directory is not read, so a file someone drops there cannot change what
boots; `--no-config` is accepted and changes nothing. Here is a minimal
config that boots an installer ISO and installs to a fresh 12 GiB disk; save
it as `rusty_box.toml`, which is gitignored and does not ship with the
repository:

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

The window opens with the VM stopped, on the VM's **Summary** page. To start
it, press **▶ Power on** in the bar at the top, or the **Power On VM** tile on
the Summary page. The shell then switches to the **Console** page.

On the first boot the disk is blank (it has no boot signature), so the BIOS
falls through to the CD and starts the installer. Once the installer has
written a bootloader to the disk, the *same* boot order boots the installed
system instead of looping back into the installer. That is why
`order = ["disk", "cdrom"]` is the right choice for an install workflow.

## The window

The desktop shell has four parts:

- the **VM bar** across the top;
- a **sidebar tree** on the left;
- the **page** for whatever is selected in the tree;
- a **status strip** along the bottom.

### Sidebar tree

The sidebar, headed *My computer*, lists the VMs in the library and has a
search box that filters them. The selected VM opens into its four pages:
**Summary**, **Console**, **Hardware** and **Images**. Clicking a page moves
to it. Clicking another VM switches to it and lands on its Summary page, but
only once the running VM has been stopped; the VM being left is saved first.

Every VM is a complete launch configuration, kept in its own file in the
library. The VM the command line described, if any, comes first, marked
"(unsaved)"; a library VM whose last save failed is marked "(save failed)".
The sidebar's `+` button adds a new VM to the library, copied from the
selected one. Library files that do not load are listed under **Could not
load**, each with its error as a tooltip and a Delete button. The **☰**
button at the left of the VM bar shows or hides the sidebar.

### VM bar

The bar shows the selected VM's name and a state badge:

- **Stopped**
- **Starting**
- **Running**
- **Faulted**, shown while an error notice is on screen

It also carries the verbs that change that state:

- **▶ Power on** is enabled when the VM is stopped and not already starting.
- **■ Power off** and **↻ Restart** are enabled while the VM is running.
- **…** opens a menu with *About Rusty Box Workstation* and *Quit*.

While the Console page is shown, the bar adds three controls for the guest's
console:

- **Ctrl+Alt+Del** sends that chord to the guest (the host OS would otherwise
  intercept it).
- **Capture mouse** / **Release mouse** toggles mouse capture.
- **Show serial** / **Hide serial** toggles the serial pane.

When the bar is too narrow for all three, they move into the **…** menu.

### Summary page

The Summary page shows:

- the state badge and the engine;
- the VM's name, which you can edit in place;
- its memory, processors, boot order, CD/DVD and disk;
- a caption on where the VM is kept: `Saved automatically to <path>`,
  `Saving to <path> when the edit ends`, or
  `Not saved: the last write to <path> failed. It is tried again at the next change, selection or power-on.`
  A temporary VM's caption reads
  `Temporary VM. Not saved until it is kept in the library.`

A library VM has a **Delete VM** button; a temporary VM has **Keep in
library** and **Discard**. `Delete VM` and `Discard` ask first
(`Delete <name>?` / `Discard <name>?`) and work only while the VM is stopped
and another VM remains; deleting removes the VM's file, and the disk images
it uses are kept. Three tiles lead
elsewhere: **Power On VM**, **Create Disk Image** (goes to the Images page)
and **Hardware Settings** (goes to the Hardware page).

### Console page

The Console page shows the guest's display, with the serial pane beside it
when that is shown. While the VM is off, the page reads *This VM is powered
off*.

### Hardware page

The Hardware page lists six devices. Selecting one opens its settings:

| Device | Settings |
|---|---|
| **Memory** | Guest memory, host memory, memory block size |
| **Processors** | Sockets, cores per socket, threads per core, IPS target, *Sync slowdown*, **Engine** (*Interpreter*, or *Windows Hypervisor* in a Windows build that carries it), max instructions (0 means unlimited) |
| **Devices** | *Enable PCI*; the boot order (move earlier or later, remove, add an attached device) |
| **Hard Disk** | Enable, disk path with *Browse…*, ATA channel and drive, *Override CHS geometry* |
| **CD/DVD** | Enable, ISO path with *Browse…*, ATA channel and drive, *Boot CD/DVD first* |
| **Display** | BIOS path, VGA BIOS path, log level, display resolution, *Register VGA on PCI* |

Settings can be edited only while the VM is powered off, and they take effect
at the next power-on.

An edit is saved to the VM's library file when it ends — when the field
loses focus or the drag ends — and at once when you switch VMs, add a VM,
keep one, power on, or the window goes behind another or closes. The file is
rewritten each time, so **hand-written comments in it are lost**. A temporary
VM (one started with `--config` or flags) is saved only once you keep it in
the library; until then the page reads *Not saved. Keep this VM in the
library from its Summary page.*

The config file's `emulator.engine` key chooses the engine (see
[Engines](#engines)); `--engine` on the command line overrides it.

### Images page

The Images page creates blank images with the `rusty_box_bximage` backend:

- a flat **hard disk**, at a size such as `512M` or `20G` (a bare number
  means MiB);
- or a **floppy** in one of the standard formats.

You give a path, choose whether to overwrite an existing file, and press
**Create image**.

A new hard disk is attached to the selected VM at once, unless the VM is
running, in which case you are asked to stop it first. The floppy drive is not
wired up yet, so a created floppy image is only written to disk.

### Status strip

The status strip shows:

- the state;
- the engine;
- memory and CPU count;
- the measured instruction rate, which reads `--- IPS` when there is none;
- *Restart queued* while a restart is pending.

## The config file, section by section

Every section and key is optional, except that a system BIOS must come from
`[rom]` or from `--bios`. Unknown keys are rejected, so a typo fails loudly
instead of being silently ignored. A command-line flag overrides the key it
corresponds to.

### `[vm]`

`name` is what the shell shows for the VM. A library file without one is
shown under its file name. The shell writes the key whenever it saves the
VM, and leaves it out when the name is blank.

### `[emulator]`

| Key | Default | Meaning |
|-----|---------|---------|
| `engine` | `"interpreter"` | Which engine runs the guest: `"interpreter"` or `"whp"`. See [Engines](#engines). |
| `cpu_capabilities` | `"preset"` | Which processor the guest is offered: `"preset"` or `"host-shared"`. See [CPU capabilities](#cpu-capabilities). |
| `memory_mib` | `32` | Guest RAM in MiB. An installer (squashfs unpack plus apt) has comfortable headroom at 2048; 1024 runs a live or installer session but is tight for the install phase itself. |
| `host_memory_mib` | `memory_mib` | Host memory backing guest RAM. A value below `memory_mib` keeps only that much resident; the rest lives in an overflow file and is swapped in on demand, which is slower (Bochs `memory: host=`). |
| `memory_block_kib` | `128` | Memory block granularity. |
| `cpus` | `1` | Flat logical CPU count, taken as `cpus × 1 × 1`. |
| `cpu_sockets` / `cpu_cores` / `cpu_threads` | `1` each | Explicit topology (see [CPU topology](#cpu-topology)). If any of the three is present, `cpus` is ignored. |
| `ips` | `4000000` | Guest-clock calibration. **Not a speed limit.** See [Pacing](#pacing-ips-sync-slowdown-max-instructions). |
| `pci` | `true` | The PCI bus (i440FX/PIIX3). Required for IDE bus-master DMA (fast disk and CD I/O) and for `pci_vga`. Leave it `true`. |
| `sync_slowdown` | `false` | Throttle emulation so guest time roughly matches real time. See [Pacing](#pacing-ips-sync-slowdown-max-instructions). |
| `sync_realtime` | `false` | Advance the PIT and ACPI timers on wall-clock time (Bochs `clock: sync=realtime`). When off, timers follow emulated time. |
| `smp_quantum` | `16` | Instructions each CPU runs before the next one gets its turn (Bochs `cpu: quantum=`). Must be 1–32. |
| `cpuid_freq` | `"none"` | How CPUID frequency leaves `0x15`/`0x16` are reported (Bochs `cpu: cpuid_freq=`): `"none"`, `"hardware"` or `"ips"`. |
| `max_instructions` | no limit | Stops the VM after exactly that many instructions. Useful for benchmarks and CI; **looks like a freeze** if you set it by accident. Leave the key out for no limit. |

`max_instructions` needs care. A value in the file or on the command line is
taken literally, so `0` runs nothing at all. Only the GUI's *Max instructions*
field reads 0 as "unlimited". A configuration the shell writes carries the key
only when a limit is set.

### `[display]`

| Key | Meaning |
|-----|---------|
| `backend` | `"egui"` (the windowed shell; the default in a build with the `gui-egui` feature, which is on by default), `"terminal"` (text mode in your terminal), or `"headless"` (no display, for benchmarks and CI). |
| `width` / `height` / `bpp` | Preferred pre-boot VBE mode. It raises the display capability ceiling so the guest *may* select this resolution (GRUB `gfxpayload`, vesafb, KMS). It does not force the mode; the guest decides. Takes effect only when both `width` and `height` are given. `bpp` is 8, 16, 24 or 32, and defaults to 32. |
| `pci_vga` | **Experimental.** Off by default. Registers the VGA adapter as a PCI device (`1234:1111`), which lets Linux's `bochs-drm` driver bind and switch the console to a KMS framebuffer at the preferred resolution. Caveat: when `bochs-drm` takes over, the text console goes dark until the framebuffer console comes up, and during a slow boot phase that can look like a hang. Confirm your config boots with it off before turning it on. |

### `[rom]`

`bios` (required unless `--bios` is given) and `vga_bios` (optional) point at
the firmware images. See [Getting the firmware](#getting-the-firmware).

### `[boot]`

`order` is a list containing `"disk"` and/or `"cdrom"`, each at most once. The
BIOS tries them in order, and the first one that boots wins.

If the section is missing, the launcher boots the disk when one is attached,
otherwise the CD. With the egui display and no media at all, the window still
opens, so media can be attached from the Hardware page.

See the quick-start note on why `["disk", "cdrom"]` handles the whole
install-then-boot lifecycle without edits.

### `[disk]`: attach an existing image

```toml
[disk]
path = "my-disk.img"
channel = 0          # ATA channel 0 or 1 (default 0)
drive = 0            # master (0) or slave (1) (default 0)
chs = { cylinders = 1024, heads = 16, sectors_per_track = 63 }  # optional override
```

When `chs` is omitted, the geometry is worked out from the file size: 16 heads
and 63 sectors per track, with as many cylinders as the size needs. The file
must be a non-zero multiple of 512 bytes. Override `chs` only for images that
expect an unusual layout.

### `[disk.create]`: create a blank disk automatically

```toml
[disk.create]
path = "my-disk.img"
size = "12G"         # default 20G
overwrite = false    # default false
```

This creates the image when it is missing, at every power-on in the shell and
at every launch of a `terminal` or `headless` run. It cannot be combined with
`[disk] path` or `chs`. Two things people trip over:

- **Creation can take a while.** The image is a flat file extended to its
  full size. On a filesystem that leaves the gap unallocated (ext4, APFS) this
  is quick and the file is sparse. On NTFS the space is allocated in full, and
  a large image takes a while. The launcher shows a notice, but the window may
  look idle until it finishes.
- **`overwrite = false` keeps your data.** With the default `false`, an
  existing valid image is reused, never recreated, so your installed OS
  survives relaunches. Set `overwrite = true` only when you deliberately want a
  factory-reset disk on every launch. In the shell, that reset happens once
  per session, at the first power-on of a VM that uses the path, and the
  power-on that would erase an existing file asks first (`Overwrite <path>?`,
  with the button `Overwrite and power on`).

### `[cdrom]`

```toml
[cdrom]
path = "ubuntu-26.04-live-server-amd64.iso"
channel = 1          # default 1
drive = 0            # default 0
```

Put the CD on channel 1 when the disk is on channel 0. Each ATA channel/drive
slot can hold only one device, and keeping the two on separate channels
matches the classic PC layout the guest expects.

### `[logging]`

`level` is one of `"trace"`, `"debug"`, `"info"`, `"warn"` or `"error"`, and
defaults to `"warn"`. `"info"` adds one-shot device lifecycle messages (PCI BAR
assignments, VGA mode changes).

**Caveat:** release builds compile out `debug` and `trace` statements entirely
(the workspace enables `tracing`'s `release_max_level_info`). Setting
`level = "debug"` on a release binary therefore shows nothing extra.
Debug-level logging needs a debug build, which is far slower, so prefer
`info` and the one-shot messages.

## Engines

The guest's instructions can be run by one of two engines. The choice is made
with `--engine` on the command line or the **Engine** field on the Hardware
page's Processors pane. In the config file it is `emulator.engine`, which
`--engine` overrides.

- **`interpreter`** (the default) is this port's own CPU. It runs everywhere.
- **`whp`** runs the guest on the Windows Hypervisor Platform. It needs three
  things: a build that carries the engine, the platform enabled on the host,
  and a guest that fits the limits below. Only a Windows build with the
  `hv-whp` feature (on by default, and it resolves to nothing on other hosts)
  and without the diagnostic `guest-trace` feature carries the engine. When
  either of the first two is missing, the launch is refused rather than
  quietly run on the interpreter:
  - a build without the engine: ``this build has no hypervisor engine, so the
    `whp` engine (chosen by `--engine`, a VM file's `emulator.engine`, or the
    shell's Hardware › Processors › Engine) cannot run. The engine is built
    only for Windows, with the `hv-whp` feature and without `guest-trace`.
    Choose `interpreter` there, or run a build that carries the engine``;
  - a host without the platform: ``this host has no Windows Hypervisor
    Platform, so the `whp` engine (chosen by `--engine`, a VM file's
    `emulator.engine`, or the shell's Hardware › Processors › Engine) cannot
    run. Enable it with: dism /Online /Enable-Feature
    /FeatureName:HypervisorPlatform``.

On the hypervisor, some things behave differently:

- **The guest's clock is the host's clock.** The machine's devices run on the
  host's time, so `ips` does not pace the guest the way it does on the
  interpreter.
- **`max_instructions` is refused.** Nothing counts the instructions the
  hypervisor runs, so a limit would end the run at a guess.
- **The CPU is always narrowed to host-shared,** whatever
  `--cpu-capabilities` says (see below).
- **Only one processor is created**, so use a `1 × 1 × 1` topology.
- **There is no instruction rate.** The status strip reads `--- IPS`, and a
  headless run ends with
  `the guest ran on the hypervisor, which does not count instructions` instead
  of an instruction count.

### CPU capabilities

`--cpu-capabilities` chooses which processor the machine offers its guest. In
the config file it is `emulator.cpu_capabilities`, which the flag overrides:

- **`preset`** (the default) is this port's own CPU model, the same on every
  host. That is what makes a run reproducible.
- **`host-shared`** narrows the model to what this host can also run:
  - AVX-512 is withdrawn when the host's processor cannot hold the AVX-512
    register state;
  - AVX is withdrawn when it cannot hold the upper halves of the YMM
    registers;
  - `MONITOR`/`MWAIT` is always withdrawn, because the hypervisor platform
    cannot give that instruction pair to a guest.

Why a guest must never be offered more than it can run is explained in
[`whp-guest-capabilities.md`](whp-guest-capabilities.md). The divergence is
registered as H4 in
[`bochs-parity-divergences.md`](bochs-parity-divergences.md).

## Pacing: IPS, sync slowdown, max instructions

This is the least obvious part of the whole setup. It applies to the
interpreter; on the hypervisor, the guest's clock is the host's.

**`ips` calibrates the guest clock; it does not limit speed.** The emulator
declares that one virtual second equals `ips` instructions. The guest's sense
of time then runs at `real_throughput ÷ ips` of real time:

- **`ips` at or slightly above your machine's real sustained throughput:**
  the guest clock runs at, or slightly slower than, real time. This is
  correct.
- **`ips` too low:** the guest clock runs *faster* than real time, so timers
  and timeouts in the guest expire early. systemd's 90-second unit timeouts
  and D-Bus's 25-second call timeouts fire spuriously, and services "fail"
  during boot for no visible reason. If you see a cascade of red `FAILED`
  units, raise `ips`.
- **`ips` absurdly high:** guest-side sleeps cost proportionally more real
  time, and an idle guest feels sluggish.

To pick a value, watch the IPS readout in the status strip during a CPU-heavy
phase (kernel boot, say), and set `ips` a little above the peak you see. For
example, if the peak is around 106M, `120000000` is a good value.

**`sync_slowdown`** makes the emulator *sleep* so that guest time stays pinned
to real time, instead of running as fast as it can. Turn it on for
interactive sessions where wall-clock timing matters (cursor blink rates,
media, games). Leave it off for installs and boots, which you want to run as
fast as possible.

**`max_instructions`**: leave it out for normal use. A limit silently stops
the VM when it runs out; it is a benchmarking and CI feature.

## CPU topology

`sockets × cores × threads` is the hardware hierarchy the guest sees through
CPUID and the MP/ACPI tables, exactly as on real hardware:

- **sockets** are physical CPU packages (a 2-socket config is a dual-CPU
  server board);
- **cores** are independent execution units in each package;
- **threads** are SMT (hyper-threading) threads in each core.

Two practical notes:

- **More vCPUs do not make the interpreter faster.** It interleaves all
  logical CPUs on **one host thread**, each running `smp_quantum` instructions
  before the next gets its turn. Extra vCPUs split the same instruction budget
  and add scheduling overhead. What the split changes is what the guest
  *believes*: that matters to its scheduler (thread, core and socket
  affinity, NUMA assumptions) and to per-socket software licensing.
- **The hypervisor engine creates one processor**, so use `1 × 1 × 1` with
  `--engine whp`.

## Input: keyboard and mouse

- **Keyboard:** keys go to the guest whenever the VM is running and the
  display has focus. While the mouse is captured, *all* keys go to the guest,
  including chords like Ctrl+C.
- **Mouse:** click the display to capture it. From then on, relative motion,
  buttons and the wheel are forwarded to the guest's PS/2 mouse, and the host
  cursor is hidden. To release it, use **Release mouse** in the VM bar (on the
  Console page).
- **Ctrl+Alt+Del** has its own button in the VM bar on the Console page,
  because the host OS would otherwise intercept the chord.

## Performance expectations

- Disk and CD I/O go through IDE bus-master DMA, which requires `pci = true`.
  If a Linux guest prints `BMDMA: BAR4 is zero, falling back to PIO` in dmesg,
  PCI is disabled in your config and all I/O is running an order of magnitude
  slower than it should.
- Some boot phases are CPU-bound userspace work, such as an installer
  generating its APT cache or unpacking an initramfs. The screen sits on one
  line for a minute or two while the IPS readout stays high. That is progress,
  not a hang.

## Troubleshooting

**"It looks frozen."** Check these in order:

1. Is the **IPS readout** in the status strip still high and changing? Then
   the guest is running, most likely through a CPU-bound phase. Give it a
   minute.
2. Did you set **`max_instructions`**? The VM stops silently when the limit
   runs out, and a literal `0` in the config file or on the command line runs
   nothing at all.
3. Is **`pci_vga = true`**? The console goes dark between `bochs-drm` binding
   and the framebuffer console coming up. Retest with it off to compare.

**The badge reads Faulted.** An error notice is on screen. It names what
failed, for example a missing BIOS file or a disk image that could not be
attached.

**Services fail during boot with timeouts** (a `FAILED` cascade, D-Bus
errors). `ips` is set below your machine's real throughput, so guest time
runs fast and timeouts expire early. Raise `ips` above the observed IPS peak.

**Disk image errors at startup.** An existing file at the `[disk.create]` path
that is not a usable flat image is rejected rather than silently overwritten.
"Not usable" means it is empty, or its size is not a multiple of 512 bytes.
Set `overwrite = true` for one launch to replace it, then set it back to
`false` (while it is on, the file is recreated empty at every launch), or
delete or move it, or point `path` elsewhere.

**The `whp` engine is refused.** One of three things, each named by its
message (the first two are quoted under [Engines](#engines)):

- the launcher was built without the engine (any non-Windows host, a build
  without `hv-whp`, or a `guest-trace` build);
- the Windows Hypervisor Platform is not enabled (the error names the `dism`
  command that enables it);
- the VM sets `max_instructions`, which the hypervisor cannot honour:
  ``max_instructions = <N> cannot be honoured by the `whp` engine (chosen by
  `--engine`, a VM file's `emulator.engine`, or the shell's Hardware ›
  Processors › Engine): a processor on the hypervisor retires instructions
  the host does not count, so the limit would end the run at a guess. Remove
  the limit or choose `interpreter` there``.

**Rebuild fails with "access denied" on the executable** (Windows). A running
VM has the `.exe` locked. Power off and close the launcher before running
`cargo build`.

**Typo in the config.** Unknown keys fail the launch with a parse error naming
the key. Check the spelling against the tables above.

## Command-line flags

A flag overrides the config file for one run, without editing the file:

```bash
# Headless benchmark run, capped at 15 billion instructions
cargo run --release -p rusty_box_gui -- \
  --config rusty_box.toml --display headless --max-instructions 15000000000

# Terminal-mode boot of a different ISO
cargo run --release -p rusty_box_gui -- --cdrom other.iso --display terminal

# Flags alone, no file: a temporary VM, kept only if you keep it
cargo run --release -p rusty_box_gui -- \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom my-installer.iso --memory-mib 2048
```

| Flag | Config key | Meaning |
|---|---|---|
| `-f`, `--config <TOML>` | — | Config file to open as a temporary VM (conflicts with `--no-config`) |
| `--no-config` | — | Accepted for compatibility; nothing is loaded implicitly |
| `--bios <PATH>` | `rom.bios` | System BIOS ROM |
| `--vga-bios <PATH>` | `rom.vga_bios` | VGA BIOS ROM |
| `--display <terminal\|headless\|egui>` | `display.backend` | Display backend |
| `--engine <interpreter\|whp>` | `emulator.engine` | Which engine runs the guest (default `interpreter`); needs a machine, see below |
| `--cpu-capabilities <preset\|host-shared>` | `emulator.cpu_capabilities` | Which processor the guest is offered (default `preset`); needs a machine, see below |
| `--boot <disk,cdrom>` | `boot.order` | Boot order: `disk`, `cdrom`, or both, comma-separated, each at most once |
| `--disk <PATH>` | `disk.path` | Attach an existing disk image |
| `--disk-chs <C:H:S>` | `disk.chs` | Override the disk geometry (`,` also accepted as separator) |
| `--create-disk <PATH>` | `disk.create.path` | Create the disk image at this path (conflicts with `--disk` / `--disk-chs`) |
| `--create-disk-size <SIZE>` | `disk.create.size` | Size of the created disk, e.g. `12G` |
| `--overwrite-created-disk` | `disk.create.overwrite` | Recreate the image on every launch |
| `--cdrom <PATH>` | `cdrom.path` | Attach an ISO |
| `--memory-mib <MIB>` | `emulator.memory_mib` | Guest RAM |
| `--host-memory-mib <MIB>` | `emulator.host_memory_mib` | Host backing for guest RAM |
| `--memory-block-kib <KIB>` | `emulator.memory_block_kib` | Memory block granularity |
| `--ips <N>` | `emulator.ips` | Guest-clock calibration |
| `--max-instructions <N>` | `emulator.max_instructions` | Instruction limit (taken literally) |
| `--smp-quantum <N>` | `emulator.smp_quantum` | Instructions per CPU per turn, 1–32 |
| `--cpuid-freq <MODE>` | `emulator.cpuid_freq` | `hardware`, `none` or `ips` |
| `--sync-realtime` | `emulator.sync_realtime` | PIT and ACPI timers on wall-clock time |
| `--cpus <N>` | `emulator.cpus` | Flat CPU count (conflicts with the three below) |
| `--cpu-sockets <N>` / `--cpu-cores <N>` / `--cpu-threads <N>` | `emulator.cpu_sockets` / `cpu_cores` / `cpu_threads` | Explicit topology |
| `--pci` / `--no-pci` | `emulator.pci` | PCI bus on or off |
| `--sync-slowdown` / `--no-sync-slowdown` | `emulator.sync_slowdown` | Pin guest time to real time |
| `--log-level <trace\|debug\|info\|warn\|error>` | `logging.level` | Log level; needs a machine, see below |

`--engine`, `--cpu-capabilities` and `--log-level` are each one VM's own
setting, so they need a machine to apply to. On a command line that names
none — no `--config` and no flag describing one — they are refused rather
than dropped, and the message names the flags it found: `` `--engine`,
`--log-level` without a machine: this command line sets how a machine runs
but names none. Pass --config FILE or --bios/--cdrom/--disk…, or set it per
VM (in the shell: Hardware › Processors › Engine, Hardware › Display › Log
level; in the VM file: `emulator.engine`, `emulator.cpu_capabilities`,
`logging.level`) ``. `--display egui` alone, like no flags at all, opens the
library.
