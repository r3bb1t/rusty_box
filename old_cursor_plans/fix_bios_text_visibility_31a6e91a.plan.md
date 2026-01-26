---
name: Fix BIOS text visibility
overview: BIOS likely writes VGA text using string ops (STOS/MOVS). Your current `cpu/string.rs` memory helpers write via raw `mem_ptr`, bypassing BxMemC memory handlers, so VGA never sees writes to 0xB0000/0xB8000 and the terminal stays blank. We’ll implement a hybrid fast-path that routes VGA/MMIO/ROM through BxMemC handlers while keeping direct RAM writes fast.
todos:
  - id: wire-mem-sys-ptr
    content: Add execution-scoped NonNull<BxMemC> pointer in CPU and set/clear in cpu_loop_n.
    status: pending
  - id: string-ops-vga-mmio
    content: Route string op memory access through BxMemC for VGA/MMIO/ROM while keeping mem_ptr fast path for RAM.
    status: pending
  - id: run-dlxlinux-verify
    content: Run dlxlinux again and confirm BIOS/VGABIOS text appears in terminal.
    status: pending
isProject: false
---

## Root cause (why you see no BIOS text)

- In [`rusty_box/src/cpu/string.rs`](rusty_box/src/cpu/string.rs), helpers `mem_read_*`/`mem_write_*` directly access `self.mem_ptr` (raw host RAM) and **never call `BxMemC::read_physical_page` / `write_physical_page`**.
- Bochs relies on the memory system to **veto** direct host mapping for VGA (`0xA0000..0xBFFFF`) so device memory handlers run (`vgacore.cc: DEV_register_memory_handlers(0xa0000..0xbffff)`).
- Result: BIOS can execute `STOSW` into `0xB8000` (or `0xB0000`) but the VGA device’s `vga_mem_write_handler` never fires → GUI sees no changes.

## Chosen approach (per your answer)

- **Hybrid fast path**:
- Keep raw `mem_ptr` direct access for normal RAM.
- Route VGA (`0xA0000..0xBFFFF`) and ROM/MMIO through `BxMemC` so registered handlers are honored.

## Implementation steps

### 1) Give CPU access to the memory subsystem during execution

- Add a new CPU field: `mem_sys: Option<NonNull<BxMemC>>`.
- Set/clear it alongside `mem_ptr` inside `BxCpuC::cpu_loop_n(...)`.
- This mirrors how we already wire `io_bus`.

### 2) Rework string-op memory helpers to consult handlers for VGA/MMIO

- Update `mem_read_byte/word/dword` and `mem_write_byte/word/dword` in [`rusty_box/src/cpu/string.rs`](rusty_box/src/cpu/string.rs):
- If address in `0xA0000..0xC0000` (covers `B0000/B8000` text apertures) → call `BxMemC::read_physical_page` / `write_physical_page` with a small stack buffer.
- Otherwise use current direct `mem_ptr` fast path.
- Keep it `no_std` friendly (stack buffers + `alloc` only).

### 3) Ensure writes hit VGA handlers

- Because VGA registers handlers for `0xA0000..0xBFFFF`, `BxMemC::write_physical_page` will invoke `vga_mem_write_handler`, setting `vga_mem_updated` and `text_dirty`.
- Then `Emulator::update_gui()` calls `vga.update()` and `TermGui::text_update(...)` renders the window.

### 4) Validate with DLX example

- Re-run `cargo run --release --example dlxlinux`.
- Expected: BIOS/VGABIOS text should start appearing once firmware writes the text buffer.

## Files to change

- [`rusty_box/src/cpu/cpu.rs`](rusty_box/src/cpu/cpu.rs)
- [`rusty_box/src/cpu/builder.rs`](rusty_box/src/cpu/builder.rs)
- [`rusty_box/src/cpu/string.rs`](rusty_box/src/cpu/string.rs)