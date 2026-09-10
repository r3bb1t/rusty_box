# Rusty Box UEFI

A UEFI application that runs the Rusty Box emulator on UEFI firmware. Inside the
emulator it boots the DLX Linux disk image through a full BIOS POST, which makes
it an emulator running as a UEFI program on real or virtual hardware.

**No Rust allocator.** `rusty_box` is built with `default-features = false`, so
the `alloc` crate is not linked. Every large structure lives in pages from
UEFI's `allocate_pages`. That makes this crate the reference for building a
machine without an allocator.

## Construction without an allocator

`src/main.rs` places the machine in three steps, each into memory the caller
owns. In outline (the real code reports each failure with `bail!` rather than
`?`):

```rust
// 1. The CPU, in zeroed UEFI pages.
let cpu = BxCpuBuilder::new().init_cpu_at(cpu_ptr, ())?;            // unsafe

// 2. Guest memory, over an external buffer of
//    rusty_box::config::mem_buffer_size(guest_bytes) bytes.
let mem_stub = BxMemoryStubC::create_from_raw(
    mem_ptr, mem_buf_size, guest_bytes, host_bytes, block_size)?;   // unsafe

// 3. The machine, in a &'static mut MaybeUninit<Emulator>.
let emu = MachineBuilder::new(config)
    .bios(BIOS_ROM)
    .vga_bios(VGA_BIOS)
    .boot_order(BootOrder::just(BootDevice::Disk))
    .disk_static(AtaSlot::PRIMARY_MASTER, DLX_DISK,
                 DiskGeometry::new(DLX_CYLINDERS.into(), DLX_HEADS, DLX_SPT))
    .build_at(emu_storage, cpus, mem_stub)?;
```

`build_at` exists only in builds without `alloc`. It places the machine in the
storage it is given, then runs the same hardware initialisation and reset that
`MachineBuilder::build` performs, with no `Box`. The machine borrows its CPUs as
a slice, `cpus`, which also lives in UEFI pages. UEFI pages are never freed here,
so the `'static` borrows hold for the life of the program.

## What it does at run time

1. Switches to a 1 MiB stack allocated from UEFI pages; the firmware's own
   stack is often much smaller.
2. Allocates and builds the machine as above: 32 MB of guest RAM, PCI enabled,
   and the embedded DLX disk as the primary master.
3. Queues keystrokes meant as F1, to pass the BIOS keyboard-error prompt.
4. Runs the guest with `emu.step(RunBudget::Instructions(100_000))`, up to
   20,000,000,000 instructions in total. After every step it prints what the
   guest wrote to the BIOS debug port (0xE9) and to COM1 on the UEFI console.
5. After 50,000,000 instructions, types keystrokes meant as a `root` login.
6. Stops when `outcome.is_terminal()` reports guest power-off, CPU shutdown, a
   stop request or an engine fault, or when a step returns an error. It then
   waits 30 seconds before returning to the firmware.

## Current status

Last observed: the BIOS completes POST and hands control to the boot sector
(`Booting from 0000:7c00`). On the way it detects 32 MB of RAM, enumerates the
PCI devices (i440FX, PIIX3, IDE, ISA bridge), builds the MP, SMBIOS, ACPI and
HPET tables, and identifies the VGA BIOS and the ATA disk (306/4/17 CHS). DLX
Linux has not been seen to finish loading after that point. This has not been
re-measured against the run loop described above.

Gaps visible in the code:

- The keystrokes are written as Set 1 scancodes (`0x3B`/`0xBB` for F1;
  `0x13`/`0x93`, `0x18`/`0x98`, … for `root` and Enter). `Keyboard::scancodes`
  takes Set 2 bytes whenever the 8042 is translating, which it is at reset
  (`scancodes_translate: true` in `rusty_box/src/iodev/keyboard.rs`). The guest
  therefore receives different keys.
- Nothing presses Enter at DLX's `LILO boot:` prompt, which waits indefinitely.
  The headless `dlxlinux` example sends one when the prompt appears.
- Only port 0xE9 and COM1 reach the UEFI console. DLX prints its kernel
  messages and login prompt to the VGA text screen, which this app never shows.

## Prerequisites

