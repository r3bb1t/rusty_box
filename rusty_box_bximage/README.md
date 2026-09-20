# rusty_box_bximage

`rusty_box_bximage` creates blank disk images for Rusty Box, compatible with the images Bochs' `bximage` tool makes. It parses human sizes like `20G` and `512M`, computes Bochs-style hard-disk geometry, and writes raw flat hard-disk and floppy images. Its only dependency is `thiserror`: it does not depend on the emulator core. It never attaches or boots an image; callers such as `rusty_box_gui` decide what to do with the file.

## Flat images, and when they are sparse

An image is written by seeking to its last 512 bytes and writing them as zeros. The crate sets no sparse flag, so whether the region before those bytes takes disk space is up to the host filesystem. Filesystems that leave a seeked-over region unallocated, such as ext4 and APFS, produce a sparse file. NTFS allocates the whole image, so a `20G` disk created on Windows takes 20 GiB of disk space immediately.

## Sizes

`ImageSize::parse` accepts a whole number followed by an optional unit, with no space between them. Leading and trailing whitespace is trimmed.

| Input | Unit |
| --- | --- |
| `512` (no unit) | MiB |
| `512M`, `512MB`, `512MiB` (any case) | MiB (1024 × 1024 bytes) |
| `20G`, `20GB`, `20GiB` (any case) | GiB (1024³ bytes) |

`MB` and `GB` are binary units here, the same as `MiB` and `GiB`. An empty string is `BxImageError::MissingSize`. A decimal (`1.5G`), zero, a sign, an inner space (`20 G`) or any other unit is `InvalidSize`. A number too large for a `u64` byte count is `SizeOverflow`.

Sizes can also be built directly with `ImageSize::mib`, `ImageSize::gib` and `ImageSize::from_bytes`. `DEFAULT_HARD_DISK_SIZE` is 20 GiB, which `rusty_box_gui` uses when a disk is created without a size.

## Hard-disk geometry

`calculate_hard_disk_geometry(size, sector_size)` follows these rules:

- Every disk has 16 heads and 63 sectors per track (`HARD_DISK_HEADS`, `HARD_DISK_SECTORS_PER_TRACK`).
- The cylinder count is the requested size divided by 16 × 63 × the sector size, rounded down. The image is therefore truncated to whole cylinders and usually comes out slightly smaller than requested. A `20G` request gives 41,610 cylinders and 21,474,754,560 bytes, not 21,474,836,480.
- Requests under 10 MiB (`MIN_HARD_DISK_SIZE`) fail with `HardDiskTooSmall`.
- The cylinder count must stay below 2^24 (`BOCHS_MAX_CYLINDERS`), or the call fails with `CylinderOverflow`. With 512-byte sectors, a request of 8064 GiB or more fails.
- The sector size is `SectorSize::Bytes512`, `Bytes1024` or `Bytes4096`. `SectorSize::from_bytes` rejects any other value with `UnsupportedSectorSize`.

## Floppies

`FloppyFormat::ALL` lists the ten bximage floppy sizes. `FloppyFormat::parse_label` matches the labels below, ignoring case. `friendly_label` returns a description such as `1.44 MB (3.5" HD)`.

| Label | Sectors | Bytes |
| --- | --- | --- |
| `160k` | 320 | 163,840 |
| `180k` | 360 | 184,320 |
| `320k` | 640 | 327,680 |
| `360k` | 720 | 368,640 |
| `720k` | 1,440 | 737,280 |
| `1.2M` | 2,400 | 1,228,800 |
| `1.44M` | 2,880 | 1,474,560 |
| `1.68M` | 3,360 | 1,720,320 |
| `1.72M` | 3,444 | 1,763,328 |
| `2.88M` | 5,760 | 2,949,120 |

## Writing an image

There are two ways to write an image:

- **To a file.** `create_flat_hard_disk(path, size, sector_size, policy)` and `create_floppy(path, format, policy)`. With `ExistingFilePolicy::CreateNew`, an existing path is refused with `AlreadyExists`. With `ExistingFilePolicy::Truncate`, it is replaced.
- **To any `Write + Seek`.** `create_flat_hard_disk_to_writer(display_path, &mut writer, size, sector_size)` and `create_floppy_to_writer(display_path, &mut writer, format)`. An in-memory `Cursor<Vec<u8>>` works, and it is how the browser build of `rusty_box_gui` produces its downloads. `display_path` only names the image in the returned `CreatedImage` and in error messages.

