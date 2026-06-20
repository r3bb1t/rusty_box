# rusty_box_bximage

`rusty_box_bximage` is a small bximage-compatible disk image creation library for Rusty Box. It parses human sizes like `20G` and `512M`, computes Bochs-style hard-disk geometry, and creates sparse flat image files without depending on the emulator core.

## Examples

```rust
use rusty_box_bximage::{
    create_flat_hard_disk, create_floppy, ExistingFilePolicy, FloppyFormat, ImageSize, SectorSize,
};

let disk = create_flat_hard_disk(
    "c.img",
    ImageSize::gib(20),
    SectorSize::Bytes512,
    ExistingFilePolicy::CreateNew,
)?;

let floppy = create_floppy(
    "boot.img",
    FloppyFormat::M1_44,
    ExistingFilePolicy::CreateNew,
)?;
# Ok::<(), rusty_box_bximage::BxImageError>(())
```

Hard disks are create-only raw flat images today. The crate intentionally does not attach or boot images; callers such as `rusty_box_gui` decide how to use the generated file.

## Verification

```powershell
cargo test -p rusty_box_bximage
```
