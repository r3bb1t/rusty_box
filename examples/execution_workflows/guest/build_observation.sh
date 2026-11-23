#!/bin/sh
set -eu

if [ "$#" -ne 2 ] && { [ "$#" -ne 3 ] || [ "$3" != --host-only ]; }; then
    echo 'usage: sh build_observation.sh STATIC_BUSYBOX OUTPUT_DIR [--host-only]' >&2
    exit 2
fi

guest_dir=$(dirname "$0")
busybox=$1
output_dir=$2
shift 2
mkdir -p "$output_dir"
build() {
    output=$1
    source=$2
    shift 2
    gcc -m32 -O2 -Wall -Wextra -Werror -ffreestanding -fno-stack-protector \
        -fno-asynchronous-unwind-tables -nostdlib -Wl,--build-id=none \
        "$@" "$guest_dir/i386_start.S" "$guest_dir/$source" -o "$output"
}

build "$output_dir/program_sum" program_sum.c -fno-pie -no-pie -static
build "$output_dir/program_text" program_text.c -fPIE -static-pie -Wl,--no-dynamic-linker
if [ "$#" -eq 0 ]; then
    build "$output_dir/rb-observe" observe_exec.c -fno-pie -no-pie -static -Wl,-Ttext-segment=0x09000000
fi
python3 "$guest_dir/mkinitramfs.py" "$busybox" "$output_dir/observation.cpio.gz" "$output_dir" "$@"
