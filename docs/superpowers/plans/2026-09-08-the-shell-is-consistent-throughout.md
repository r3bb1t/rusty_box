# The Shell Is Consistent Throughout — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Every pane, button and control in the desktop shell matches the style established on the Home tab at `f184706`.

**Architecture:** Appearance only, in `rusty_box_gui/src/app.rs`, working within the palette and the `ActionTileWeight` vocabulary that commit introduced. No behaviour changes.

**Status:** written from a live audit of all four tabs at `f184706`. NOT yet executed.

## Global Constraints

- Read `CLAUDE.md` first. **Edit with Edit/Write, never shell heredocs, `sed -i`, or Python**; **never run `cargo fmt`** — `app.rs` is ~5,000 lines and a reformat would make the diff unreviewable; never `let _ = <Result>`; comments state today's invariant, never history.
- **Do NOT use the LSP tools** — they hang indefinitely here. rust-analyzer also emits stale false-positive errors (seven times on 2026-09-08); reproduce every diagnostic with a real cargo command.
- **The gate cannot verify this.** It builds this crate only as `cargo build -p rusty_box_gui --release`, which proves nothing renders. You must screenshot before and after via the `egui` MCP tools (never desktop screenshots).
- `cargo xtask ci` before the commit; never pipe it, redirect to a log and grep for `FAILED`. A gate run can be KILLED (not failed) by another project's `Stop-Process` on `xtask.exe`: log stops mid-step, `FAILED` 0, exit `0xffffffff` — retry once.
- **Never stage** `ROADMAP.md`, `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked `docs/superpowers/specs/2026-08-22-*.md`. Explicit `git add <path>` only. **Leave `stash@{0}` alone.**
- Commit messages end with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- Only ONE GUI may hold inspection port 5719 and a WHP partition at a time; close yours before rebuilding (do not rename the running binary — that left a 12 MB orphan once). `egui_mcp` keeps a stale connection after a GUI closes and refuses `attach` with "already connected" until `disconnect` is called.

## Launch command

```bash
EGUI_INSPECTION=1 cargo run --release -p rusty_box_gui -- --no-config --display egui \
  --engine whp \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --boot cdrom \
  --memory-mib 512 --host-memory-mib 512 --ips 300000000
```

Attach to 127.0.0.1:5719. The "Power On VM" **Button** is the one with `role: Button` (three nodes match that text). Alpine reaches `login:` in ~30 s on WHP.

---

## The audit — what was actually seen at `f184706`

Home was polished; the other three panes were not. Every item below was observed in a rendered frame, not inferred.

### 1. Tab selection has two different treatments

Home's selected tab draws a **cyan underline**. Console, Hardware and Images draw the selected tab as a **boxed button with a visible border** — egui's default selected-button frame showing through. One idiom, applied everywhere: the underline.

### 2. The hero returns on Images

`f184706` deliberately removed the hero from Home. **Images still has one** — "Disk Images" + subtitle, in a card that is only half the width of the form card beneath it, leaving a ragged right edge. Hardware uses a third treatment: a title *inside* the content card, `Hardware Summary | Memory`, with an ad-hoc pipe separator.

Give all three panes the same page-header shape Home now uses.

### 3. Primary/secondary button weight is used only on Home

`ActionTileWeight::{Primary, Secondary}` exists but the other panes ignore it:
- **Images:** `Create image` — that pane's primary action — renders as a plain grey button.
- **Hardware:** `Save settings to config file` — same.

Each pane's main verb should carry the primary weight; the rest secondary.

### 4. Cards do not fill, and float

- **Hardware:** the Devices list is a rounded card that ends mid-air at y≈375 with its last item at y≈345; the right-hand settings card ends at y≈400 leaving ~350 px of dead space below.
- **Images:** hero card half-width, form card full-width.

Decide a single rule for how a pane's cards occupy their column and apply it to all three.

### 5. Field rows are ragged

- **Images:** the `Path` input starts at x≈303, the `Size` input at x≈297 — visibly misaligned; `Overwrite existing file` renders with no visible checkbox glyph and an odd indent.
- **Hardware:** `Guest memory` / `Host memory` / `Memory block` inputs are boxes of differing widths, and the values do not sit on a shared left edge.

Labels and inputs should share one grid within a pane.

### 6. The Console has no empty state

Powered off, the Console tab is **entirely blank** — two empty regions divided by a hairline, no text at all. It should say what is true and what to do (the guest is powered off; the toolbar's Power On starts it), in `TEXT_MUTED`, centred in the VGA region.

### 7. Toolbar glyphs are mixed

`▶ Power On`, `■ Power Off`, `↻ Restart VM`, `▣ Hardware`, `□ New Image` — five glyph styles from three different families. Pick one convention or drop the glyphs from the non-verb buttons.

---

## Known limitation, NOT a styling defect

**The status bar's IPS reads `---` while a guest runs on WHP.** The hypervisor path publishes `ips = 0` to the bridge, so there is no rate to display. It is deliberately rendered muted so it cannot pass for a real value. Fixing it means giving the bridge a rate for hypervisor runs — a `RunSummary`/bridge change, out of scope for an appearance task. Do not "fix" it by inventing a number.

Related: `RunSummary.instructions_executed` is `Option<u64>` and `None` on the hypervisor, because a hardware processor retires instructions the host does not count (commit `60f86c3`).

## Suggested task split

1. **Tabs, headers and buttons** (items 1–3) — the vocabulary items; one commit.
2. **Layout: card fill and field grids** (items 4–5) — the measurement items; one commit, and the one most likely to need a second rendered pass, because a first fix that compiles can still mis-measure (that happened on `f184706`).
3. **Console empty state and toolbar glyphs** (items 6–7) — small, independent.

Each task: screenshot before, change, screenshot after, put both in the report, and abandon any item that looks worse rendered than described.
