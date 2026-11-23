#!/usr/bin/env python3
"""Build the guest control environment from a static i386 BusyBox."""

from pathlib import Path
import gzip
import stat
import sys


def newc(entries):
    data = bytearray()
    for inode, (name, mode, body, major, minor) in enumerate(entries, 1):
        name = name.encode("ascii") + b"\0"
        nlink = 2 if stat.S_ISDIR(mode) else 1
        fields = [inode, mode, 0, 0, nlink, 0, len(body), 0, 0, major, minor, len(name), 0]
        data += b"070701" + "".join(f"{field:08x}" for field in fields).encode("ascii")
        data += name
        data += bytes(-len(data) % 4)
        data += body
        data += bytes(-len(data) % 4)
    return data


def main():
    if len(sys.argv) not in (3, 4):
        raise SystemExit("usage: mkinitramfs.py STATIC_BUSYBOX OUTPUT.cpio.gz [OBSERVATION_BIN_DIR]")
    busybox = Path(sys.argv[1]).read_bytes()
    if busybox[:7] != b"\x7fELF\x01\x01\x01" or int.from_bytes(busybox[18:20], "little") != 3:
        raise SystemExit("expected an i386 BusyBox ELF")
    init = Path(__file__).with_name("init").read_bytes()
    directory = stat.S_IFDIR | 0o755
    executable = stat.S_IFREG | 0o755
    symlink = stat.S_IFLNK | 0o777
    entries = [
        ("bin", directory, b"", 0, 0),
        ("dev", directory, b"", 0, 0),
        ("tmp", directory, b"", 0, 0),
        ("dev/console", stat.S_IFCHR | 0o600, b"", 5, 1),
        ("dev/null", stat.S_IFCHR | 0o666, b"", 1, 3),
        ("bin/busybox", executable, busybox, 0, 0),
        *[(f"bin/{name}", symlink, b"busybox", 0, 0)
          for name in ("cat", "chmod", "mkfifo", "mv", "rm", "sh", "stty", "uname", "wc")],
        ("init", executable, init, 0, 0),
    ]
    if len(sys.argv) == 4:
        entries.append(("proc", directory, b"", 0, 0))
        for name in ("rb-observe", "program_sum", "program_text"):
            path = Path(sys.argv[3]) / name
            if not 0 < path.stat().st_size <= 1024 * 1024:
                raise SystemExit(f"unexpected guest fixture size for {name}")
            entries.append((f"bin/{name}", executable, path.read_bytes(), 0, 0))
    entries.append(("TRAILER!!!", 0, b"", 0, 0))
    Path(sys.argv[2]).write_bytes(gzip.compress(newc(entries), mtime=0))


if __name__ == "__main__":
    main()
