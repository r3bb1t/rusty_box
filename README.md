# Rusty Box

Rusty Box is a Rust port of the [Bochs](https://bochs.sourceforge.io/) x86 emulator: a whole 32/64-bit x86 PC. It runs guests on its own software interpreter on every host, or on the Windows Hypervisor Platform (WHP) on Windows. Its front end, Rusty Box Workstation, is a VMware-style VM shell that runs on the desktop, on Android and in the browser.

## What runs today

| Guest | How far it gets | Engine | Settings on record |
|-------|-----------------|--------|--------------------|
| DLX Linux (the Bochs `hd10meg.img`) | The login prompt, then an interactive bash shell. `cargo xtask ci` boots it to `dlx login:` on every run. | Interpreter. It does not reach `login:` on WHP today (see [Execution engines](#execution-engines)). | the `dlxlinux` example |
| Alpine Linux 3.24.1 (`alpine-virt-3.24.1-x86_64.iso`) | `login:`, a root login and a working shell. No network: the machine has no network adapter. | Interpreter and WHP | 256 MiB, `ips` 300,000,000 |
| Ubuntu Server 26.04 live-server (`ubuntu-26.04-live-server-amd64.iso`) | Boots to the installer: in under 5 minutes on WHP, against 20–30 minutes on the interpreter (reported by the maintainer, not timed by a harness). | WHP and interpreter | 2048 MiB, one CPU, `ips` 120,000,000, `smp_quantum` 32, a 1280×720×32 display, CD only ([the VM file](#the-ubuntu-server-recipe)) |
| Windows 10 22H2 | The installer starts: its start screen was reached after 88.2 billion guest instructions (2026-07-14). | Interpreter | 2 GiB, `ips` 120,000,000, PCI on |
| Windows 7 SP1 | Setup reached: the edition picker (2026-07-25). | Interpreter | 2048 MiB, `ips` 300,000,000 |

What is not claimed:

- a finished install of Windows or Ubuntu, or an installed system booting;
- anything past the Windows 10 installer's start screen;
- a run of any Windows on WHP;
- networking of any kind, in any guest.

The table is for the desktop shell and the example harnesses. The Android shell has booted Alpine on a phone as far as the ISOLINUX `boot:` prompt (2026-09-11). The gate checks that the browser shell compiles, and no guest boot in it is recorded.

The [UEFI application](examples/rusty_box_uefi/) runs the emulator on UEFI firmware with no allocator. It completes BIOS POST and reaches the boot sector.

### The Ubuntu Server recipe

The Ubuntu row's settings as a VM file. Save it in the repository root, next to the ISO, and open it with `cargo run --release -p rusty_box_gui -- --config ubuntu.toml`. It opens as a temporary VM, which `Keep in library` on its Summary page adds to the library.

```toml
[emulator]
memory_mib = 2048
host_memory_mib = 2048
cpu_sockets = 1
cpu_cores = 1
cpu_threads = 1
ips = 120000000
pci = true
sync_slowdown = false
smp_quantum = 32

[display]
width = 1280
height = 720
bpp = 32

[rom]
bios = "cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"
vga_bios = "cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"

[boot]
order = ["cdrom"]

[cdrom]
path = "ubuntu-26.04-live-server-amd64.iso"
```

Relative paths in a VM file resolve against the file's own folder.

## Firmware and disk images

The repository does not ship the BIOS ROMs or the DLX disk image. Both locations are gitignored:

| File | Expected path | Source |
|------|---------------|--------|
| System BIOS | `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` | a Bochs checkout: `git clone https://github.com/bochs-emu/Bochs cpp_orig/bochs` |
| VGA BIOS | `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` | the same checkout |
| DLX Linux disk | `dlxlinux/hd10meg.img` | [Bochs disk images](https://bochs.sourceforge.io/diskimages.html) |

The Bochs checkout is also the C++ reference source that every port decision is checked against. The browser builds, the UEFI application and `cargo xtask ci` embed or read these files, so they fail without them. [rusty_box/examples/README.md](rusty_box/examples/README.md) lists the other locations the desktop examples search.

## Quick start

Always build with `--release`. Debug builds are far too slow to boot a guest.

1. Clone Bochs for the ROMs, as in the table above.
2. Run the shell:

   ```bash
   cargo run --release -p rusty_box_gui
   ```

3. It opens on the VM library, powered off. A new VM starts at 32 MiB of memory and an IPS target of 4,000,000, and no guest in the table boots with those. For Alpine, set the BIOS and VGA BIOS paths under Hardware › Display, the memory to 256 MiB under Hardware › Memory, the IPS target to 300,000,000 under Hardware › Processors, and attach the ISO under Hardware › CD/DVD. Then press `▶ Power on` in the VM bar.

The same machine can be given as flags. This one boots Alpine with the settings from the table:

```bash
cargo run --release -p rusty_box_gui -- \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --boot cdrom --memory-mib 256 --ips 300000000
```

On Windows, with the Hypervisor Platform enabled, add `--engine whp` to run it on the hypervisor.

Headless harnesses and the gate:

```bash
# DLX Linux, headless (boots to the login prompt)
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# Alpine Linux, headless BIOS boot
ALPINE_ISO=alpine-virt-3.24.1-x86_64.iso RUSTY_BOX_HEADLESS=1 cargo run --release --example alpine_direct --features std

# The full local gate
cargo xtask ci
```

`alpine_direct` prints the guest's serial console and a progress line every 50 million instructions. It does not stop at `login:`; it runs until its instruction budget is spent. The repository's `.cargo/config.toml` sets `MAX_INSTRUCTIONS=20000000000` for every cargo command, so the six boot examples, which all read it, stop after 20 billion instructions unless you override it.

Flags and VM files work together:

- Machine flags such as `--bios` or `--cdrom` describe a **temporary VM**, marked "(unsaved)" until you keep it in the library.
- `--config PATH` opens a file as a temporary VM too.
- A `--config` file that is already one of the library's VM files opens as that library VM, if nothing on the command line changes it: no machine flag, and none of `--engine`, `--cpu-capabilities` or `--log-level`.
- Nothing is read from the current directory, and `--no-config` changes nothing.
- To power on, a VM needs a BIOS path and a hard disk or CD/DVD.

[docs/getting-started.md](docs/getting-started.md) walks through the config file and the settings whose meaning is not obvious. [rusty_box_gui/README.md](rusty_box_gui/README.md) covers the shell in full.

On Linux, the egui GUI needs the display-server development packages that CI installs:

```bash
sudo apt-get install -y pkg-config libgl1-mesa-dev libx11-dev libxi-dev libxcursor-dev \
  libxrandr-dev libxinerama-dev libxkbcommon-dev libwayland-dev
```

## Execution engines

`rusty_box_gui` runs a guest on one of two engines, chosen per VM under Hardware › Processors › Engine, or with `--engine`.

### Interpreter

The interpreter (`--engine interpreter`) is the default. It is this port of Bochs, and it runs on every host and target, the Android APK and the browser included. It counts every instruction, so a run can stop at `max_instructions`. The status strip shows the live instruction rate. That rate depends on the host CPU and on the guest's instruction mix: BIOS real-mode code runs slower than a long-mode kernel or userspace. `ips` calibrates the guest clock against it without limiting speed; [docs/getting-started.md](docs/getting-started.md) explains how to choose it.

### Windows Hypervisor Platform

The WHP engine (`--engine whp`) runs the guest's instructions on the host processor, through `WinHvPlatform`. The devices are still this port's own models, run on host time by a device thread, except the local APIC: that is the hypervisor partition's own, in x2APIC mode (xAPIC where the host offers only that).

It needs:

- a Windows build of `rusty_box_gui` that carries the hypervisor path: the `hv-whp` feature, on by default, and no `guest-trace`;
- a Windows host with the Hypervisor Platform enabled.

When the build lacks the path or the host lacks the platform, `--engine whp` is refused. It never falls back to the interpreter.

Measured at commit `4f46a11` with the `alpine_probe` harness:

> Alpine 3.24.1 reaches `login:` in 27.1 s on WHP vs 65.0 s on the interpreter (2.40×), Intel Core i5-12450H, 2026-09-12, commit 4f46a11 (median of 3 interleaved runs per engine, `alpine_probe`, timed from power-on to `login:`).

- From the boot loader (ISOLINUX) to `login:`, WHP is 2.90× faster: 22.2 s against 64.4 s.
- Ubuntu Server 26.04 reaches its installer in under 5 minutes on WHP, against 20–30 minutes on the interpreter. That is the maintainer's report, not a harness measurement.
- The firmware stage is slower on WHP. From power-on to the boot loader, BIOS POST takes 4.9 s there against 0.6 s on the interpreter, so the whole gain comes after the boot loader starts.
- This is a boot-time figure, not a throughput ratio. A WHP machine's devices run on host time, so much of its 27 s is the guest's own timed waits and idle time.
- DLX Linux does not reach `login:` on WHP today. It gets through the BIOS and LILO, and then its kernel loses disk interrupts.

A machine on the hypervisor differs from an interpreter machine in these ways:

- **One processor runs.** The partition has one virtual processor. On a machine built with more, only processor 0 executes, and the machine is not refused.
- **No instruction count.** A `max_instructions` limit is refused, and the status strip has no IPS readout: it shows `--- IPS`.
- **Device timers run on host time**, not on the instruction count.
- **The guest is always offered `host-shared` CPU capabilities**, whatever `--cpu-capabilities` says. That is the guest's CPU model narrowed to what the host can carry: it drops AVX-512 unless the host holds its state, and always drops `MONITOR`/`MWAIT`. On the interpreter, the default `preset` offers the whole model, the same on every host. [docs/whp-guest-capabilities.md](docs/whp-guest-capabilities.md) explains why.
- **The local APIC is the hypervisor's.** Every other device is this port's model on both engines; on WHP the local APIC is the partition's own, so the APIC timer and the APIC registers a guest programs are the hypervisor's, not this port's.
- **One partition per process.** A process can hold only one hypervisor partition at a time.
- **Windows desktop only.** The Android APK and the browser shell run the interpreter.

`cargo xtask ci` runs the WHP crates' tests; those that need hardware skip when there is no hypervisor. No guest boot on WHP is part of the gate.

## The shell: Rusty Box Workstation

`rusty_box_gui` is the launcher meant for people who want to run a VM rather than program against the library. [rusty_box_gui/README.md](rusty_box_gui/README.md) has the detail. In brief:

- **The VM library.** Every VM is one TOML file in a per-user folder. On Windows that is `%APPDATA%\rusty_box\vms`. Every VM file there is listed at each launch, and an edit is saved to its file when the edit ends. A VM given on the command line is a temporary VM, marked "(unsaved)", until `Keep in library` adds it. A `+` button copies the selected VM, and `Delete VM` removes a VM's file but keeps its disk images.
- **Four pages per VM.**
  - **Summary:** the VM's state and settings.
  - **Console:** the guest's display, with a serial pane that has an input line and a `Copy Log` button.
  - **Hardware:** memory; processors, with the engine; PCI and boot order; the hard disk; the CD/DVD; ROMs and display resolution. Its edits apply at the next power-on.
  - **Images:** creates disk images.
- **The VM bar** carries `▶ Power on`, `■ Power off` and `↻ Restart`. On the Console page it adds `Ctrl+Alt+Del`, `Capture mouse` / `Release mouse` and `Show serial` / `Hide serial`. The status strip below the page shows the state, the engine, memory and CPU count, and the instruction rate.
- **Disk creation.** The Images page creates bximage-compatible flat hard disks from 10 MiB to just under 8064 GiB, and floppy images in the ten bximage formats. A new hard disk is attached to the selected VM if it is stopped. A floppy image cannot be attached, because the machine has no floppy controller. `--create-disk` provisions a boot disk at startup.
- **Mouse and keyboard.**
  - The guest has a PS/2 mouse. Click the guest's display to capture it, or use `Capture mouse`; `Release mouse` releases it.
  - Capture hides the host cursor over the display but does not lock the pointer, which can leave the display.
  - The mouse wheel does not reach the guest: the mouse is a plain PS/2 mouse, which is never switched into IntelliMouse wheel mode.
  - The keyboard is forwarded while the VM runs.

### Android

```bash
cargo xtask android build
cargo xtask android run
cargo xtask android screenshot rustybox_android.png
```

The APK runs the same shell as the desktop, laid out for a phone:

- Its VMs live in a library in the app's storage.
- Its Browse buttons open an on-device file browser.
- A Keys pad sends the keys a soft keyboard lacks.
- The Bochs ROMs travel inside the APK. The ISO a VM boots is chosen on the phone, under Hardware › CD/DVD.
- It runs the interpreter only.

`cargo xtask android` installs the Android SDK components, the Rust target and `cargo-apk` as needed. It signs with a generated local dev keystore under your home directory, and uses `adb_client` for install, launch and screenshots. See [xtask/README.md](xtask/README.md).

### Browser

Two front ends build for `wasm32-unknown-unknown` with [trunk](https://trunkrs.dev/) (`rustup target add wasm32-unknown-unknown`, `cargo install --locked trunk`). Both compile the BIOS ROMs in, so the Bochs checkout above must be present:

```bash
# The GUI's browser shell
cd rusty_box_gui && trunk serve --release --port 8080

# The standalone web demo: boots the embedded DLX image (needs dlxlinux/hd10meg.img) or an uploaded Alpine ISO
cd examples/rusty_box_web && trunk serve --release --port 8080
```

Then open `http://localhost:8080`.

The browser shell:

- boots an uploaded `.iso` or `.img` file as its CD/DVD;
- sets the memory, from 1 to 4096 MB, and the processor count before boot;
- downloads blank disk images of up to 64 MiB;
- forwards the mouse whenever the pointer is over the display, with no capture step;
- has no host filesystem, no VM files and no command line;
- runs the interpreter only.

## The emulated PC

**CPU**

- Intel Core i7 Skylake-X is the model every machine runs. An AMD Ryzen model, with AMD SVM, is ported and unit-tested, but `MachineBuilder` and the GUI cannot select it.
- Full x87 FPU on Berkeley SoftFloat 3e (80-bit extended precision).
- AVX-512 (F, DQ, CD, BW, VL), AVX2, SSE4.2, AES-NI, SHA, BMI1/BMI2.
- Intel VMX, implemented and unit-tested. No run of a hypervisor inside a guest is recorded.
- SMP guests on the interpreter (`--cpus`, or `--cpu-sockets` / `--cpu-cores` / `--cpu-threads`). The WHP engine runs one processor.
- System Management Mode.

**Chipset and devices**

- the i440FX PCI host bridge and the PIIX3 PCI-to-ISA bridge; PCI can be switched off;
- the 8259 PIC pair and an I/O APIC, and a local APIC per processor, with x2APIC (on WHP the local APIC is the hypervisor partition's own);
- the 8254 PIT, the CMOS real-time clock, the 8237 DMA controller and the HPET;
- PIIX3 IDE with bus-master DMA, for hard disks (32 GB and larger) and an ATAPI CD-ROM;
- the 8042 keyboard controller, with a PS/2 keyboard and a PS/2 mouse;
- VGA with the Bochs VBE extensions. It can also be registered on PCI as 1234:1111, for Linux `bochs-drm`; that option is experimental;
- a 16550A UART on COM1 (ports `0x3F8`–`0x3FF`); there is no COM2 to COM4;
- PIIX4 ACPI power management, with S5 soft-off and S3 suspend, and SMBus ports;
- QEMU `fw_cfg`;
- port 92h, the A20 gate and fast reset.

There is no network adapter, no sound card (not even the PC speaker), no USB controller and no floppy controller.

**Memory and firmware**

- Guest memory is block-based and can exceed 4 GB. A guest larger than its host-memory budget (`host_memory_mib`) keeps the rest in an overflow file.
- The firmware is the Bochs BIOS and the LGPL VGA BIOS, from a Bochs checkout.

**Snapshots**

- An interpreter machine can be saved and restored in a fresh process; see the `snapshot_resume` example.

## Using it as a library

- **Building a machine.** `MachineBuilder::new(config).build()` builds an interpreter machine, and `build_on::<WhpEngine>()` builds a hypervisor one. The engine is part of the machine's type.
- **Instrumentation.** A machine's type parameter `T: Instrumentation` observes the CPU. `()` observes nothing, and its empty hook mask makes the CPU skip every dispatch. A hook such as `pre_syscall` sees each system call: `shellcode_trace` uses it to intercept a shellcode's syscalls, and `alpine_strace` to trace Alpine's.
- **No allocator.** The core compiles without `alloc` (no_std, no heap) for bare-metal and UEFI targets. The [UEFI application](examples/rusty_box_uefi/) places the whole machine in caller-provided memory.

## Examples

Developer harnesses in `rusty_box/examples/`, each run with `cargo run --release --example <name> --features <features>`:

| Example | Features | What it does |
|---------|----------|--------------|
| `dlxlinux` | `std` | DLX Linux in the terminal, or headless with `RUSTY_BOX_HEADLESS=1` (the CI boot gate) |
| `dlxlinux_egui` | `std,gui-egui` | DLX Linux in an egui window |
| `rusty_box_egui` | `std,gui-egui` | One window for DLX or Alpine (`RUSTY_BOX_BOOT=dlx\|alpine\|alpine-direct`; picks Alpine when it finds an ISO) |
| `alpine_direct` | `std` | Alpine Linux from the ISO, BIOS or direct kernel boot |
| `alpine` | `std` | Alpine Linux from a disk image or ISO |
| `alpine_strace` | `std,gui-egui` | Alpine with guest syscall tracing |
| `shellcode_trace` | `std` | Runs a Linux x86-64 shellcode in flat long mode and intercepts its syscalls with a `pre_syscall` hook |
| `snapshot_resume` | `std` | Boots a guest from CD, saves a snapshot, and restores it in a fresh process (set `RB_ISO` to a bootable ISO) |
| `perfbench` | `std` | CPU-core microbenchmark, needing no BIOS or disk (`cargo xtask perf-baseline` archives a build of it) |

The WHP engine's harnesses are in `rusty_box_whp_engine/examples/`: `dlx_whp`, `alpine_probe`, `alpine_bench`, `compute_bench` and `step_bench`. The engine figure above comes from `alpine_probe`. [rusty_box/examples/README.md](rusty_box/examples/README.md) documents the environment variables.

## Getting an Alpine Linux ISO

1. Visit [alpinelinux.org/downloads](https://alpinelinux.org/downloads/).
2. Download the **Virtual** x86_64 ISO. This page and the harnesses use `alpine-virt-3.24.1-x86_64.iso`.
3. Place it in the project root and run from the root. The harnesses look for it differently:
   - `alpine_strace`, `alpine_bench` and `scripts/boot_gate.sh` open `alpine-virt-3.24.1-x86_64.iso` by default;
   - `alpine_direct` defaults to `alpine-virt-3.23.3-x86_64.iso`, so pass `ALPINE_ISO=alpine-virt-3.24.1-x86_64.iso`, as in the quick start;
   - the `rusty_box_egui` example also picks up any `.iso` whose name contains `alpine`, in the workspace root, its parent, or the current directory.

The browser builds upload the ISO from the page instead.

## Architecture

```
Emulator<T: Instrumentation = (), E = SoftwareEngine>
+-- cpus            one BxCpuC<T> per processor, boot processor at index 0
|                   (the model is runtime data: CpuModel::Corei7SkylakeX | CpuModel::AmdRyzen;
|                   MachineBuilder always builds Corei7SkylakeX)
+-- engine: E       what retires guest instructions: SoftwareEngine (the interpreter)
|                   or rusty_box_whp_engine::WhpEngine
+-- BxMemC          memory (block-based, supports >4 GB)
+-- BxDevicesC      I/O port dispatch (65536 ports, fixed arrays)
+-- DeviceManager   devices: interrupt fabric, PIT, CMOS, DMA, keyboard, HPET, IDE,
|                   VGA, ACPI, PCI host and ISA bridges, serial, fw_cfg
+-- BxPcSystemC     timers and the A20 line
+-- gui             NoGui, TermGui or EguiGui [alloc only]
```

`T` is the instrumentation hook, a type rather than a feature: `()` observes nothing, and its empty hook mask makes the CPU skip every dispatch. Instructions execute on an `ExecCtx`, which holds disjoint borrows of one CPU and the machine parts it touches for one scheduler slice.

## Feature flags (`rusty_box`)

| Flag | Default | Description |
|------|---------|-------------|
| `std` | yes | Standard library: terminal display, file-backed disks, tempfile. Implies `alloc`. |
| `alloc` | yes (via `std`) | Heap allocation. Enables `MachineBuilder::build()`, the display front ends, diagnostics and `StopHandle`. |
| `gui-egui` | yes | egui/eframe display (`EguiGui`). |
| `profiling` | no | Profiling counters in the CPU loop. Implies `std`. |
| `bench` | no | Criterion benchmarks (`benches/cpu_bench.rs`). Implies `std`. |

`rusty_box_gui` has its own features: `gui-egui` and `hv-whp` (both default), `guest-trace` and `windows-gui-subsystem`.

### Build configurations

```bash
# The emulator library with its default features
cargo build --release

# no_std + no_alloc: core emulation only, no heap
cargo check --release -p rusty_box --no-default-features

# no_std + alloc: adds MachineBuilder::build(), display front ends, diagnostics
cargo check --release -p rusty_box --no-default-features --features alloc

# UEFI application: no allocator, placement construction
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi
```

### Key design principles

- **No global state** -- each `Emulator` is fully self-contained. Interpreter machines can run side by side; a process can hold only one WHP partition at a time.
- **Bochs parity** -- every behavioural divergence from the Bochs C++ source is a bug. The deliberate exceptions are registered, with evidence, in [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md).
- **no_std + no_alloc core** -- CPU, memory, decoder, devices and the machine compile without `alloc`; fixed-size arrays and ring buffers replace `Vec`/`VecDeque`.
- **CPU models as data** -- `CpuModel` is a closed runtime enum (Skylake-X, Ryzen); the model answers CPUID leaves and supplies the ISA, VMX and SVM extension bitmasks cached at init. `MachineBuilder` builds every processor as Skylake-X.
- **`Send` by derivation** -- the tree has no `unsafe impl Send`; the [Safety Doctrine](docs/safety-doctrine.md) (R0-R9) governs every API and safety seam.

## Project structure

```
rusty_box/
+-- rusty_box/                 # Emulator library: CPU, memory, machine, I/O devices, examples
|   +-- src/cpu/               # CPU (instruction handlers, mirrors Bochs cpu/)
|   +-- src/memory/            # Memory subsystem
|   +-- src/iodev/             # Most machine-wired devices (PIT, CMOS, IDE, serial, ACPI, HPET, ...)
|   +-- src/pic.rs, src/dma.rs # The 8259 PIC pair and the 8237 DMA controller
|   +-- src/emulator/          # Emulator, MachineBuilder, execution engines
|   +-- examples/              # Developer harnesses (DLX, Alpine, egui, tracing, benchmarks)
+-- rusty_box_core/            # no_std, forbid(unsafe) foundations (the engine seam and GPA plan, ring buffer, time units, snapshot sections)
+-- rusty_box_devices/         # no_std, forbid(unsafe) device models with no CPU or bus access (VGA, PCI)
+-- rusty_box_decoder/         # x86 instruction decoder
+-- rusty_box_whp_sys/         # Windows Hypervisor Platform FFI leaf (every hypervisor call is confined to its windows.rs)
+-- rusty_box_whp/             # Safe wrapper over the WHP leaf
+-- rusty_box_whp_engine/      # Runs a rusty_box machine's guest on WHP
+-- rusty_box_gui/             # Desktop, Android and browser VM shell (egui): VM library, CLI/TOML config, interpreter or WHP engine, disk image creation
+-- rusty_box_bximage/         # bximage-compatible disk image creation
+-- xtask/                     # Local CI gate (cargo xtask ci) and automation
+-- examples/rusty_box_web/    # Standalone WASM web demo
+-- examples/rusty_box_uefi/   # UEFI application (no allocator)
+-- examples/no_alloc_smoke/   # no_alloc link smoke test for x86_64-unknown-none
+-- scripts/                   # Bochs table generators (gen_*.py) and developer tooling
+-- docs/                      # Doctrine, parity registry, user guide, WHP notes
+-- cpp_orig/bochs/            # Bochs checkout: reference source and ROMs (gitignored)
+-- dlxlinux/                  # DLX Linux disk image (gitignored)
```

## Testing

```bash
# The full local gate; run it before every commit
cargo xtask ci
cargo xtask ci --skip-boot   # without the DLX boot gate
cargo xtask ci --full        # plus the integration tests and the GUI release build

# The fast loop
cargo test --release -p rusty_box --lib --features std

# One crate
cargo test --release -p rusty_box_decoder

# Fuzz the decoder (targets: fuzz_fetchdecode64, fuzz_fetchdecode32_32, fuzz_fetchdecode32_16)
cd rusty_box_decoder && cargo +nightly fuzz run fuzz_fetchdecode64
```

A bare `cargo test` tests only the default workspace member (`rusty_box`), in a debug build.

`cargo xtask ci` runs 27 steps; [xtask/README.md](xtask/README.md) lists every one.

1. The doctrine ratchets, over eight source trees, the GUI's included. They check the unsafe-token baselines, that there is no `unsafe impl Send/Sync`, the blanket `dead_code` allows, and that there is no production `unwrap`/`expect`.
2. A 25-step matrix:
   - It tests `rusty_box_core`, `rusty_box_devices`, the three WHP crates, the decoder and `rusty_box`.
   - It checks `rusty_box_core` and `rusty_box_devices` for bare metal.
   - It checks `rusty_box` without std (with and without alloc), for bare metal, for wasm, with all features, and with debug assertions.
   - It checks `rusty_box_gui` for the host, with its tests, and for wasm.
   - It builds the UEFI application.
   - It runs the public-API and doc examples and the doctrine compile-fail fixtures.
3. The DLX boot gate: DLX boots headlessly to its login prompt, on the interpreter. It is the only guest boot in the gate.

The gate does not run the tests of `rusty_box_gui` or `rusty_box_bximage`; run those with `cargo test --release -p <crate>`.

It needs the rustup targets `x86_64-unknown-none`, `x86_64-unknown-uefi` and `wasm32-unknown-unknown`, plus the firmware and disk image above. [CONTRIBUTING.md](CONTRIBUTING.md) has the details.

## Documentation

- [docs/getting-started.md](docs/getting-started.md) -- using the GUI: config file, pacing, CPU topology, disks, display
- [rusty_box_gui/README.md](rusty_box_gui/README.md) -- the shell: the VM library, pages, engines, disk images, Android, browser
- [CONTRIBUTING.md](CONTRIBUTING.md) -- build, test, and the rules every change follows
- [docs/safety-doctrine.md](docs/safety-doctrine.md) -- the Safety Doctrine, rules R0-R9
- [docs/bochs-parity-divergences.md](docs/bochs-parity-divergences.md) -- the registry of deliberate divergences from Bochs
- [docs/bochs-upstream-bugs.md](docs/bochs-upstream-bugs.md) -- upstream Bochs bugs this port does not reproduce
- [docs/whp-guest-capabilities.md](docs/whp-guest-capabilities.md) -- what a machine on the hypervisor may advertise to its guest
- Crate READMEs: [examples](rusty_box/examples/README.md), [rusty_box_web](examples/rusty_box_web/README.md), [rusty_box_uefi](examples/rusty_box_uefi/README.md), [rusty_box_bximage](rusty_box_bximage/README.md), [xtask](xtask/README.md)

## References

- [Bochs x86 Emulator](https://bochs.sourceforge.io/)
- [Intel Software Developer Manual, Volume 2](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) (Instruction Set Reference)
- [Sandpile.org](https://www.sandpile.org/) (x86 opcode maps)

## License

This project is a derivative work of the [Bochs](https://bochs.sourceforge.io/) x86 emulator
and is licensed under the [GNU Lesser General Public License v2.1](LICENSE) (LGPL-2.1-or-later).

See [THIRD-PARTY-LICENSES](THIRD-PARTY-LICENSES) for bundled third-party code (Berkeley SoftFloat 3e, Hauser FPU transcendentals).
