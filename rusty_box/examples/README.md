# Examples

## Required Files

The examples need BIOS and disk image files that are **not included** in this repository.

| File | Purpose | Where to get it |
|------|---------|----------------|
| `BIOS-bochs-latest` | System BIOS (128 KB) | [Bochs source](https://bochs.sourceforge.io/) `bios/BIOS-bochs-latest` |
| `VGABIOS-lgpl-latest.bin` | VGA BIOS | [Bochs source](https://bochs.sourceforge.io/) `bios/VGABIOS-lgpl-latest.bin` |
| `dlxlinux/hd10meg.img` | DLX Linux hard disk (10 MB) | [Bochs DLX Linux](https://bochs.sourceforge.io/diskimages.html) |
| `alpine-virt-*.iso` | Alpine Linux ISO (optional) | [alpinelinux.org/downloads](https://alpinelinux.org/downloads/) (Virtual x86) |

Place them in any of these locations (searched in order):
- `binaries/bios/`
- Project root
- Parent directory

## Available Examples

| Example | Features | Description |
|---------|----------|-------------|
| `dlxlinux` | `std` | Headless DLX Linux boot (text output to stdout) |
| `dlxlinux_egui` | `std,gui-egui` | DLX Linux with graphical VGA display |
| `rusty_box_egui` | `std,gui-egui` | Unified GUI — boots DLX or Alpine (set `RUSTY_BOX_BOOT=alpine`) |
| `alpine_direct` | `std` | Alpine Linux headless (BIOS or direct kernel boot) |

## Running

```bash
# DLX Linux — headless
RUSTY_BOX_HEADLESS=1 cargo run --release --example dlxlinux --features std

# DLX Linux — GUI
cargo run --release --example rusty_box_egui --features "std,gui-egui"

# Alpine Linux — headless BIOS boot
RUSTY_BOX_HEADLESS=1 MAX_INSTRUCTIONS=3500000000 cargo run --release --example alpine_direct --features std
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `RUSTY_BOX_HEADLESS=1` | Skip GUI, output to stdout |
| `RUSTY_BOX_BOOT=alpine` | Boot Alpine instead of DLX (egui example) |
| `MAX_INSTRUCTIONS=N` | Stop after N instructions |
| `ALPINE_ISO=/path/to.iso` | Path to Alpine ISO |
| `RUST_LOG=debug` | Enable debug logging |

### Linux GUI Dependencies

On Ubuntu/Debian, install display server libraries before building with `gui-egui`:

```bash
sudo apt install -y libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxinerama-dev libxcursor-dev libxi-dev libgl-dev
```
