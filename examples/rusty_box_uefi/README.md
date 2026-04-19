# Rusty Box UEFI

UEFI application that runs a Bochs-compatible x86 emulator on bare UEFI firmware.
Boots DLX Linux via full BIOS POST -- an emulator-in-emulator running on real (or virtual) hardware.

**No Rust allocator required.** All large structures (CPU ~17 MB, Emulator ~3 MB, guest RAM 36 MB)
are placed via UEFI `allocate_pages` with pointer-based initialization. The `alloc` crate is not
linked at all.

## What it does

The UEFI application:

1. Allocates memory for CPU, Emulator, and 32 MB guest RAM via UEFI boot services
2. Loads embedded BIOS ROM, VGA BIOS, and DLX Linux disk image (all compiled in)
3. Runs a full BIOS POST: memory sizing, PCI enumeration, ACPI/SMBIOS/MP tables
4. Boots DLX Linux through the emulated BIOS boot path (MBR -> LILO -> kernel)
5. Prints BIOS and serial output to the UEFI console

## Current status

The emulator completes full BIOS POST and reaches `Booting from 0000:7c00`.
The BIOS successfully:

- Detects 32 MB RAM
- Enumerates PCI devices (i440FX, PIIX3, IDE, ISA bridge)
- Builds MP, SMBIOS, ACPI, and HPET tables
- Identifies VGA BIOS and ATA disk (306/4/17 CHS geometry)
- Transfers control to the boot sector

DLX Linux boot from the MBR is in progress -- the kernel load phase after
`Booting from 0000:7c00` currently stalls. This is a known limitation being
investigated (likely related to read-only disk data or IDE DMA timing).

## Prerequisites

- Rust toolchain with the `x86_64-unknown-uefi` target
- A UEFI VM or machine for testing (VMware Workstation, QEMU+OVMF, or real hardware)

## Step-by-step: Build and run

### 1. Install the UEFI target

```bash
rustup target add x86_64-unknown-uefi
```

### 2. Build the EFI binary

```bash
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi
```

The binary is produced at:
```
target/x86_64-unknown-uefi/release/rusty_box_uefi.efi
```

### 3. Prepare EFI disk directory

Create a directory with the standard EFI boot layout:

```bash
mkdir -p efi_disk/EFI/BOOT
cp target/x86_64-unknown-uefi/release/rusty_box_uefi.efi efi_disk/EFI/BOOT/BOOTX64.EFI
```

Or use the helper script:

```bash
python examples/rusty_box_uefi/make_vmdk.py
```

This creates a `rusty_box_uefi_disk/` directory with the correct layout.

### 4a. Run with VMware Workstation

1. **Create a new VM:**
   - Guest OS: Other 64-bit
   - Memory: 512 MB or more
   - Remove the default hard disk

2. **Enable EFI firmware:**
   - VM Settings -> Options -> Advanced -> Firmware type: **UEFI**

3. **Set up the EFI disk:**
   - Copy the `efi_disk/` directory contents into your VM folder
     (so `<VM folder>/efi_disk/EFI/BOOT/BOOTX64.EFI` exists)
   - Add a hard disk: Use an existing virtual disk -> browse to `boot.vmdk`
   - OR: create a shared folder pointing to `efi_disk/` and navigate from UEFI Shell

4. **Add a startup.nsh** (optional, for auto-boot):
   Create `efi_disk/startup.nsh` containing:
   ```
   \EFI\BOOT\BOOTX64.EFI
   ```

5. **Power on the VM.** The emulator starts automatically via `BOOTX64.EFI`.

6. **To update after rebuilding:**
   ```bash
   cp target/x86_64-unknown-uefi/release/rusty_box_uefi.efi "<VM folder>/efi_disk/EFI/BOOT/BOOTX64.EFI"
   ```
   Then restart the VM.

### 4b. Run with QEMU

Requires OVMF firmware (included with most QEMU installations).

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

### 4c. Run on real hardware

1. Format a USB drive as FAT32
2. Copy the `efi_disk/` contents to the USB root
3. Boot from the USB drive with UEFI boot enabled in BIOS settings

## EFI disk layout

```
efi_disk/
+-- EFI/
|   +-- BOOT/
|       +-- BOOTX64.EFI    # The emulator binary (~4 MB, includes BIOS ROMs + DLX disk)
+-- startup.nsh             # Optional: auto-run script for UEFI Shell
```

All data is embedded in the binary at compile time:
- **BIOS ROM** (`BIOS-bochs-latest`, 128 KB)
- **VGA BIOS** (`VGABIOS-lgpl-latest.bin`, 40 KB)
- **DLX Linux disk** (`hd10meg.img`, 10 MB)

## How it works

1. UEFI firmware loads `BOOTX64.EFI`
2. The app switches to a 1 MB heap-allocated stack (UEFI default is ~128 KB)
3. Allocates pages for BxCpuC (~17 MB), Emulator (~3 MB), and guest RAM (~36 MB)
4. Initializes CPU via `BxCpuBuilder::init_cpu_at()` (placement construction)
5. Creates memory stub via `BxMemoryStubC::create_from_raw()` (external buffer)
6. Initializes Emulator via `Emulator::init_at()` (no Box, no allocator)
7. Loads BIOS + VGA BIOS into guest memory, configures CMOS and disk geometry
8. Runs the emulated CPU in 100K-instruction batches with device ticking
9. Drains BIOS debug output (port 0xE9) and serial output (COM1) to UEFI console

## Verbose output

Build with the `verbose` feature for per-batch instruction count and RIP logging:

```bash
cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi --features verbose
```

## Memory layout

| Allocation | Size | Method |
|------------|------|--------|
| CPU (BxCpuC) | ~17 MB | `uefi::boot::allocate_pages` + `init_cpu_at` |
| Guest RAM + BIOS ROM | ~36 MB | `uefi::boot::allocate_pages` + `create_from_raw` |
| Emulator struct | ~3 MB | `uefi::boot::allocate_pages` + `init_at` |
| Stack | 1 MB | `uefi::boot::allocate_pages` + asm switch |
| **Total** | **~57 MB** | No Rust `#[global_allocator]` used |

## Limitations

- DLX Linux boot stalls after `Booting from 0000:7c00` (under investigation)
- Disk writes are silently dropped (DLX disk image is `&'static [u8]`, read-only)
- Serial console output only (no VGA framebuffer rendering)
- 32 MB guest RAM (configurable in source)
- No networking in the guest
- GeForce GPU emulation excluded (requires alloc for 16-256 MB VRAM)
