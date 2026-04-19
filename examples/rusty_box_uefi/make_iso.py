#!/usr/bin/env python3
"""
Prepare a UEFI boot directory for the Rusty Box emulator.

Creates a directory structure that QEMU's -drive format=vvfat can boot from,
or that can be used to create a FAT partition with external tools.

Usage:
    python make_iso.py [--alpine-iso PATH] [--output DIR]

Then run with QEMU:
    qemu-system-x86_64 \\
      -drive if=pflash,format=raw,readonly=on,file=OVMF_CODE.fd \\
      -drive format=vvfat,rw=on,dir=rusty_box_uefi_disk \\
      -nographic
"""

import argparse
import shutil
import sys
from pathlib import Path


def find_workspace_root():
    d = Path(__file__).resolve().parent
    while d != d.parent:
        if (d / "Cargo.toml").exists() and (d / "rusty_box").is_dir():
            return d
        d = d.parent
    return None


def main():
    parser = argparse.ArgumentParser(description="Prepare Rusty Box UEFI boot directory")
    parser.add_argument("--alpine-iso", help="Path to Alpine Linux ISO")
    parser.add_argument("--output", default="rusty_box_uefi_disk", help="Output directory")
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

    out = Path(args.output)
    efi_dir = out / "EFI" / "BOOT"
    rb_dir = out / "rusty_box"

    # Clean and create
    if out.exists():
        shutil.rmtree(out)
    efi_dir.mkdir(parents=True)
    rb_dir.mkdir(parents=True)

    # Copy EFI binary
    shutil.copy2(efi_path, efi_dir / "BOOTX64.EFI")
    print(f"EFI binary: {efi_path.stat().st_size} bytes (BIOS ROMs embedded)")

    # Copy Alpine ISO
    alpine_path = None
    if args.alpine_iso:
        alpine_path = Path(args.alpine_iso)
    if alpine_path is None or not alpine_path.exists():
        for candidate in sorted(ws.glob("alpine-virt*.iso")):
            alpine_path = candidate
            break
    if alpine_path and alpine_path.exists():
        shutil.copy2(alpine_path, rb_dir / "alpine.iso")
        print(f"Alpine ISO: {alpine_path.stat().st_size // (1024*1024)} MB ({alpine_path.name})")
    else:
        print("WARNING: No Alpine ISO found. The UEFI app will fail at runtime.")
        print("  Pass --alpine-iso PATH or place alpine-virt-*.iso in workspace root")

    # Startup script
    (out / "startup.nsh").write_text("\\EFI\\BOOT\\BOOTX64.EFI\r\n")

    print(f"\nBoot directory: {out}/")
    for p in sorted(out.rglob("*")):
        if p.is_file():
            print(f"  {p.relative_to(out)}  ({p.stat().st_size} bytes)")

    ovmf_win = "C:/Program Files/qemu/share/edk2-x86_64-code.fd"
    ovmf_linux = "/usr/share/OVMF/OVMF_CODE.fd"
    print(f"\nTo run:")
    print(f"  Windows:")
    print(f'    qemu-system-x86_64 -drive "if=pflash,format=raw,readonly=on,file={ovmf_win}" -drive format=vvfat,rw=on,dir={out} -nographic')
    print(f"  Linux:")
    print(f"    qemu-system-x86_64 -drive if=pflash,format=raw,readonly=on,file={ovmf_linux} -drive format=vvfat,rw=on,dir={out} -nographic")


if __name__ == "__main__":
    main()
