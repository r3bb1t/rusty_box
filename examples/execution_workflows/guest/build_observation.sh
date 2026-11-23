#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo 'usage: sh build_observation.sh STATIC_BUSYBOX OUTPUT_DIR' >&2
    exit 2
fi

guest_dir=$(dirname "$0")
mkdir -p "$2"
build() {
    output=$1
    source=$2
    shift 2
    gcc -m32 -O2 -Wall -Wextra -Werror -ffreestanding -fno-stack-protector \
        -fno-asynchronous-unwind-tables -nostdlib -Wl,--build-id=none \
        "$@" "$guest_dir/i386_start.S" "$guest_dir/$source" -o "$output"
}

build "$2/rb-observe" observe_exec.c -fno-pie -no-pie -static -Wl,-Ttext-segment=0x09000000
build "$2/program_sum" program_sum.c -fno-pie -no-pie -static
build "$2/program_text" program_text.c -fPIE -static-pie -Wl,--no-dynamic-linker
python3 "$guest_dir/mkinitramfs.py" "$1" "$2/observation.cpio.gz" "$2"
