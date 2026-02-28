# Rusty Box Web — x86 Emulator in the Browser

Run the Rusty Box x86 emulator (a Rust port of Bochs) directly in your web browser via WebAssembly.

Boots DLX Linux 1.3.89 from a 10 MB hard disk image with full VGA text output, BIOS POST, LILO boot loader, and Linux kernel startup — all at ~3 MIPS in the browser.

## Prerequisites

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

## Run in Browser (WASM)

```bash
cd rusty_box_web
trunk serve --release --port 8080
```

Open http://localhost:8080 in your browser. The emulator starts automatically:
1. BIOS POST (VGA BIOS, ATA detection)
2. LILO boot loader
3. Linux 1.3.89 kernel decompression and startup

## Run Natively (Desktop with egui GUI)

```bash
cargo run --release --example dlxlinux_egui --features "std,gui-egui"
```

This opens a native window with the same emulator. The native build runs on a
dedicated thread and achieves higher IPS (~15 MIPS).

## Architecture

The WASM build uses cooperative single-threaded execution:

```
eframe::App::update() called each frame (~60 fps)
  1. emu.step_batch(50_000)     — run CPU instructions
  2. emu.update_display(&mut d) — render VGA text to pixel framebuffer
  3. upload framebuffer texture  — display via egui
  4. process keyboard input      — push scancodes to emulator
```

No threads, no `Arc<Mutex<>>` — the emulator and display are owned directly
by the app struct. The `step_batch()` method handles:
- Device ticking (PIT, PIC, keyboard, VGA)
- PIC interrupt delivery to the CPU
- HLT time-advancement (Bochs-style BX_TICKN acceleration)
- A20 line synchronization

## Embedded Assets

Binary assets are compiled into the WASM module via `include_bytes!`:
- **BIOS**: `cpp_orig/bochs/bios/BIOS-bochs-latest` (128 KB)
- **VGA BIOS**: `cpp_orig/bochs/bios/VGABIOS-lgpl-latest.bin` (38 KB)
- **Disk**: `dlxlinux/hd10meg.img` (10.2 MB)

Total WASM size: ~9 MB (uncompressed), ~4 MB with gzip.

## Build for Deployment

```bash
cd rusty_box_web
trunk build --release
```

Static files are generated in `dist/`:
- `index.html`
- `rusty_box_web.js`
- `rusty_box_web_bg.wasm`

Serve these with any static HTTP server.