Every call returns a `CreatedImage` with these fields:

- `path`
- `bytes`: the final size.
- `kind`: `HardDisk { geometry }` or `Floppy { format }`.
- `bochsrc_line`: the matching Bochs configuration line, either `ata0-master: type=disk, path="…", mode=flat` (with `, sect_size=N` added for sectors other than 512 bytes) or `floppya: image="…", status=inserted`.

## Examples

Sizes and geometry, which touch no file:

```rust
use rusty_box_bximage::{calculate_hard_disk_geometry, BxImageError, ImageSize, SectorSize};

fn main() -> Result<(), BxImageError> {
    assert_eq!(ImageSize::parse("20G")?, ImageSize::gib(20));
    assert_eq!(ImageSize::parse("512")?, ImageSize::mib(512));
    for rejected in ["1.5G", "0", "20 G", "20T"] {
        assert!(matches!(
            ImageSize::parse(rejected),
            Err(BxImageError::InvalidSize { .. })
        ));
    }

    let geometry = calculate_hard_disk_geometry(ImageSize::gib(20), SectorSize::Bytes512)?;
    assert_eq!(geometry.cylinders, 41_610);
    assert_eq!((geometry.heads, geometry.sectors_per_track), (16, 63));
    assert_eq!(geometry.final_bytes, 21_474_754_560);

    assert!(matches!(
        calculate_hard_disk_geometry(ImageSize::mib(9), SectorSize::Bytes512),
        Err(BxImageError::HardDiskTooSmall { .. })
    ));
    assert!(calculate_hard_disk_geometry(ImageSize::gib(8063), SectorSize::Bytes512).is_ok());
    assert!(matches!(
        calculate_hard_disk_geometry(ImageSize::gib(8064), SectorSize::Bytes512),
        Err(BxImageError::CylinderOverflow { .. })
    ));
    Ok(())
}
```

Images written to memory:

```rust
use rusty_box_bximage::{
    create_flat_hard_disk_to_writer, create_floppy_to_writer, BxImageError, CreatedImageKind,
    FloppyFormat, ImageSize, SectorSize,
};
use std::io::Cursor;

fn main() -> Result<(), BxImageError> {
    let mut disk = Cursor::new(Vec::new());
    let created =
        create_flat_hard_disk_to_writer("c.img", &mut disk, ImageSize::mib(10), SectorSize::Bytes512)?;
    // 20 cylinders x 16 heads x 63 sectors x 512 bytes.
    assert_eq!(created.bytes, 10_321_920);
    assert_eq!(disk.get_ref().len() as u64, created.bytes);
    assert!(matches!(
        &created.kind,
        CreatedImageKind::HardDisk { geometry } if geometry.cylinders == 20
    ));
    assert_eq!(
        created.bochsrc_line,
        r#"ata0-master: type=disk, path="c.img", mode=flat"#
    );

    let mut floppy = Cursor::new(Vec::new());
    let created = create_floppy_to_writer("boot.img", &mut floppy, FloppyFormat::M1_44)?;
    assert_eq!(floppy.get_ref().len(), 1_474_560);
    assert_eq!(created.bochsrc_line, r#"floppya: image="boot.img", status=inserted"#);
    Ok(())
}
```

Images written to files. This example is compiled but not run by the doc tests, because it would write a 20 GiB file into the working directory:

```rust,no_run
use rusty_box_bximage::{
    create_flat_hard_disk, create_floppy, BxImageError, ExistingFilePolicy, FloppyFormat,
    ImageSize, SectorSize,
};

fn main() -> Result<(), BxImageError> {
    let disk = create_flat_hard_disk(
        "c.img",
        ImageSize::parse("20G")?,
        SectorSize::Bytes512,
        ExistingFilePolicy::CreateNew,
    )?;
    println!("{} bytes; bochsrc: {}", disk.bytes, disk.bochsrc_line);

    let floppy = create_floppy("boot.img", FloppyFormat::M1_44, ExistingFilePolicy::Truncate)?;
    println!("bochsrc: {}", floppy.bochsrc_line);
    Ok(())
}
```

## Verification

This README is the crate's documentation (`#![doc = include_str!("../README.md")]` in `src/lib.rs`), so its examples are doc tests:

```powershell
cargo test --release -p rusty_box_bximage          # unit tests and doc tests
cargo test --release -p rusty_box_bximage --doc    # the examples above only
```
