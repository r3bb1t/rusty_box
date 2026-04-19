# Rusty Box UEFI

UEFI application that runs a Bochs-compatible x86 emulator. Boots Alpine Linux
via direct kernel boot (bypassing emulated BIOS POST). Emulator-in-emulator.

BIOS and VGA BIOS ROMs are embedded at compile time via `include_bytes!`.
The Alpine ISO is loaded from the EFI System Partition at runtime.

## Prerequisites

- Rust with `x86_64-unknown-uefi` target
- Python 3.6+ (for ISO creation)
- Alpine Linux ISO (download from [alpinelinux.org](https://alpinelinux.org/downloads/), Virtual x86_64)
- QEMU with OVMF, or VMware (EFI mode), for testing

## Build

```bash
rustup target add x86_64-unknown-uefi
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi
```

## Create bootable ISO

```bash
python make_iso.py --alpine-iso /path/to/alpine-virt-3.21.3-x86_64.iso
```

## Run

```bash
# QEMU (Linux)
qemu-system-x86_64 \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd \
  -cdrom rusty_box_uefi.iso \
  -serial stdio

# QEMU (Windows, QEMU installed to default path)
qemu-system-x86_64 ^
  -drive if=pflash,format=raw,readonly=on,file="C:/Program Files/qemu/share/edk2-x86_64-code.fd" ^
  -cdrom rusty_box_uefi.iso ^
  -serial stdio

# VMware: attach ISO as CD-ROM, set firmware to EFI in VM settings
# Bochs: set romimage to OVMF, attach ISO as ata0-slave cdrom
```

## EFI System Partition layout

```
/EFI/BOOT/BOOTX64.EFI       -- the emulator binary (includes BIOS ROMs)
/rusty_box/alpine.iso        -- Alpine Linux ISO
```

## How it works

1. UEFI firmware loads `BOOTX64.EFI`
2. The app reads `alpine.iso` from the ESP via UEFI Simple File System protocol
3. Parses ISO 9660 to extract `vmlinuz` and `initramfs`
4. Creates a 128 MB guest memory emulator (CPU, memory, timers)
5. Loads kernel + initramfs into guest memory via Linux boot protocol
6. Runs the emulated x86 CPU in a batch loop

## Limitations

- Direct kernel boot only (no emulated BIOS POST)
- Serial console output (no VGA framebuffer)
- 128 MB guest RAM (configurable in source)
- No networking in the guest
