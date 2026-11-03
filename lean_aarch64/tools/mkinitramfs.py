#!/usr/bin/env python3
"""Build the initramfs the login runs boot: Alpine's minirootfs as a
gzip-compressed newc cpio, with busybox init as /init and a getty on the
PL011 (ttyAMA0), so the boot ends at Alpine's `login:` prompt, where `root`
logs in with no password.

    python3 -I mkinitramfs.py alpine-minirootfs-3.23.7-aarch64.tar.gz initramfs.cpio.gz

The archive's members keep their modes, owners and times; /dev/console is
added (the kernel opens it for /init's standard streams), and /etc/inittab
is replaced, and root's password is emptied in /etc/shadow (the minirootfs
locks it). Nothing is read from or written to the host's filesystem but
the two named files.
"""
import gzip
import io
import sys
import tarfile

INITTAB = b"""::sysinit:/bin/mount -t proc proc /proc
::sysinit:/bin/mount -t sysfs sysfs /sys
::sysinit:/bin/mount -t devtmpfs devtmpfs /dev
ttyAMA0::respawn:/sbin/getty -L 0 ttyAMA0 vt100
::ctrlaltdel:/sbin/reboot
::shutdown:/bin/umount -a -r
"""


class Cpio:
    def __init__(self):
        self.out = io.BytesIO()
        self.ino = 1

    def entry(self, name, mode, data=b"", uid=0, gid=0, mtime=0, rdev=(0, 0)):
        name_b = name.encode() + b"\0"
        hdr = "070701" + "".join(
            f"{v:08x}"
            for v in (
                self.ino, mode, uid, gid, 1, mtime, len(data),
                0, 0, rdev[0], rdev[1], len(name_b), 0,
            )
        )
        self.ino += 1
        self.out.write(hdr.encode() + name_b)
        self.out.write(b"\0" * (-(110 + len(name_b)) % 4))
        self.out.write(data)
        self.out.write(b"\0" * (-len(data) % 4))

    def finish(self):
        self.entry("TRAILER!!!", 0)
        return self.out.getvalue()


def main(src, dst):
    c = Cpio()
    seen = set()
    with tarfile.open(src, "r:gz") as t:
        for m in t.getmembers():
            name = m.name.lstrip("./").rstrip("/")
            if not name or name in seen:
                continue
            seen.add(name)
            perm = m.mode & 0o7777
            if name == "etc/inittab":
                c.entry(name, 0o100644, INITTAB, m.uid, m.gid, int(m.mtime))
            elif name == "etc/shadow":
                lines = t.extractfile(m).read().split(b"\n")
                lines = [b"root::" + l.split(b":", 2)[2] if l.startswith(b"root:") else l for l in lines]
                c.entry(name, 0o100000 | perm, b"\n".join(lines), m.uid, m.gid, int(m.mtime))
            elif m.isdir():
                c.entry(name, 0o040000 | perm, b"", m.uid, m.gid, int(m.mtime))
            elif m.issym():
                c.entry(name, 0o120000 | 0o777, m.linkname.encode(), m.uid, m.gid, int(m.mtime))
            elif m.islnk():
                data = t.extractfile(t.getmember(m.linkname)).read()
                c.entry(name, 0o100000 | perm, data, m.uid, m.gid, int(m.mtime))
            elif m.isreg():
                c.entry(name, 0o100000 | perm, t.extractfile(m).read(), m.uid, m.gid, int(m.mtime))
            elif m.ischr():
                c.entry(name, 0o020000 | perm, b"", m.uid, m.gid, int(m.mtime), (m.devmajor, m.devminor))
    if "dev" not in seen:
        c.entry("dev", 0o040755)
    c.entry("dev/console", 0o020600, rdev=(5, 1))
    c.entry("init", 0o120777, b"/bin/busybox")
    with open(dst, "wb") as f:
        f.write(gzip.compress(c.finish(), 9, mtime=0))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
