#!/usr/bin/env bash
# Boot a nested KVM guest on the Arm board, interactively, in one go.
#
# Alpine Linux runs on the board's Cortex-A57 with EL2 (virtualization
# on); its KVM comes up in nVHE mode with the in-kernel vGIC, and QEMU
# inside it runs a second Alpine under KVM. The serial console is this
# terminal: when the script below has started the nested guest, what you
# type goes to the guest.
#
#   Ctrl-]       quit the emulator (from anywhere)
#   Ctrl-A x     quit the nested QEMU, back to the host Alpine's shell
#
# What it does, each step skipped when its result is already there:
#   1. downloads Alpine's aarch64 virt ISO and checks its SHA-512;
#   2. takes the kernel, initramfs and file tree out of it;
#   3. downloads QEMU and its dependencies from Alpine's repositories,
#      checking each index's signature and each package's checksum;
#   4. builds an ext4 disk holding the ISO's tree (the host's boot media),
#      the packages, and the nested guest's kernel, initramfs and ISO;
#   5. builds the runner (cargo, release);
#   6. boots it.
#
# Usage: scripts/nested_kvm.sh [--ips N] [--host-only] [--rebuild] [--no-build]
#   --ips N      emulated instructions a second (default 1000000000; the
#                board's 50M makes the nested guest crawl: a nested tick
#                costs tens of thousands of instructions)
#   --host-only  stop at the host Alpine's shell, QEMU installed, nested
#                guest not started
#   --rebuild    download and build every asset again
#   --no-build   use the runner already built
# Environment: RUSTY_BOX_NESTED_DIR (default <repo>/target/nested) holds
# the downloads and the disk; HOST_RAM_MIB (1024), NESTED_RAM_MIB (256).
#
# Needs: cargo, curl, python3 (with venv, unless bsdtar is installed),
# openssl, mke2fs (e2fsprogs 1.43 or later), sha512sum. Tested on Linux.
set -euo pipefail

ALPINE_VERSION=3.23.7
ALPINE_BRANCH=v3.23
ISO_NAME=alpine-virt-$ALPINE_VERSION-aarch64.iso
ISO_URL=https://dl-cdn.alpinelinux.org/alpine/$ALPINE_BRANCH/releases/aarch64/$ISO_NAME

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WORK=${RUSTY_BOX_NESTED_DIR:-$REPO/target/nested}
HOST_RAM_MIB=${HOST_RAM_MIB:-1024}
NESTED_RAM_MIB=${NESTED_RAM_MIB:-256}
IPS=1000000000
NESTED=yes
BUILD=yes
REBUILD=no

while [ $# -gt 0 ]; do
  case $1 in
    --ips) IPS=$2; shift 2 ;;
    --host-only) NESTED=no; shift ;;
    --rebuild) REBUILD=yes; shift ;;
    --no-build) BUILD=no; shift ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
done

say() { printf '\033[1m== %s\033[0m\n' "$*" >&2; }
need() {
  command -v "$1" >/dev/null 2>&1 || { echo "missing: $1 ($2)" >&2; exit 1; }
}
need cargo "https://rustup.rs"
need curl "your package manager"
need python3 "your package manager"
need openssl "your package manager"
need mke2fs "e2fsprogs"
need sha512sum "coreutils"

ASSETS="python3 -I $REPO/scripts/nested_kvm_assets.py"
mkdir -p "$WORK"
cd "$WORK"
if [ "$REBUILD" = yes ]; then
  rm -rf iso-tree apks index Image disk.img .disk-done
fi

# 1. The ISO.
if [ ! -f "$ISO_NAME" ]; then
  say "downloading $ISO_NAME"
  curl -fL --retry 3 -o "$ISO_NAME.part" "$ISO_URL"
  mv "$ISO_NAME.part" "$ISO_NAME"
fi
if [ ! -f "$ISO_NAME.sha512" ]; then
  curl -fsSL --retry 3 -o "$ISO_NAME.sha512" "$ISO_URL.sha512"
fi
say "checking $ISO_NAME"
sha512sum -c "$ISO_NAME.sha512" >/dev/null

