#!/usr/bin/env bash
# Boot regression gate.
#
# The unit suites cannot catch bugs that only surface deep in a real guest.
# The SMM GPR-restore defect fixed in feb28c5 is the reference case: it left
# every unit test and the Alpine boot green while corrupting Windows 7 far
# enough in that only a full boot showed it.
#
# Two checks, both headless and bounded:
#
#   alpine   Boot the Alpine ISO and require the serial console to reach the
#            login prompt. Catches gross regressions fast (~1 min).
#
#   win7     Boot a Windows 7 install ISO far enough to diff the BIOS message
#            *sequence* against upstream Bochs. A healthy run ends
#              IDE time out / Booting from 07c0:0000 /
#              int13_harddisk: function 02 / function 08 x3
#            A broken one replaces one `function 08` with two
#              *** int 15h function AX=0000, BX=0000 not yet supported!
#            lines. Deterministic and about two minutes; no GUI needed.
#
# Usage:
#   scripts/boot_gate.sh alpine
#   scripts/boot_gate.sh win7 /path/to/win7.iso
#   scripts/boot_gate.sh all  /path/to/win7.iso
#
# Exits non-zero on the first failing check.

set -u

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EMU="$REPO/target/release/rusty_box_gui.exe"
[ -x "$EMU" ] || EMU="$REPO/target/release/rusty_box_gui"
BIOS="$REPO/cpp_orig/bochs/bochs/bios/BIOS-bochs-latest"
VGABIOS="$REPO/cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin"
ALPINE_ISO="$REPO/alpine-virt-3.24.1-x86_64.iso"
WORK="${TMPDIR:-${TEMP:-/tmp}}/rusty_box_boot_gate"
mkdir -p "$WORK"

die() { echo "boot_gate: $*" >&2; exit 2; }
[ -x "$EMU" ] || die "no release binary — run: cargo build --release -p rusty_box_gui"
[ -r "$BIOS" ] || die "missing BIOS at $BIOS"

run_alpine() {
    local out="$WORK/alpine.txt"
    rm -f "$out"
    echo "== alpine: booting to login prompt =="
    # The guest idles at the login prompt forever, so cap the wall clock and
    # judge the run purely by what reached the serial console.
    timeout 400 "$EMU" --no-config --display headless --no-sync-slowdown \
        --bios "$BIOS" --vga-bios "$VGABIOS" \
        --cdrom "$ALPINE_ISO" --boot cdrom \
        --memory-mib 256 --host-memory-mib 256 --pci \
        --ips 300000000 --log-level warn > "$out" 2>&1

    if grep -qE "Kernel panic|not syncing|invalid opcode|general protection" "$out"; then
        echo "FAIL: guest faulted"
        grep -nE "Kernel panic|not syncing|invalid opcode|general protection" "$out" | head
        return 1
    fi
    if ! grep -q "localhost login" "$out"; then
        echo "FAIL: never reached the login prompt (see $out)"
        tail -20 "$out"
        return 1
    fi
    echo "PASS: alpine reached the login prompt"
    return 0
}

run_win7() {
    local iso="$1" out="$WORK/win7.txt"
    [ -r "$iso" ] || die "win7 ISO not readable: $iso"
    rm -f "$out"
    echo "== win7: diffing the BIOS message sequence =="
    timeout 400 "$EMU" --no-config --display headless \
        --bios "$BIOS" --vga-bios "$VGABIOS" \
        --cdrom "$iso" --boot cdrom \
        --memory-mib 2048 --host-memory-mib 2048 \
        --ips 300000000 --max-instructions 2500000000 \
        --log-level info > "$out" 2>&1

    if grep -q "int 15h function AX=0000" "$out"; then
        echo "FAIL: 'int 15h function AX=0000 not yet supported' appeared —"
        echo "      this is the signature of the SMM/CPU-state corruption class."
        return 1
    fi

    local tail_seq
    tail_seq=$(grep -oE "IDE time out|Booting from [0-9a-f:]+|int13_harddisk: function [0-9]+" "$out" | tail -6 | tr '\n' '|')
    local want="IDE time out|Booting from 07c0:0000|int13_harddisk: function 02|int13_harddisk: function 08|int13_harddisk: function 08|int13_harddisk: function 08|"
    if [ "$tail_seq" != "$want" ]; then
        echo "FAIL: BIOS sequence diverged from upstream Bochs"
        echo "  want: $want"
        echo "  got:  $tail_seq"
        return 1
    fi
    echo "PASS: win7 BIOS sequence matches upstream Bochs"
    return 0
}

case "${1:-}" in
    alpine) run_alpine ;;
    win7)   run_win7 "${2:?usage: boot_gate.sh win7 <win7.iso>}" ;;
    all)    run_alpine && run_win7 "${2:?usage: boot_gate.sh all <win7.iso>}" ;;
    *)      die "usage: boot_gate.sh {alpine|win7 <iso>|all <iso>}" ;;
esac