- A Rust toolchain with the `x86_64-unknown-uefi` target.
- The three files the binary embeds with `include_bytes!`. They are not in the
  repository (`/cpp_orig` and `/dlxlinux` are gitignored), so the build fails
  until they exist at these paths, relative to the workspace root:

  | File | Size | Source |
  |------|------|--------|
  | `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` | 128 KB | a Bochs source checkout |
  | `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` | 32 KB | a Bochs source checkout |
  | `dlxlinux/hd10meg.img` | 10.2 MiB | [Bochs DLX Linux disk image](https://bochs.sourceforge.io/diskimages.html) |

- A UEFI machine to run it on: QEMU with OVMF, VMware Workstation with UEFI
  firmware, or real hardware.

## Build

```bash
rustup target add x86_64-unknown-uefi
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi
```

The binary is written to
`target/x86_64-unknown-uefi/release/rusty_box_uefi.efi`. It is larger than the
10.2 MiB disk image it carries.

The `verbose` feature adds a log line roughly every 500,000 instructions. The
line shows the running total, RIP, the step's progress with its unit, and the
interrupt flag:

```bash
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi --features verbose
```

`cargo xtask ci` builds this crate in its "UEFI example build" step, without
`verbose`.

## Prepare the boot directory

UEFI firmware boots `EFI/BOOT/BOOTX64.EFI` from a FAT volume. From the workspace
root:

```bash
mkdir -p rusty_box_uefi_disk/EFI/BOOT
cp target/x86_64-unknown-uefi/release/rusty_box_uefi.efi rusty_box_uefi_disk/EFI/BOOT/BOOTX64.EFI
printf '\\EFI\\BOOT\\BOOTX64.EFI\r\n' > rusty_box_uefi_disk/startup.nsh
```

```
rusty_box_uefi_disk/
+-- EFI/
|   +-- BOOT/
|       +-- BOOTX64.EFI    # the emulator, with both ROMs and the DLX disk embedded
+-- startup.nsh            # the UEFI Shell runs this at startup
```

`python examples/rusty_box_uefi/make_iso.py` builds the same directory in the
current directory (the `--output` option names it; the default is
`rusty_box_uefi_disk`) and prints QEMU commands (without `-m`). `make_vmdk.py`
builds the same `rusty_box_uefi_disk` directory, whatever `--output` says.
Despite its name, it writes no VMDK. Both scripts delete the directory first if
it exists.

Both scripts also create a `rusty_box/` subdirectory in the output directory,
which the app does not use. If they find an Alpine ISO (given with
`--alpine-iso`, or the first `alpine-virt*.iso` in the workspace root), they
copy it there, as `rusty_box_uefi_disk/rusty_box/alpine.iso`. The app never
reads that file, and `make_iso.py`'s warning that the app "will fail at
runtime" without one does not apply.

### QEMU with OVMF

**Linux:**
```bash
qemu-system-x86_64 \
  -m 512 \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd \
  -drive format=vvfat,rw=on,dir=rusty_box_uefi_disk \
  -nographic
```

**Windows:**
```cmd
qemu-system-x86_64 ^
  -m 512 ^
  -drive if=pflash,format=raw,readonly=on,file="C:\Program Files\qemu\share\edk2-x86_64-code.fd" ^
  -drive format=vvfat,rw=on,dir=rusty_box_uefi_disk ^
  -nographic
```

### VMware Workstation

Set **VM Settings → Options → Advanced → Firmware type** to **UEFI**, and give
the VM a FAT-formatted disk that holds the contents of `rusty_box_uefi_disk/`.
Neither helper script produces a VMDK, so make that disk with an external tool.

### Real hardware

1. Format a USB drive as FAT32.
2. Copy the contents of `rusty_box_uefi_disk/` to the drive's root.
3. Boot from the drive with UEFI boot enabled.

## Memory layout

| Allocation | Size | Method |
|------------|------|--------|
| Stack | 1 MiB | `uefi::boot::allocate_pages`, switched to in `asm!` |
| CPU (`BxCpuC`) | `size_of::<BxCpuC>()`, printed at startup | `allocate_pages` + `BxCpuBuilder::init_cpu_at` |
| CPU slice | one `&'static mut BxCpuC` | `allocate_pages` |
| Guest memory buffer | 32 MiB of RAM + 4 MiB BIOS ROM + 128 KiB expansion ROM + 8 KiB (`rusty_box::config::mem_buffer_size`) | `allocate_pages` + `BxMemoryStubC::create_from_raw` |
| Machine (`Emulator`) | `size_of::<Emulator>()`, printed at startup | `allocate_pages` + `MachineBuilder::build_at` |

No `#[global_allocator]` is defined.

## Limitations

- The boot gaps listed under [Current status](#current-status).
- Guest disk writes complete but are discarded: the DLX image is a
  `&'static [u8]`, and the ATA write path for a borrowed image stores nothing.
- No VGA output reaches the screen; only port 0xE9 and COM1 are printed.
- 32 MB of guest RAM, set in the `EmulatorConfig` in `src/main.rs`.
- No network device; the emulator does not model one.
- No GeForce display model: it needs `alloc`, which this build does not link.
