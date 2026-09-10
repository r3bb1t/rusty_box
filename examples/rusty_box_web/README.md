# Rusty Box Web — x86 Emulator in the Browser

A small egui app that runs the Rusty Box emulator (a Rust port of Bochs) in a
web browser through WebAssembly. It opens a launcher with two choices:

- **Boot DLX Linux** — the DLX Linux 10 MB hard disk image embedded in the
  module, 32 MB of guest RAM, booting from the hard disk.
- **Load Alpine Linux ISO** — a browser file picker (accepts `.iso` and
  `.img`); the uploaded image is attached as a CD-ROM and booted with 256 MB of
  guest RAM.

`rusty_box_gui` has a separate browser build with the full VMware-style shell
(`cd rusty_box_gui && trunk serve --release --port 8080`; see
[rusty_box_gui/README.md](../../rusty_box_gui/README.md)). This crate is the
smaller of the two: one launcher page, and the only one that embeds DLX.

## Prerequisites

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

The build embeds three files with `include_bytes!` (`src/app.rs`). They are not
in the repository (`/cpp_orig` and `/dlxlinux` are gitignored), so the build
fails until they exist at these paths, relative to the workspace root:

| File | Source |
|------|--------|
| `cpp_orig/bochs/bochs/bios/BIOS-bochs-latest` | a Bochs source checkout, `bios/BIOS-bochs-latest` (128 KB) |
| `cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin` | a Bochs source checkout (32 KB; padded to whole 512-byte blocks at run time) |
| `dlxlinux/hd10meg.img` | [Bochs DLX Linux disk image](https://bochs.sourceforge.io/diskimages.html) (10.2 MiB) |

## Run in the browser

```bash
cd examples/rusty_box_web
trunk serve --release --port 8080
```

Open http://localhost:8080 and choose **Boot DLX Linux** or **Load Alpine Linux
ISO**. Nothing is built until you choose.

Input is keyboard only; there is no mouse. Typed text is sent as scancodes, plus
Enter, Tab, Backspace, Space, Esc, F1–F12, the arrow keys, Home/End,
PgUp/PgDn and Ins/Del.

## Run natively

```bash
cargo run --release -p rusty_box_web
```

This opens the same app in a native window, running the same single-threaded
loop. The Alpine file picker only exists in the browser build; natively, the
button logs a warning and does nothing, so only DLX can be booted. For a
threaded desktop front end, use `cargo run --release -p rusty_box_gui`.

## Frame loop

The app owns the emulator and the display directly and runs cooperatively on
one thread (`src/app.rs`, `WasmEmulatorApp::ui`). On each frame:

1. It calls `emu.step(RunBudget::Instructions(50_000))` repeatedly until the
   frame has made 200,000 units of progress (`FRAME_BUDGET`). It stops early
   when `outcome.is_terminal()` (guest power-off, CPU shutdown, a stop request
   or an engine fault), when `outcome.progress.stalled()`, or on an error.
2. It renders the VGA state with `emu.display().render_into(&mut self.display)`.
3. It feeds this frame's keyboard events to the guest as scancodes.
4. It uploads the framebuffer as an egui texture, scaled to a whole multiple.

`Emulator::step` runs the guest, ticks the devices and syncs the A20 line, then
returns control, so a single-threaded host keeps its event loop. Its
`BatchOutcome` says how far the guest got and why the step returned.

## Build for deployment

```bash
cd examples/rusty_box_web
trunk build --release
```

Trunk writes the site to `examples/rusty_box_web/dist/`:

- `index.html`
- `rusty_box_web.js`
- `rusty_box_web_bg.wasm`

`Trunk.toml` sets `filehash = false`, so these names do not change between
builds, and `index.html` asks Trunk for `wasm-opt` level 2. The `.wasm` is
larger than the 10.2 MiB DLX disk image it embeds. Serve the `dist/` files with
any static HTTP server.

`cargo xtask ci` neither builds nor checks this crate: its wasm steps cover only
the `rusty_box` library and `rusty_box_gui`. Build it yourself after changing
anything it uses.