# 2. Its tree, kernel and initramfs.
if [ ! -f iso-tree/boot/vmlinuz-virt ]; then
  say "extracting the ISO"
  rm -rf iso-tree && mkdir -p iso-tree
  if command -v bsdtar >/dev/null 2>&1; then
    bsdtar -xf "$ISO_NAME" -C iso-tree
  else
    if [ ! -x venv/bin/python ]; then
      python3 -m venv venv
      venv/bin/pip install --quiet --disable-pip-version-check pycdlib
    fi
    venv/bin/python -I "$REPO/scripts/nested_kvm_assets.py" iso-tree "$ISO_NAME" iso-tree
  fi
  chmod -R u+w iso-tree
fi
if [ ! -f Image ]; then
  $ASSETS image iso-tree/boot/vmlinuz-virt Image
fi

# 3. QEMU and its dependencies.
if [ ! -f apks/.done ]; then
  say "downloading QEMU's packages"
  rm -rf apks index
  $ASSETS fetch iso-tree/boot/initramfs-virt index apks qemu-system-aarch64
  touch apks/.done
fi

# 4. The host's disk: its boot media, the packages, the nested guest's files.
if [ ! -f .disk-done ]; then
  say "building disk.img"
  rm -rf media disk.img
  cp -a iso-tree media
  mkdir -p media/extra media/nested
  cp apks/*.apk media/extra/
  cp Image media/nested/Image
  cp iso-tree/boot/initramfs-virt media/nested/initramfs-virt
  cp "$ISO_NAME" media/nested/alpine.iso
  SIZE_MIB=$(( $(du -sm media | cut -f1) + 256 ))
  mke2fs -q -t ext4 -L alpine -d media disk.img "${SIZE_MIB}M"
  rm -rf media
  touch .disk-done
fi

# 5. The runner.
RUNNER=$REPO/target/release/examples/linux_virt
if [ "$BUILD" = yes ]; then
  say "building the runner"
  (cd "$REPO" && cargo build --release -p rusty_box_aarch64 --features std --example linux_virt)
fi
[ -x "$RUNNER" ] || { echo "no runner at $RUNNER (drop --no-build)" >&2; exit 1; }

# 6. Boot. The script logs in, installs QEMU from the disk and starts the
# nested guest; then the terminal is the console.
NESTED_QEMU="qemu-system-aarch64 -M virt,gic-version=2 -accel kvm -cpu host -m $NESTED_RAM_MIB -nographic -no-reboot -kernel /media/vda/nested/Image -initrd /media/vda/nested/initramfs-virt -drive if=virtio,format=raw,readonly=on,file=/media/vda/nested/alpine.iso -append 'console=ttyAMA0 modules=loop,squashfs,sd-mod,usb-storage'"
STEPS=(
  --until 'login: ' --type-serial 'root\r' --until '# '
  --type-serial 'apk add --quiet --no-progress --no-network --force-non-repository /media/vda/extra/*.apk && echo QEMU-READY\r'
  --until 'QEMU-READY' --until '# '
)
if [ "$NESTED" = yes ]; then
  STEPS+=(--type-serial "$NESTED_QEMU\r")
fi

say "booting (Ctrl-] quits; the first prompt takes a minute or two)"
SAVED_TTY=
restore_tty() {
  if [ -n "$SAVED_TTY" ]; then
    stty "$SAVED_TTY"
    SAVED_TTY=
  fi
}
if [ -t 0 ]; then
  SAVED_TTY=$(stty -g)
  trap restore_tty EXIT
  stty raw -echo
fi
# The runner's end-of-run report goes to stderr; it is held until the
# terminal is itself again, so its lines are not staggered by raw mode.
REPORT=$WORK/last-run-report.txt
STATUS=0
"$RUNNER" --cpu cortex-a57 --virtualization --ips "$IPS" \
  --kernel Image --initrd iso-tree/boot/initramfs-virt \
  --append "console=ttyAMA0 modules=loop,squashfs,sd-mod,usb-storage,ext4 alpine_dev=vda:ext4" \
  --disk disk.img --ram-mib "$HOST_RAM_MIB" \
  "${STEPS[@]}" --interactive 2> "$REPORT" || STATUS=$?
restore_tty
echo
if [ "$STATUS" -eq 0 ]; then
  grep -E '^(stop:|ticks:|console script)' "$REPORT" || true
else
  cat "$REPORT"
fi
say "the runner's full report: $REPORT"
exit "$STATUS"
