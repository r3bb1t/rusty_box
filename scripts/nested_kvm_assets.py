"""The files scripts/nested_kvm.sh boots the nested KVM guest from.

    nested_kvm_assets.py image <vmlinuz-virt> <Image>
        The kernel's arm64 Image out of Alpine's EFI zboot vmlinuz (a PE
        file whose "zimg" header names a gzip payload), for the board's
        Linux boot protocol, which takes a plain Image.

    nested_kvm_assets.py iso-tree <image.iso> <out dir>
        Every file of an ISO 9660 image by its Rock Ridge names (pycdlib).

    nested_kvm_assets.py fetch <initramfs-virt> <work dir> <out dir> <package>...
        Alpine aarch64 packages and every dependency, checked as apk checks
        them: each repository index's RSA signature against Alpine's keys
        (taken from the ISO's own initramfs), and each package's control
        section against the checksum the signed index gives it.

Downloads go through curl, so a proxy and CA bundle set for curl apply.
"""
import base64
import gzip
import hashlib
import io
import os
import re
import struct
import subprocess
import sys
import tarfile
import zlib

ALPINE_BRANCH = 'v3.23'
MIRROR = f'https://dl-cdn.alpinelinux.org/alpine/{ALPINE_BRANCH}'
REPOS = ('main', 'community')
ARCH = 'aarch64'


def zboot_image(vmlinuz, out):
    data = open(vmlinuz, 'rb').read()
    if data[0:2] != b'MZ' or data[4:8] != b'zimg':
        raise SystemExit(f'{vmlinuz}: not an EFI zboot image')
    offset, size = struct.unpack('<II', data[8:16])
    kind = data[24:32].rstrip(b'\0').decode()
    if kind != 'gzip':
        raise SystemExit(f'{vmlinuz}: payload is {kind}, only gzip is read')
    image = zlib.decompress(data[offset:offset + size], 31)
    if image[56:60] != b'ARM\x64':
        raise SystemExit(f'{vmlinuz}: the payload is not an arm64 Image')
    with open(out, 'wb') as f:
        f.write(image)
    print(f'{out}: {len(image)} bytes')


def iso_tree(image, out_dir):
    import pycdlib

    iso = pycdlib.PyCdlib()
    iso.open(image)
    files = 0
    try:
        for dirpath, _dirnames, filenames in iso.walk(rr_path='/'):
            local_dir = os.path.join(out_dir, dirpath.lstrip('/'))
            os.makedirs(local_dir, exist_ok=True)
            for name in filenames:
                rr = dirpath.rstrip('/') + '/' + name
                iso.get_file_from_iso(os.path.join(local_dir, name), rr_path=rr)
                files += 1
    finally:
        iso.close()
    print(files, 'files ->', out_dir)


def curl(url, dest):
    subprocess.run(['curl', '-fsSL', '--retry', '3', '-o', dest, url], check=True)


def gzip_members(data):
    """The raw bytes of each gzip member of `data`, in order."""
    members = []
    start = 0
    while start < len(data):
        d = zlib.decompressobj(31)
        d.decompress(data[start:])
        used = len(data) - start - len(d.unused_data)
        members.append(data[start:start + used])
        start += used
    return members


def initramfs_keys(path, keys_dir):
    data = gzip.open(path, 'rb').read()
    i = 0
    os.makedirs(keys_dir, exist_ok=True)
    while data[i:i + 6] == b'070701':
        fields = [int(data[i + 6 + 8 * k:i + 14 + 8 * k], 16) for k in range(13)]
        name_len, size = fields[11], fields[6]
        name = data[i + 110:i + 110 + name_len - 1].decode()
        i = (i + 110 + name_len + 3) & ~3
        body = data[i:i + size]
        i = (i + size + 3) & ~3
        if name.startswith('etc/apk/keys/') and name.endswith('.pub'):
            with open(os.path.join(keys_dir, os.path.basename(name)), 'wb') as f:
                f.write(body)
        if name == 'TRAILER!!!':
            break
    keys = sorted(os.listdir(keys_dir))
    if not keys:
        raise SystemExit(f'{path}: no apk keys found')
    return keys


