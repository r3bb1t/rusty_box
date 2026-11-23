#!/bin/sh
set -eu
if [ "$#" -ne 1 ]; then
    echo 'usage: sh build.sh OUTPUT_DIRECTORY' >&2
    exit 2
fi
fixture_dir=$(dirname "$0")
mkdir -p "$1"
for program in compute filter; do
    gcc -m32 -march=i386 -O2 -Wall -Wextra -Werror -ffreestanding -fno-stack-protector \
        -fno-asynchronous-unwind-tables -fno-pie -no-pie -static -nostdlib \
        -Wl,--build-id=none "$fixture_dir/start.S" "$fixture_dir/$program.c" -o "$1/$program"
done
