# Getting Started with Rusty Box

This guide covers the `rusty_box_gui` launcher: where its firmware comes from,
how to set up a first VM, the settings each known guest needs, how the window
is organised, how the mouse and keyboard reach the guest, how to write a VM
file, and the settings whose meaning is not obvious (the engines, pacing, CPU
topology, disk provisioning and display options). Every flag, key and control
named here exists in `rusty_box_gui/src/`, or, for the console view with its
serial pane and mouse capture, in `rusty_box/src/gui/`.

## What you need

- **A Rust toolchain** (stable). Build with `cargo`, in release mode.
- **The Bochs firmware ROMs.** They are **not** in this repository: the
  launcher reads a system BIOS and a VGA BIOS from files you name. See
  [Getting the firmware](#getting-the-firmware).
- **A guest image**, meaning a bootable ISO (an installer, say) and/or a
  hard-disk image. The launcher can create a blank disk image for you. For a
  first VM, use the Alpine Linux **Virtual** x86_64 ISO,
  `alpine-virt-3.24.1-x86_64.iso`; the front page's
  [Getting an Alpine Linux ISO](../README.md#getting-an-alpine-linux-iso)
  says where to download it.

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
cargo run --release -p rusty_box_gui
```

The shell keeps its VMs in a library folder (`%APPDATA%\rusty_box\vms` on
Windows; see [rusty_box_gui/README.md](../rusty_box_gui/README.md#desktop)
for the other platforms) and lists every VM in it at every launch. When
`APPDATA` is unset, or not an absolute path, there is no such folder, and the
launch is refused with a message saying which variable it looked at.

On a first run the library is empty, so the shell opens on a blank temporary
VM called **New VM**, powered off. (A `rusty_box.toml` beside the executable
is the one exception: a first run imports it, as
[rusty_box_gui/README.md](../rusty_box_gui/README.md#desktop) describes.) Its
settings are the defaults listed under
[A new VM's defaults](#a-new-vms-defaults), and it boots no guest as it is:
[Alpine Linux 3.24.1](#alpine-linux-3241) walks through setting it up.

To start from a file instead, pass it with `--config <path>` (or
`-f <path>`): it opens as a temporary VM, marked "(unsaved)", and **Keep in
library** on its Summary page adds it to the library for good. A file that is
already in the library opens as that library VM instead, when nothing else on
the command line changes it (with `--memory-mib 64` beside it, say, it is a
temporary VM again, so the override never reaches the file). A
`rusty_box.toml` in the current or parent directory is not read, so a file
someone drops there cannot change what boots; `--no-config` is accepted and
changes nothing.

To start a VM, press `▶ Power on` in the bar at the top, or the `Power On VM`
tile on the VM's **Summary** page. The shell then switches to the **Console**
page.

## Running guests

These are the guests with a recorded milestone, with the settings on record
for each. The front page's [What runs today](../README.md#what-runs-today)
table lists the same guests. DLX Linux, the first row there, is booted by the
`dlxlinux` example rather than from the shell.

### A new VM's defaults

A blank **New VM**, and any key a VM file leaves out, gets:

| Setting | Default |
|---|---|
| Guest memory | 32 MiB |
| Host memory | the same as guest memory |
| Memory block | 128 KiB |
| Processors | 1 socket × 1 core × 1 thread |
| IPS target | 4,000,000 |
| PCI | on |
| Sync slowdown | off |
| Engine | Interpreter |
| BIOS, VGA BIOS, hard disk, CD/DVD | none |

No guest below is recorded at these values. Each recipe sets the memory and
the IPS target: Alpine uses 256 MiB and 300,000,000, and the others 2048 MiB
and 120,000,000 or 300,000,000. An IPS target far below what the host really
runs makes the guest's timers expire early (see
[Pacing](#pacing-ips-sync-slowdown-max-instructions)). The sidebar's `+`
button copies the selected VM, so a VM set up once can be the starting point
of the next.

### Alpine Linux 3.24.1

**What it reaches:** `login:`, a root login and a working shell, on the
interpreter and on WHP. The machine has no network adapter, so nothing that
needs a network works in the guest.

**In the shell,** starting from **New VM** or any other stopped VM:

1. **Hardware › Display:** `Browse…` beside `BIOS path` to
   `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest`, and beside
   `VGA BIOS path` to
   `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin`.
2. **Hardware › CD/DVD:** `Browse…` beside `ISO path` to
   `alpine-virt-3.24.1-x86_64.iso`. Choosing a file ticks `Enable CD/DVD`.
3. **Hardware › Memory:** set both `Guest memory` and `Host memory` to 256.
4. **Hardware › Processors:** set `IPS target` to 300000000, and leave one
   socket, one core and one thread. To run on WHP, set `Engine` to
   `Windows Hypervisor` (see [Execution engines](#execution-engines)).
5. **Summary:** give the VM a name. A temporary VM, such as **New VM**, also
   has `Keep in library` there: press it so that the VM is listed at the next
   launch.
6. Press `▶ Power on`.

**As a VM file:** save this as `alpine.toml` in the repository root, next to
the ISO, and open it with
`cargo run --release -p rusty_box_gui -- --config alpine.toml`. It opens as a
temporary VM named `alpine`. Add `engine = "whp"` under `[emulator]` to run
it on WHP. The front page's [Quick start](../README.md#quick-start) gives the
same machine as flags.

```toml
[emulator]
memory_mib = 256
ips = 300000000

[rom]
bios = "cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"
vga_bios = "cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"

[cdrom]
path = "alpine-virt-3.24.1-x86_64.iso"
```

`host_memory_mib` follows `memory_mib`, and with only a CD attached the boot
order is the CD.

**What to expect:**

- The ISOLINUX `boot:` prompt needs no key: the ISO's boot menu times out
  after one second of guest time, and the shell also queues one Enter for any
  VM whose boot order starts with the CD/DVD.
- The kernel and OpenRC's services start, and the display ends on
  `Welcome to Alpine Linux 3.24` and `localhost login:`. A `login:` prompt
  appears in the serial pane too.
- Log in as `root`; no password is asked.

**How long:** the `alpine_probe` harness, with the same memory and IPS
target, measured power-on to `login:` at 65.0 s on the interpreter and 27.1 s
on WHP (median of 3 runs per engine, Intel Core i5-12450H, 2026-09-12). It
offered both engines the `host-shared` processor; the shell's interpreter
offers `preset` unless the VM file sets `cpu_capabilities = "host-shared"`,
so a shell boot on the interpreter is not the identical machine.

**Not recorded:** installing Alpine to a disk (`setup-alpine`).

### Ubuntu Server 26.04 live-server

**What it reaches:** the installer, from
`ubuntu-26.04-live-server-amd64.iso`. A finished install, or an installed
system booting, is not recorded.

**The VM file** is on the front page, under
[The Ubuntu Server recipe](../README.md#the-ubuntu-server-recipe): 2048 MiB of
guest and host memory, one processor (1 × 1 × 1), `ips` 120,000,000, PCI on,
sync slowdown off, `smp_quantum` 32, a 1280×720 display at 32 bits per pixel,
and the CD as the only boot device. It attaches no hard disk.

**In the shell,** the same machine is: `Guest memory` and `Host memory` 2048;
`IPS target` 120000000; `Display resolution` `1280×720 @ 32bpp`; the ISO
under CD/DVD; `Register VGA on PCI (experimental KMS / bochs-drm)` left off.
`smp_quantum` has no control in the shell (see
[Settings only the VM file carries](#settings-only-the-vm-file-carries)).

**Engine and how long:** the maintainer reports that it reaches the installer
in under 5 minutes on WHP and in 20–30 minutes on the interpreter. These are
not harness measurements. To run it on WHP, choose `Windows Hypervisor` under
Hardware › Processors › Engine, add `engine = "whp"` under the file's
`[emulator]`, or pass `--engine whp`.

### Windows 10 22H2

**What it reaches:** the installer starts. Its start screen was reached on the
interpreter after 88.2 billion guest instructions, on 2026-07-14. Nothing past
that screen is recorded, nor any run on WHP, nor any run since that July
record.

**Settings on record:** the Windows 10 22H2 ISO on the CD/DVD, 2 GiB of guest
and host memory, `ips` 120,000,000, PCI on and sync slowdown off, on the
interpreter. As a VM file, with your own ISO's name in `path`:

```toml
[emulator]
memory_mib = 2048
host_memory_mib = 2048
ips = 120000000
pci = true
sync_slowdown = false

[rom]
bios = "cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"
vga_bios = "cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"

[cdrom]
path = "windows-10-22h2-x64.iso"
```

**Not recorded:** the processor count, a hard disk and its size, a display
mode, whether a key had to be pressed at a "boot from CD" prompt, and whether
the installer was driven with the mouse. The file leaves each at its default:
one processor, no disk, the default display.

**How long:** in that July run, the launcher's headless display reached the
same instruction count in about 17½ minutes (1,053 s and 1,057 s on two
builds of that time). No later build is timed.

### Windows 7 SP1

**What it reaches:** Setup, on the interpreter: the edition picker, on
2026-07-25. No run on WHP is claimed.

**Settings on record:** 2048 MiB of memory, the CD as the boot device and
`ips` 300,000,000; set up the VM as in the Windows 10 file above, with
`ips = 300000000`. The run used a Windows 7 SP1 x64 installation ISO (a
third-party build of it) and took about 20 minutes to reach the picker.

## The machine the guest sees

- **The processor** is always the Intel Core i7 Skylake-X model. No flag, key
  or shell control chooses another; see
  [The emulated PC](../README.md#the-emulated-pc) for its extensions and the
  device list.
- **The devices** are a PS/2 keyboard and mouse, IDE disks and an ATAPI
  CD-ROM with bus-master DMA, serial ports, VGA with the Bochs VBE extensions,
  and the PCI, ACPI, HPET and APIC chipset around them.
- **There is no network adapter, no sound card, no USB controller and no
  floppy controller.** No guest has networking; the mouse is a PS/2 mouse,
  never a USB tablet; and a floppy image the Images page creates cannot be
  attached.

## The window

The desktop shell has four parts:

- the **VM bar** across the top;
- a **sidebar tree** on the left;
- the **page** for whatever is selected in the tree;
- a **status strip** along the bottom.

### Sidebar tree

The sidebar, headed `My computer`, lists the VMs in the library and has a
`Search VMs` box that filters them by name, boot order, and disk and CD/DVD
path, ignoring case. The selected VM opens into its four pages:
**Summary**, **Console**, **Hardware** and **Images**. Clicking a page moves
to it. Clicking another VM switches to it and lands on its Summary page, but
only once the running VM has been stopped: while a VM runs or starts, the
switch is refused with `Stop the running VM before selecting another VM.`
The VM being left is saved first.

Every VM is a complete launch configuration, kept in its own file in the
library. The VM the command line described, if any, comes first, marked
"(unsaved)"; a library VM whose last save failed is marked "(save failed)".
The sidebar's `+` button adds a new VM to the library, copied from the
selected one; while a VM runs it is refused with
`Stop the running VM before adding a VM.` Library files that do not load are
listed under **Could not load**, each with its error as a tooltip and a
Delete button. The **☰** button at the left of the VM bar shows or hides the
sidebar.

### VM bar

The bar shows the selected VM's name and a state badge:

- **Stopped**
- **Starting**
- **Running**
- **Faulted**, shown while an error notice is on screen and the VM is neither
  running nor starting. The notice's `Close` button clears it.

It also carries the verbs that change that state:

- `▶ Power on` is enabled when the VM is stopped and not already starting.
- `■ Power off` and `↻ Restart` are enabled while the VM is running.
- `…` opens a menu with `About Rusty Box Workstation` and `Quit`.

While the Console page is shown, the bar adds three controls for the guest's
console:

- `Ctrl+Alt+Del` sends that chord to the guest (the host OS would otherwise
  intercept it). It is enabled only while the VM runs.
- `Capture mouse` / `Release mouse` toggles mouse capture (see
  [Mouse and keyboard](#mouse-and-keyboard)). It is enabled only while the
  VM runs.
- `Show serial` / `Hide serial` toggles the serial pane. It is always
  enabled.

When the bar is too narrow for all three, they move into the `…` menu.

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
it uses are kept. `Cancel`, Escape or a click outside the dialog dismisses
it. Three tiles lead elsewhere: **Power On VM**, **Create Disk Image** (goes
to the Images page) and **Hardware Settings** (goes to the Hardware page).

### Console page

The Console page shows the guest's display, with the serial pane below it
when that is shown; it is shown by default. While the VM is off, the display
reads `This VM is powered off`. While a startup disk is being created, the
display shows a spinner and a notice naming the image until the guest draws
its first frame.

The serial pane is headed `Serial Console (ttyS0)` and shows what the guest
writes to its first serial port, COM1:

- the log, scrolled to its end;
- a `serial input` line: `Send`, or Enter, sends the line with a newline
  added, and works only while the VM runs;
- `Copy Log`, which copies the whole log to the clipboard;
- a `Paste` button, which is always disabled.

Dragging the pane's top edge resizes it. What appears in it is up to the
guest: Alpine's live ISO puts a `login:` prompt there as well as on the
display, and a guest that does not use its serial port leaves the pane empty.

### Hardware page

The Hardware page lists six devices. Selecting one opens its settings:

| Device | Settings |
|---|---|
| **Memory** | `Guest memory` and `Host memory`, each 1 to 4096 MB; `Memory block`, 1 to 65,536 KiB |
| **Processors** | `Sockets`, `Cores / socket` and `Threads / core`, with the logical total and a warning above the SMP limit; `IPS target`; `Sync slowdown`; `Engine` (`Interpreter`, or `Windows Hypervisor` in a Windows build that carries it); `Max instructions` (0 means unlimited) |
| **Devices** | `Enable PCI`; `Boot order (first match boots)`, whose entries move earlier or later or are removed, with `Add disk` / `Add cdrom` for an attached device the order leaves out; `Effective boot order` |
| **Hard Disk** | `Enable hard disk`, `Disk path` with `Browse…`, `ATA channel` and `drive`, `Override CHS geometry`; `Detected CHS` and `Controller` for an attached disk, or `Attached disk` `None`; `Create disk image`, which opens the Images page |
| **CD/DVD** | `Enable CD/DVD`, `ISO path` with `Browse…`, `ATA channel` and `drive`, `Boot CD/DVD first`; `Controller`, or `Attached CD/DVD` `None` |
| **Display** | `BIOS path` and `VGA BIOS path`, each with `Browse…`; `Log level`; `Display resolution`; `Register VGA on PCI (experimental KMS / bochs-drm)`; `Adapter`, `Applied BIOS` and `Applied VGA BIOS`; `Open console` |

Settings can be edited only while the VM is powered off, and they take effect
at the next power-on. While the VM runs, the page reads
`Power off before changing VM hardware.`

- **Browse.** Choosing a file for `Disk path` or `ISO path` also ticks that
  device's `Enable` box.
- **Boot order.** An attached device the order leaves out is added to it,
  after the listed ones; `Effective boot order` shows the order a power-on
  uses.
- **Display resolution.** The choices are `Default (VGA / VBE)` and five
  presets: `1024×768 @ 32bpp`, `1280×720 @ 32bpp`, `1280×1024 @ 32bpp`,
  `1600×1200 @ 32bpp` and `1920×1080 @ 32bpp`. The caption under it reads
  `Raises the VBE ceiling so the guest can select this mode (via GRUB gfxpayload / vesafb).`
  Any other mode is set in the VM file.
- **Register VGA on PCI.** Its tooltip reads
  `Exposes the adapter as PCI 1234:1111 so Linux bochs-drm can bind for a full KMS framebuffer. Experimental — verify with a guest boot.`
- **Memory above 4096 MB.** The Memory pane holds both memory fields to
  1–4096 MB. A VM file may ask for more, but showing that VM's Memory pane
  lowers the value to 4096, and the VM's next power-on or save then uses
  4096. Keep a larger guest's memory in the file, and do not open its Memory
  pane.

An edit is saved to the VM's library file when it ends — when the field
loses focus or the drag ends — and at once when you switch VMs, add a VM,
keep one, power on, or the window goes behind another or closes. The file is
rewritten each time, so **hand-written comments in it are lost**. A temporary
VM (one started with `--config` or flags) is saved only once you keep it in
the library; until then the page reads
`Not saved. Keep this VM in the library from its Summary page.`

The config file's `emulator.engine` key chooses the engine (see
[Execution engines](#execution-engines)); `--engine` on the command line
overrides it.

### Images page

The Images page creates blank images with the `rusty_box_bximage` backend:

- `Kind`: `Hard disk` or `Floppy`.
- `Path`, which starts as `c.img`, with `Browse…`: a native save dialog that
  suggests `c.img` or `floppy.img`. Dropping a file onto the page puts its
  path in the field.
- For a hard disk, `Size`, which starts as `20G`, with
  `Examples: 10M, 512M, 20G, 512` beside it. A bare number means MiB. The
  size must be at least 10 MiB, and under 8064 GiB.
- For a floppy, `Floppy format`: one of the ten bximage formats, from 160 KB
  to 2.88 MB.
- `Overwrite existing file`, off by default. Without it, an existing file is
  refused.
- `Create image` creates it.

A new hard disk is attached to the selected VM at once if the VM is stopped,
with the notice `Attached created disk image to <name>.` If the VM is running
or starting, the image is still created, but not attached: the notice reads
`Disk image created. Stop the VM before attaching it.`, and you attach it
yourself under Hardware › Hard Disk once the VM is off. A floppy image is only
written, with the notice
`Floppy image created. Floppy drive emulation is not wired yet.`: the machine
has no floppy controller.

### Status strip

The status strip shows:

- the state;
- the engine;
- memory and CPU count;
- the measured instruction rate, which reads `--- IPS` when there is none;
- `Restart queued` while a restart is pending.

If the state the shell shares with the emulator thread cannot be read, the
strip shows `State unavailable` instead.

## Mouse and keyboard

The guest has a PS/2 keyboard and a PS/2 mouse on the 8042 controller. The
mouse reports relative motion only: there is no USB tablet or other absolute
pointer, so the guest's pointer moves by as much as the host pointer moves,
not to where the host pointer is.

**The mouse, in the desktop shell:**

- **Capturing.** Capture needs a running VM. Click the guest's display, or
  press `Capture mouse` in the VM bar on the Console page. The click that
  captures is not passed to the guest.
- **While captured,** and while the host pointer is over the display, its
  motion and the left, right and middle buttons go to the guest, and the host
  cursor is hidden.
- **Capture is not a pointer lock.** The host pointer can leave the display.
  Outside it, the cursor shows again and nothing reaches the guest until the
  pointer comes back. Since the motion is relative, the two pointers then
  sit in different places; move the host pointer back over the display and
  carry on.
- **Releasing.** Only `Release mouse` ends capture. No key does, and powering
  the VM off does not either.
- **The wheel does not reach the guest.** The mouse is a plain PS/2 mouse,
  which is never switched into IntelliMouse wheel mode.

**The keyboard, in the desktop shell:**

- While the VM runs and the Console page is shown, keys go to the guest,
  unless a text field of the shell has the keyboard, such as the
  `serial input` line or `Search VMs`. While the mouse is captured, keys go
  to the guest even then. Clicking the display captures the mouse, so it also
  gives the keyboard back to the guest.
- **Keys are sent by position, not by character.** A key reaches the guest as
  the physical key it is, whatever the host's keyboard layout, and the guest's
  own layout setting decides which character it types.
- Ctrl+C, Ctrl+X and Ctrl+V reach the guest as those chords.
- The right-hand Shift, Ctrl and Alt keys reach the guest as the left-hand
  ones.
- A chord the host OS takes before the window sees it, as it usually does
  Ctrl+Alt+Del, does not reach the guest; the `Ctrl+Alt+Del` button in the VM
  bar sends that one.

**The browser shell** forwards the mouse whenever the pointer is over the
display, with no capture step, and does not hide the cursor. It forwards keys
while its Console page is shown and no text field has the keyboard. A browser
reports no physical key, so there the host's layout decides the key sent.

**On Android** the Console page has its own header instead of the VM bar, so
it has no `Capture mouse`, `Ctrl+Alt+Del` or `Show serial` button. A `Keys`
button at the bottom right opens a pad with a `Text to type` field and `Send`,
and buttons for `Esc`, `Tab`, `Enter`, `Backspace`, `Left`, `Up`, `Down`,
`Right`, `Ctrl+C` and `Ctrl+Alt+Del`. Touch input to the guest is not
recorded.

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
| `engine` | `"interpreter"` | Which engine runs the guest: `"interpreter"` or `"whp"`. See [Execution engines](#execution-engines). |
| `cpu_capabilities` | `"preset"` | Which processor the guest is offered: `"preset"` or `"host-shared"`. See [CPU capabilities](#cpu-capabilities). |
| `memory_mib` | `32` | Guest RAM in MiB. The default is too small for the guests under [Running guests](#running-guests): Alpine uses 256, and Ubuntu Server and Windows 2048. |
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
taken literally, so `0` runs nothing at all. Only the GUI's `Max instructions`
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

`order = ["disk", "cdrom"]` suits a VM that is to install a guest onto a
fresh disk. On the first boot the disk is blank (it has no boot signature),
so the BIOS falls through to the CD and starts the installer. Once an
installer has written a boot loader to the disk, the *same* order boots the
disk instead of the installer again. No guest install is recorded yet (see
[Running guests](#running-guests)).

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

Without `overwrite`, this creates the image when it is missing, at every
power-on in the shell and at every launch of a `terminal` or `headless` run.
With `overwrite = true`, see the second point below. It cannot be combined with
`[disk] path` or `chs`. Two things people trip over:

- **Creation can take a while.** The image is a flat file extended to its
  full size. On a filesystem that leaves the gap unallocated (ext4, APFS) this
  is quick and the file is sparse. On NTFS the space is allocated in full, and
  a large image takes a while. In the shell, the Console page shows a spinner
  and a notice naming the image until the guest draws its first frame.
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

### Settings only the VM file carries

The shell has no control for these; a VM file sets them, and most also have
a flag (see [Command-line flags](#command-line-flags)). The shell keeps what a
VM file sets for each when it saves the VM:

- `emulator.cpu_capabilities` (on WHP it is `host-shared` whatever the file
  says);
- `emulator.sync_realtime`, `emulator.smp_quantum` and `emulator.cpuid_freq`;
- `display.backend`, which decides whether `--config FILE` opens the shell or
  runs in the terminal or headless;
- a `display.width` and `display.height` other than the five presets, or a
  `bpp` other than 32;
- `[disk.create]`: the Images page creates a disk at once and attaches it as a
  plain `disk.path`.

Guest and host memory above 4096 MiB, and a memory block above 65,536 KiB,
are also set only in the file, and are lowered to the pane's limit when the
Memory pane shows them (see [Hardware page](#hardware-page)).

## Execution engines

The guest's instructions are run by one of two engines. The choice is made
with `--engine` on the command line or with Hardware › Processors › Engine in
the shell. In the config file it is `emulator.engine`, which `--engine`
overrides.

- **`interpreter`** (the default) is this port's own CPU. It runs everywhere,
  including the Android APK and the browser shell. It counts every
  instruction, so the status strip shows the rate and `max_instructions`
  works, and with the default `preset` capabilities it offers the same
  processor on every host.
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

### Which one to choose

Choose `whp` for Alpine Linux or Ubuntu Server, with one processor, on a
Windows host with the platform enabled. It boots both sooner:

> Alpine 3.24.1 reaches `login:` in 27.1 s on WHP vs 65.0 s on the interpreter (2.40×), Intel Core i5-12450H, 2026-09-12, commit 4f46a11 (median of 3 interleaved runs per engine, `alpine_probe`, timed from power-on to `login:`).

- Ubuntu Server 26.04 reaches its installer in under 5 minutes on WHP,
  against 20–30 minutes on the interpreter. That is the maintainer's report,
  not a harness measurement.
- The gain comes after the boot loader starts. BIOS POST is slower on WHP:
  4.9 s there against 0.6 s on the interpreter, in the same measurement.
- The figure is a time to `login:`, not a throughput ratio: a WHP machine's
  devices run on host time, so much of its 27 s is the guest's own timed
  waits and idle time.

Choose the interpreter for everything else:

- **DLX Linux**, which does not reach `login:` on WHP today;
- **Windows**, which is claimed only on the interpreter;
- **more than one processor**, which only the interpreter runs;
- **an instruction limit**, an instruction count, or a processor that is the
  same on every host;
- **Android and the browser**, which have no other engine.

The front page's
[Windows Hypervisor Platform](../README.md#windows-hypervisor-platform)
section has the rest of the measurement.

### How a machine on WHP differs

- **The guest's clock is the host's clock.** The machine's devices run on the
  host's time, so `ips` does not pace the guest the way it does on the
  interpreter.
- **One processor runs.** The partition has one virtual processor, which
  runs the machine's processor 0. A larger topology is accepted, not refused:
  its other processors are built but never run. Use `1 × 1 × 1`.
- **`max_instructions` is refused.** Nothing counts the instructions the
  hypervisor runs, so a limit would end the run at a guess.
- **The CPU is always narrowed to host-shared,** whatever
  `--cpu-capabilities` says (see below).
- **There is no instruction rate.** The status strip reads `--- IPS`, and a
  headless run ends with
  `the guest ran on the hypervisor, which does not count instructions` instead
  of an instruction count.
- **One partition per process.** A process can hold only one hypervisor
  partition at a time.
- **Other differences a guest can see** are registered as `H` entries in
  [`bochs-parity-divergences.md`](bochs-parity-divergences.md). Among them:
  A20 never masks an address (H1), the time-stamp counter is the host's (H2),
  `CPUID` withholds VMX and SVM (H3), the processor may be narrowed (H4), and
  device time is host time (H5).

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

To start, use the value from the guest's recipe under
[Running guests](#running-guests). To tune it, watch the IPS readout in the
status strip during a CPU-heavy phase (kernel boot, say), and set `ips` a
little above the peak you see. For example, if the peak is around 106M,
`120000000` is a good value.

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
- **The hypervisor engine runs only processor 0.** A larger topology is
  accepted with `--engine whp`, but its other processors never run, so use
  `1 × 1 × 1` there.

## Performance expectations

- Disk and CD I/O go through IDE bus-master DMA, which requires `pci = true`.
  If a Linux guest prints `BMDMA: BAR4 is zero, falling back to PIO` in dmesg,
  PCI is disabled in your config and all I/O is running an order of magnitude
  slower than it should.
- Some boot phases are CPU-bound userspace work, such as an installer
  generating its APT cache or unpacking an initramfs. The screen sits on one
  line for a minute or two while the IPS readout stays high. That is progress,
  not a hang.
- The recipes under [Running guests](#running-guests) give each guest's
  recorded time.

## Troubleshooting

**"It looks frozen."** Check these in order:

1. On the interpreter, is the **IPS readout** in the status strip still high
   and changing? Then the guest is running, most likely through a CPU-bound
   phase. Give it a minute. (On WHP the readout is always `--- IPS`.)
2. Did you set **`max_instructions`**? The VM stops silently when the limit
   runs out, and a literal `0` in the config file or on the command line runs
   nothing at all.
3. Is **`pci_vga = true`**? The console goes dark between `bochs-drm` binding
   and the framebuffer console coming up. Retest with it off to compare.

**The badge reads Faulted.** An error notice is on screen. It names what
failed, for example a missing BIOS file or a disk image that could not be
attached.

**Keys do not reach the guest.** The Console page must be shown, and a text
field of the shell may have the keyboard. Click the guest's display: that
captures the mouse, and while it is captured every key goes to the guest.

**The guest's pointer stopped following the mouse.** The host pointer has
left the display; move it back over the display. The guest's pointer moves
relative to where it was, not to where the host pointer is.

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
message (the first two are quoted under
[Execution engines](#execution-engines)):

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
# Headless benchmark run of the Alpine VM file, capped at 15 billion instructions
cargo run --release -p rusty_box_gui -- \
  --config alpine.toml --display headless --max-instructions 15000000000

# Terminal-mode boot of an ISO: nothing is loaded implicitly, so name the BIOS
cargo run --release -p rusty_box_gui -- \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom other.iso --display terminal

# Flags alone, no file: the Alpine recipe as a temporary VM, kept only if you keep it
cargo run --release -p rusty_box_gui -- \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --memory-mib 256 --ips 300000000
```

The terminal-mode run uses the defaults for everything it does not name, so
add the memory and IPS target the guest needs (see
[Running guests](#running-guests)).

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