def signed_index(repo, work, keys_dir):
    """The APKINDEX text of `repo`, its signature verified."""
    path = os.path.join(work, f'APKINDEX.{repo}.tar.gz')
    curl(f'{MIRROR}/{repo}/{ARCH}/APKINDEX.tar.gz', path)
    data = open(path, 'rb').read()
    members = gzip_members(data)
    sig_tar = tarfile.open(fileobj=io.BytesIO(gzip.decompress(members[0])))
    sig = [m for m in sig_tar.getmembers() if m.name.startswith('.SIGN.RSA.')][0]
    key = sig.name[len('.SIGN.RSA.'):]
    sig_path = os.path.join(work, f'{repo}.sig')
    signed_path = os.path.join(work, f'{repo}.signed')
    open(sig_path, 'wb').write(sig_tar.extractfile(sig).read())
    open(signed_path, 'wb').write(b''.join(members[1:]))
    subprocess.run(['openssl', 'dgst', '-sha1', '-verify', os.path.join(keys_dir, key),
                    '-signature', sig_path, signed_path], check=True,
                   stdout=subprocess.DEVNULL)
    index_tar = tarfile.open(fileobj=io.BytesIO(gzip.decompress(b''.join(members[1:]))))
    return index_tar.extractfile('APKINDEX').read().decode(), key


def parse(text, repo, packages, providers):
    for block in text.strip().split('\n\n'):
        p = {'repo': repo}
        for line in block.splitlines():
            k, _, v = line.partition(':')
            p[k] = v
        name = p['P']
        packages[name] = p
        for token in p.get('p', '').split():
            providers.setdefault(re.split('[<>=~]', token)[0], name)


def resolve(roots, packages, providers):
    wanted, todo = set(), list(roots)
    while todo:
        token = todo.pop()
        if token.startswith('!'):
            continue
        name = re.split('[<>=~]', token)[0]
        pkg = name if name in packages else providers.get(name)
        if pkg is None:
            raise SystemExit(f'nothing provides {token}')
        if pkg in wanted:
            continue
        wanted.add(pkg)
        todo.extend(packages[pkg].get('D', '').split())
    return sorted(wanted)


def fetch(initramfs, work, out, roots):
    os.makedirs(work, exist_ok=True)
    os.makedirs(out, exist_ok=True)
    keys_dir = os.path.join(work, 'keys')
    print('keys:', initramfs_keys(initramfs, keys_dir))
    packages, providers = {}, {}
    for repo in REPOS:
        text, key = signed_index(repo, work, keys_dir)
        print(f'{repo}: index signed by {key}, verified')
        parse(text, repo, packages, providers)
    wanted = resolve(roots, packages, providers)
    print(len(wanted), 'packages')
    for name in wanted:
        p = packages[name]
        file = f"{name}-{p['V']}.apk"
        dest = os.path.join(out, file)
        curl(f"{MIRROR}/{p['repo']}/{ARCH}/{file}", dest)
        control = gzip_members(open(dest, 'rb').read())[1]
        checksum = 'Q1' + base64.b64encode(hashlib.sha1(control).digest()).decode()
        if checksum != p['C']:
            raise SystemExit(f'{file}: checksum {checksum} is not the index\'s {p["C"]}')
    print('all', len(wanted), 'checksums match their signed index')


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    command, args = sys.argv[1], sys.argv[2:]
    if command == 'image' and len(args) == 2:
        zboot_image(*args)
    elif command == 'iso-tree' and len(args) == 2:
        iso_tree(*args)
    elif command == 'fetch' and len(args) >= 4:
        fetch(args[0], args[1], args[2], args[3:])
    else:
        raise SystemExit(__doc__)


if __name__ == '__main__':
    main()
