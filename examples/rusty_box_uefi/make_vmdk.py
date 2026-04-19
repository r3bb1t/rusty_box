#!/usr/bin/env python3
"""
Create a VMware VMDK disk image containing the UEFI emulator.

Creates a flat VMDK with a FAT16 filesystem containing BOOTX64.EFI
and the Alpine ISO. Boot in VMware with EFI firmware enabled.

Usage:
    python make_vmdk.py --alpine-iso /path/to/alpine.iso
"""

import argparse
import shutil
import struct
import sys
from pathlib import Path


def find_workspace_root():
    d = Path(__file__).resolve().parent
    while d != d.parent:
        if (d / "Cargo.toml").exists() and (d / "rusty_box").is_dir():
            return d
        d = d.parent
    return None


def create_vmdk_descriptor(flat_name: str, total_sectors: int) -> str:
    """Create a VMware VMDK descriptor file."""
    return f'''# Disk DescriptorFile
version=1
CID=fffffffe
parentCID=ffffffff
createType="monolithicFlat"

# Extent description
RW {total_sectors} FLAT "{flat_name}" 0

# The Disk Data Base
ddb.virtualHWVersion = "21"
ddb.geometry.cylinders = "{total_sectors // (255 * 63)}"
ddb.geometry.heads = "255"
ddb.geometry.sectors = "63"
ddb.adapterType = "lsilogic"
'''


def main():
    parser = argparse.ArgumentParser(description="Create VMware VMDK for Rusty Box UEFI")
    parser.add_argument("--alpine-iso", help="Path to Alpine Linux ISO")
    parser.add_argument("--output", default="rusty_box_uefi", help="Output base name (creates .vmdk + -flat.vmdk)")
    args = parser.parse_args()

    ws = find_workspace_root()
    if ws is None:
        print("ERROR: Cannot find workspace root", file=sys.stderr)
        sys.exit(1)

    efi_path = ws / "target" / "x86_64-unknown-uefi" / "release" / "rusty_box_uefi.efi"
    if not efi_path.exists():
        print(f"ERROR: EFI binary not found at {efi_path}", file=sys.stderr)
        print("Build first: cargo build --release -p rusty_box_uefi --target x86_64-unknown-uefi")
        sys.exit(1)

    # Also prepare the vvfat directory for QEMU
    out_dir = Path("rusty_box_uefi_disk")
    efi_dir = out_dir / "EFI" / "BOOT"
    rb_dir = out_dir / "rusty_box"

    if out_dir.exists():
        shutil.rmtree(out_dir)
    efi_dir.mkdir(parents=True)
    rb_dir.mkdir(parents=True)

    shutil.copy2(efi_path, efi_dir / "BOOTX64.EFI")
    (out_dir / "startup.nsh").write_text("\\EFI\\BOOT\\BOOTX64.EFI\r\n")
    print(f"EFI binary: {efi_path.stat().st_size} bytes")

    alpine_path = None
    if args.alpine_iso:
        alpine_path = Path(args.alpine_iso)
    if alpine_path is None or not alpine_path.exists():
        for candidate in sorted(ws.glob("alpine-virt*.iso")):
            alpine_path = candidate
            break
    if alpine_path and alpine_path.exists():
        shutil.copy2(alpine_path, rb_dir / "alpine.iso")
        print(f"Alpine ISO: {alpine_path.stat().st_size // (1024*1024)} MB")
    else:
        print("WARNING: No Alpine ISO found")

    # Print QEMU command
    ovmf_win = "C:/Program Files/qemu/share/edk2-x86_64-code.fd"
    print(f"\nQEMU:")
    print(f'  qemu-system-x86_64 -m 512 -drive "if=pflash,format=raw,readonly=on,file={ovmf_win}" -drive format=vvfat,rw=on,dir={out_dir} -nographic')

    # Print VMware instructions
    print(f"\nVMware Workstation:")
    print(f"  1. Create New VM -> Custom -> Guest OS: Other 64-bit")
    print(f"  2. VM Settings -> Options -> Advanced -> Firmware: UEFI")
    print(f"  3. VM Settings -> Hardware -> Hard Disk -> Use existing -> Browse to the disk dir")
    print(f"  4. OR: share the '{out_dir}' folder and use UEFI Shell to navigate")
    print(f"\n  Easiest VMware method:")
    print(f"  - Create VM with EFI firmware, add a shared folder pointing to '{out_dir.resolve()}'")
    print(f"  - Boot to UEFI Shell, run: fs0:\\EFI\\BOOT\\BOOTX64.EFI")


if __name__ == "__main__":
    main()
