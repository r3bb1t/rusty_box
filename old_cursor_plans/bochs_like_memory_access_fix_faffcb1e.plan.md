---
name: bochs_like_memory_access_fix
overview: "Adjust the BIOS/VGABIOS text-output fix plan to match Bochs’ memory model: use direct host pointers only when allowed (v2h/getHostMemAddr), otherwise route through handler-aware physical-page read/write so VGA text writes are observed."
todos:
  - id: wire-mem-sys-ptr
    content: Wire a CPU execution-scoped pointer to `BxMemC` (Bochs-style access gate) alongside existing `mem_ptr`/`io_bus`, and set/clear it around `cpu_loop_n*`.
    status: completed
  - id: string-ops-vga-mmio
    content: "Replace the current `mem_read_*`/`mem_write_*` raw-RAM-only behavior with Bochs-like “host-pointer-or-fallback”: use `BxMemC::get_host_mem_addr` when available; otherwise use `read_physical_page`/`write_physical_page` so VGA/MMIO/ROM handlers run (fixes string ops and other memory ops transitively)."
    status: completed
  - id: run-dlxlinux-verify
    content: Re-run the `dlxlinux` example and confirm BIOS/VGABIOS text is visible (VGA text mode and/or always-on port `0xE9`).
    status: in_progress
isProject: false
---

## What Bochs actually does (source of truth)
- Bochs string fast paths (`cpu/faststring.cc`, `cpu/io.cc`) first try `v2h_read_byte`/`v2h_write_byte` and explicitly bail out if host access is vetoed (returns `NULL`).
- When host access is vetoed (MMIO/VGA/ROM/handlers), Bochs falls back to handler-aware paths (`readPhysicalPage`/`writePhysicalPage`). Example: DMA helpers in `memory/memory.cc` call `getHostMemAddr()` and if `NULL` they loop over `readPhysicalPage`/`writePhysicalPage`.

## What’s wrong in Rust right now
- Many instruction implementations ultimately call `BxCpuC::mem_read_*`/`mem_write_*` (including `string.rs`). Those helpers *only* use `mem_ptr` (raw RAM) and return 0 / drop writes when out-of-range.
- This bypasses the already-Bochs-like `BxMemC::get_host_mem_addr` veto rules and `read_physical_page`/`write_physical_page` handler dispatch, so writes to VGA text aperture never reach the VGA memory handlers.

## Corrected approach (mirrors Bochs)
- **CPU memory access must be “host-pointer-or-fallback”**:
  - Try to obtain a direct host slice via `BxMemC::get_host_mem_addr(paddr, rw, cpus)` (Bochs `getHostMemAddr`).
  - If it returns `Some(slice)`, do the read/write directly on that slice (fast path).
  - If it returns `None` (veto/MMIO/ROM/handlers), perform the access via `BxMemC::read_physical_page` / `BxMemC::write_physical_page` so device handlers run.
- To do this without borrow-checker overhead and while staying `no_std + alloc`, **wire an execution-scoped raw pointer to `BxMemC` into the CPU**, just like the existing `io_bus: Option<NonNull<BxDevicesC>>` pattern.

## Files likely touched
- `rusty_box/src/cpu/cpu.rs`: add `mem_bus` pointer, set/clear during `cpu_loop_n*`.
- `rusty_box/src/cpu/string.rs`: keep using `mem_read_*`/`mem_write_*`, but those helpers will become handler-aware.
- `rusty_box/src/cpu/*`: no need to rewrite every opcode if `mem_read_*`/`mem_write_*` becomes correct (most call sites already funnel through them).

## Data flow after fix
```mermaid
flowchart TD
CpuInstr[CpuInstruction] --> MemHelpers[mem_read/mem_write]
MemHelpers -->|try_getHostMemAddr| HostPtr[DirectHostSlice]
MemHelpers -->|veto| PhysPage[read_physical_page/write_physical_page]
PhysPage --> Handlers[MemoryHandlers(VGA/ROM/MMIO)]
Handlers --> VgaText[VGA_Text_Buffer]
VgaText --> TermGui[TermGui_render]
```

## Acceptance criteria
- Running `rusty_box/examples/dlxlinux.rs` prints **either port `0xE9` output and/or visible VGA BIOS text** in the terminal (Term GUI), without adding `std` dependencies to the core crate.
- No new `static mut`, no mutexes; raw pointers are execution-scoped and cleared after the loop.