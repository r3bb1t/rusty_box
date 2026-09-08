# The Shell Is One Tree — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the desktop shell's four chrome layers with a sidebar tree
and a VM bar, on a named type/spacing/colour scale that a reviewer can check.

**Architecture:** A new `rusty_box_gui/src/shell/` module owns the design
vocabulary (`theme.rs`, `widgets.rs`) and the two new chrome surfaces
(`sidebar.rs`, `vm_bar.rs`), which draw from borrowed data and **return an
action rather than mutating the app**. Navigation state collapses from two
independent fields into one `Destination` value (`destination.rs`) — the only
pure logic here, and the only thing that gets unit tests.

**Tech Stack:** Rust, egui/eframe 0.35 (`egui::Panel`, `ui.scope_builder`,
`ctx.style_mut_of`), `egui_mcp` inspection on `127.0.0.1:5719`.

**Spec:** `docs/superpowers/specs/2026-09-08-the-shell-is-one-tree-design.md`.
Supersedes `docs/superpowers/plans/2026-09-08-the-shell-is-consistent-throughout.md`.

## Global Constraints

- **Read `CLAUDE.md` first.** Edit with the Edit/Write tools — **never** shell
  heredocs, `sed -i`, or Python. A scripted edit has damaged this tree twice.
- **Never run `cargo fmt`** — it rewrites 169 files over your edits.
- **Do NOT use the LSP tools.** They hang indefinitely in this workspace, and
  rust-analyzer emits stale false-positive errors (seven times on 2026-09-08).
  Reproduce every diagnostic with a real `cargo` command.
- **Never `let _ = <Result>`.** Propagate with `?` or match every variant.
- **No `dyn` in signatures** (doctrine R8). Use generics and `impl Trait` in
  argument position only.
- **Comments state today's invariant, never history.** No "this used to…", no
  before/after, nothing named `*_new` or `*_v2`.
- **Release builds only.** Every cargo command carries `--release`.
- **Never stage** `ROADMAP.md`,
  `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked
  `docs/superpowers/specs/2026-08-22-*.md` files — they are someone else's
  work. Use explicit `git add <path>`, never `git add -A`. **Leave `stash@{0}`
  alone.**
- Stay on `wip/atom-execctx`. Do not create branches.
- Commit messages end with:
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- **Do not touch `WebShellApp`** (its own `nav_button` at `app.rs:3618`, its own
  `draw_library` at `app.rs:3214`, its own toolbar). It shares `ShellPage`,
  `ShellChrome`, `shell_should_draw_library` and `VmLibraryEntry`; all four
  survive this change. Keep it compiling.
- **Do not touch the `#[cfg(target_os = "android")]` blocks' behaviour.** The
  android target cannot be compiled here, so changes there are unverifiable.
- **The gate cannot see rendering.** `cargo xtask ci` builds this crate only as
  `cargo build -p rusty_box_gui --release`. Every task screenshots through the
  `egui` MCP tools — never desktop screenshots.
- Only ONE process may hold inspection port 5719 and a WHP partition. Close
  your GUI before rebuilding; do not rename a running binary (that left a 12 MB
  orphan once). `egui_mcp` keeps a stale connection after a GUI exits and
  refuses `attach` with "already connected" until `disconnect` is called.
- `cargo xtask ci` runs only before the **final** commit. Never pipe it —
  redirect to a log and grep for `FAILED`. A run can be **killed, not failed**,
  by another project's `Stop-Process` on `xtask.exe`: the log stops mid-step,
  `FAILED` is 0, exit is `0xffffffff`. Retry once before blaming your change.

## Launch command for every screenshot step

```bash
EGUI_INSPECTION=1 cargo run --release -p rusty_box_gui -- --no-config --display egui \
  --engine whp \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --boot cdrom \
  --memory-mib 512 --host-memory-mib 512 --ips 300000000
```

Then `attach` to `127.0.0.1:5719` with a generous `timeout_secs`, `screenshot`,
`query_tree`. The shell starts **Stopped**. Alpine reaches `login:` in ~30 s on
WHP. `wait_for "login:"` matches only the **serial** label — the VGA region is
a texture, not text.

## File structure

| file | responsibility |
|---|---|
| `rusty_box_gui/src/shell/mod.rs` | declares submodules, re-exports their items |
| `rusty_box_gui/src/shell/theme.rs` | palette, type + spacing scale, `configure_shell_style`, `shell_card_frame` |
| `rusty_box_gui/src/shell/widgets.rs` | `page_header`, `field_row`, `action_tile*`, `status_dot`, `status_text`, `hairline_*`, `home_fact`, `metadata_text`, `ShellStateBadge` |
| `rusty_box_gui/src/shell/destination.rs` | `ShellPage`, `Destination`, `SidebarAction`, `VmBarAction` + tests |
| `rusty_box_gui/src/shell/sidebar.rs` | `VmLibraryEntry`, `draw_sidebar` |
| `rusty_box_gui/src/shell/vm_bar.rs` | `VmBarState`, `draw_vm_bar` |
| `rusty_box_gui/src/app.rs` | the app struct, the pages, and action → effect wiring |

---

### Task 1: The shell module, its theme and its scale

Moves the palette out of `app.rs` and gives the design its named constants. No
pixel changes yet — this task must render **identically**.

**Files:**
- Create: `rusty_box_gui/src/shell/mod.rs`
- Create: `rusty_box_gui/src/shell/theme.rs`
- Modify: `rusty_box_gui/src/lib.rs` (declare the module)
- Modify: `rusty_box_gui/src/app.rs:29-54` (delete the constants), `:758-789`
  (delete `configure_shell_style` and `shell_card_frame`), and its `use` block

**Interfaces:**
- Consumes: nothing
- Produces: `crate::shell::theme::{BG_BASE, BG_PANEL, BG_CARD, STROKE_HAIRLINE,
  TEXT_PRIMARY, TEXT_MUTED, ACCENT_CYAN, ACCENT_BLUE, ACCENT_AMBER, ACCENT_RED,
  TEXT_DISPLAY, TEXT_TITLE, TEXT_BODY, TEXT_SECONDARY, TEXT_CAPTION,
  SPACE_ITEM, SPACE_GROUP, SPACE_CARD, SPACE_PAGE, configure_shell_style,
  shell_card_frame}`

- [ ] **Step 1: Screenshot the four panes before anything changes**

Launch the GUI with the command above. `attach`, then `screenshot` on Home,
Console, Hardware and Images (click each tab in the current tab strip). Save
all four — the report compares against them. Close the GUI before building.

- [ ] **Step 2: Create the module declaration**

`rusty_box_gui/src/shell/mod.rs`:

```rust
//! The desktop shell's design vocabulary and its two chrome surfaces.
//!
//! `theme` fixes the palette and the type and spacing scale; `widgets` holds
//! the pieces every pane is built from; `destination` is where the shell is
//! pointed; `sidebar` and `vm_bar` draw from borrowed data and return what the
//! user asked for, leaving the effect to the caller.

pub(crate) mod theme;
pub(crate) mod widgets;
```

- [ ] **Step 3: Create `theme.rs` with the palette moved verbatim and the scale added**

`rusty_box_gui/src/shell/theme.rs`. Move the ten colour constants from
`app.rs:29-54` **byte for byte** — a changed digit here is invisible in review
and changes every pane:

```rust
use egui::{Color32, Stroke};

pub(crate) const BG_BASE: Color32 = Color32::from_rgb(0x0B, 0x0F, 0x14);
pub(crate) const BG_PANEL: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);
pub(crate) const BG_CARD: Color32 = Color32::from_rgb(0x17, 0x21, 0x2B);
pub(crate) const STROKE_HAIRLINE: Color32 = Color32::from_rgb(0x26, 0x34, 0x43);
pub(crate) const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE8, 0xEE, 0xF5);
pub(crate) const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x98, 0xA8);
pub(crate) const ACCENT_CYAN: Color32 = Color32::from_rgb(0x46, 0xD9, 0xC7);
pub(crate) const ACCENT_BLUE: Color32 = Color32::from_rgb(0x6A, 0xA8, 0xFF);
pub(crate) const ACCENT_AMBER: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
pub(crate) const ACCENT_RED: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x6C);

/// The shell's five type sizes. Nothing outside this list is a legal font size
/// in a pane, and two weights carry every distinction: regular, and `.strong()`.
pub(crate) const TEXT_DISPLAY: f32 = 22.0;
pub(crate) const TEXT_TITLE: f32 = 16.0;
pub(crate) const TEXT_BODY: f32 = 14.0;
pub(crate) const TEXT_SECONDARY: f32 = 12.5;
pub(crate) const TEXT_CAPTION: f32 = 11.0;

/// Vertical rhythm, in multiples of four: between items inside one group,
/// between groups, inside a card's border, and around a page's content.
pub(crate) const SPACE_ITEM: f32 = 8.0;
pub(crate) const SPACE_GROUP: f32 = 12.0;
pub(crate) const SPACE_CARD: i8 = 16;
pub(crate) const SPACE_PAGE: i8 = 20;
```

Then move `configure_shell_style` (from `app.rs:758`) and `shell_card_frame`
(from `app.rs:783`) into this file unchanged except for `pub(crate)` and
`SPACE_CARD` in the card's margin:

```rust
pub(crate) fn shell_card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(SPACE_CARD))
}
```

- [ ] **Step 4: Declare the module and import it in `app.rs`**

In `rusty_box_gui/src/lib.rs`, beside `pub mod app;`:

```rust
#[cfg(feature = "gui-egui")]
pub(crate) mod shell;
```

In `app.rs`, delete lines 29–34 and 51–54 (the constants), delete
`configure_shell_style` and `shell_card_frame`, and add near the other `use`
lines:

```rust
use crate::shell::theme::{
    configure_shell_style, shell_card_frame, ACCENT_AMBER, ACCENT_BLUE, ACCENT_CYAN, ACCENT_RED,
    BG_BASE, BG_CARD, BG_PANEL, STROKE_HAIRLINE, TEXT_MUTED, TEXT_PRIMARY,
};
```

- [ ] **Step 5: Build both shells**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean. Then the browser shell, which shares these constants:
Run: `cargo check --release -p rusty_box_gui --target wasm32-unknown-unknown`
Expected: clean. If the wasm target is not installed, say so in the report and
run `cargo check --release -p rusty_box_gui --no-default-features` instead.

- [ ] **Step 6: Screenshot and compare**

Relaunch, screenshot the same four panes. **They must be pixel-identical to
Step 1.** Any difference means a constant was mistyped — find it before
committing.

- [ ] **Step 7: Commit**

```bash
git add rusty_box_gui/src/shell/mod.rs rusty_box_gui/src/shell/theme.rs rusty_box_gui/src/lib.rs rusty_box_gui/src/app.rs
git commit -m "refactor(gui): the shell's palette and scale are a module, not a preamble

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: The widget vocabulary

Moves the shared drawing helpers into `widgets.rs` and adds the two the panes
are missing: a page header and a field row.

**Files:**
- Create: `rusty_box_gui/src/shell/widgets.rs`
- Modify: `rusty_box_gui/src/shell/mod.rs` (already declares `widgets`)
- Modify: `rusty_box_gui/src/app.rs` — delete `metadata_text` (:3721),
  `hardware_intro` (:3727), `status_dot` (:3855), `status_text` (:3862),
  `ShellStateBadge` (:3878), `hairline_below` (:3912), `hairline_above` (:3922),
  `home_fact` (:3932), `ActionTileWeight` (:3978), `ACTION_TILE_MIN_HEIGHT`
  (:3985), `action_tile` (:3987), `action_tile_enabled` (:4001),
  `action_tile_footer` (:4069), `disabled_tile` (:4088)

**Interfaces:**
- Consumes: everything Task 1 produced
- Produces: `crate::shell::widgets::{page_header, field_row,
  FIELD_LABEL_WIDTH, ActionTileWeight, ACTION_TILE_MIN_HEIGHT, action_tile,
  action_tile_enabled, status_dot, status_text, hairline_above, hairline_below,
  home_fact, metadata_text, ShellStateBadge}`

- [ ] **Step 1: Move the existing helpers unchanged**

Cut each function listed above from `app.rs` into `widgets.rs`, changing only
their visibility to `pub(crate)` and keeping every `#[cfg(...)]` attribute
exactly as it stands. `ShellStateBadge` gains a derive so the two new surfaces
can take it by value:

```rust
/// The one-word state the status strip, the sidebar and the VM bar all show,
/// with the accent that state owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct ShellStateBadge {
    pub(crate) label: &'static str,
    pub(crate) color: Color32,
}
```

`shell_state_badge` (`app.rs:3885`) **stays in `app.rs`** — it maps that file's
`ShellStatus` and must import `ShellStateBadge` from here.

- [ ] **Step 2: Put `home_fact` on the scale**

It currently uses 10.5 and 13.0, which are on no scale. In `widgets.rs`:

```rust
/// A labelled fact in a page header: a muted caption over its value.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn home_fact(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).size(TEXT_CAPTION).color(TEXT_MUTED));
        ui.label(RichText::new(value).size(TEXT_BODY).color(TEXT_PRIMARY));
    });
}
```

- [ ] **Step 3: Add the page header, replacing `hardware_intro`**

`hardware_intro` is the same shape with a different name and no scale. Delete
it and add:

```rust
/// Every pane opens with exactly this: the pane's name over one line saying
/// what it does, then the gap that separates a header from its content.
pub(crate) fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(TEXT_TITLE).strong().color(TEXT_PRIMARY));
    ui.label(RichText::new(subtitle).size(TEXT_CAPTION).color(TEXT_MUTED));
    ui.add_space(SPACE_GROUP);
}
```

- [ ] **Step 4: Add the field row**

```rust
/// The width every label column in the shell reserves, so that two inputs in
/// the same pane start on the same x whatever their labels say.
pub(crate) const FIELD_LABEL_WIDTH: f32 = 104.0;

/// One labelled control on a pane's grid. The label is allocated a fixed
/// column; the caller's widgets take the rest of the row.
pub(crate) fn field_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(FIELD_LABEL_WIDTH, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(TEXT_BODY).color(TEXT_MUTED));
            },
        );
        add_contents(ui)
    })
    .inner
}
```

- [ ] **Step 5: Point `app.rs` at the module and fix every call site**

Add to `app.rs`'s imports:

```rust
use crate::shell::widgets::{
    action_tile, action_tile_enabled, field_row, hairline_above, hairline_below, home_fact,
    metadata_text, page_header, status_dot, status_text, ActionTileWeight, ShellStateBadge,
};
```

Replace the four `hardware_intro(ui, title, body)` calls in
`draw_hardware_detail` with `page_header(ui, title, body)` — same arguments,
same order.

- [ ] **Step 6: Build**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean, and **no `dead_code` warning**. A warning here means a helper
was moved but its call site was not updated — the doctrine ratchet counts
blanket allows, so do not silence it.

- [ ] **Step 7: Screenshot Hardware and compare**

Relaunch and screenshot Hardware. The four device sections now use
`page_header`, so their subtitle drops from the default body size to 11 px.
Everything else is unchanged.

- [ ] **Step 8: Commit**

```bash
git add rusty_box_gui/src/shell/widgets.rs rusty_box_gui/src/shell/mod.rs rusty_box_gui/src/app.rs
git commit -m "refactor(gui): the pieces a pane is built from live in one place, and there is a page header

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: The destination is one value

The only task with real unit tests. `selected_vm` and `selected_page` become
one `Destination` that cannot hold a contradictory pair.

**Files:**
- Create: `rusty_box_gui/src/shell/destination.rs`
- Modify: `rusty_box_gui/src/shell/mod.rs`
- Modify: `rusty_box_gui/src/app.rs` — `ShellPage` (:435), `ShellChrome`
  (:509-552), and all 12 `ShellPage::Home` sites plus every
  `chrome.selected_page` / `chrome.selected_vm` reference in both shells
- Test: `rusty_box_gui/src/shell/destination.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing from Tasks 1–2
- Produces: `crate::shell::destination::{ShellPage, Destination, SidebarAction,
  VmBarAction}`; `ShellChrome::{destination, page(), selected_vm(), go_to()}`

- [ ] **Step 1: Write the failing tests**

`rusty_box_gui/src/shell/destination.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::{Destination, ShellPage};

    #[test]
    fn selecting_another_vm_lands_on_its_summary() {
        let at = Destination::new(0, ShellPage::Hardware);
        assert_eq!(at.select_vm(2), Destination::new(2, ShellPage::Home));
    }

    #[test]
    fn reselecting_the_shown_vm_keeps_the_page() {
        let at = Destination::new(1, ShellPage::Console);
        assert_eq!(at.select_vm(1), at);
    }

    #[test]
    fn selecting_a_page_keeps_the_vm() {
        let at = Destination::new(2, ShellPage::Home);
        assert_eq!(
            at.select_page(ShellPage::Images),
            Destination::new(2, ShellPage::Images)
        );
    }

    #[test]
    fn removing_an_earlier_profile_shifts_the_selection_down() {
        let at = Destination::new(2, ShellPage::Console);
        assert_eq!(
            at.clamped_after_removal(0, 2),
            Destination::new(1, ShellPage::Console)
        );
    }

    #[test]
    fn removing_the_selected_profile_clamps_to_the_last_remaining() {
        let at = Destination::new(2, ShellPage::Console);
        assert_eq!(
            at.clamped_after_removal(2, 2),
            Destination::new(1, ShellPage::Console)
        );
    }

    #[test]
    fn removing_the_only_other_profile_leaves_the_first() {
        let at = Destination::new(1, ShellPage::Hardware);
        assert_eq!(
            at.clamped_after_removal(1, 1),
            Destination::new(0, ShellPage::Hardware)
        );
    }

    #[test]
    fn every_page_the_tree_lists_carries_a_label() {
        assert_eq!(
            ShellPage::ALL.map(ShellPage::label),
            ["Summary", "Console", "Hardware", "Images"]
        );
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --release -p rusty_box_gui --lib destination`
Expected: FAIL — `cannot find type Destination in this scope`.

- [ ] **Step 3: Write the model**

Above the tests in the same file:

```rust
//! Where the shell is pointed, and what its two chrome surfaces ask for.

/// One of a VM's pages. The desktop tree labels `Home` "Summary" because it
/// summarises the selected VM; the browser shell's home page is a genuine home
/// — the place an ISO is uploaded — so the variant keeps the name that is true
/// in both shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellPage {
    Home,
    Console,
    Hardware,
    Images,
}

impl ShellPage {
    /// The pages a VM node lists, in the order the tree draws them.
    pub(crate) const ALL: [Self; 4] = [Self::Home, Self::Console, Self::Hardware, Self::Images];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Home => "Summary",
            Self::Console => "Console",
            Self::Hardware => "Hardware",
            Self::Images => "Images",
        }
    }
}

/// Which VM profile the shell shows, and which of its pages. One value, so the
/// pair cannot drift apart the way two independent fields can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Destination {
    vm: usize,
    page: ShellPage,
}

impl Destination {
    pub(crate) const fn new(vm: usize, page: ShellPage) -> Self {
        Self { vm, page }
    }

    pub(crate) const fn vm(self) -> usize {
        self.vm
    }

    pub(crate) const fn page(self) -> ShellPage {
        self.page
    }

    /// Moving to a different VM lands on its summary; re-selecting the VM
    /// already shown leaves the page where the user put it.
    pub(crate) fn select_vm(self, vm: usize) -> Self {
        if vm == self.vm {
            self
        } else {
            Self {
                vm,
                page: ShellPage::Home,
            }
        }
    }

    pub(crate) fn select_page(self, page: ShellPage) -> Self {
        Self { page, ..self }
    }

    /// The destination after `removed` is deleted from a library that then
    /// holds `remaining` profiles, clamped so it always names a live one.
    pub(crate) fn clamped_after_removal(self, removed: usize, remaining: usize) -> Self {
        let vm = if self.vm > removed { self.vm - 1 } else { self.vm };
        Self {
            vm: vm.min(remaining.saturating_sub(1)),
            page: self.page,
        }
    }
}

impl Default for Destination {
    fn default() -> Self {
        Self::new(0, ShellPage::Home)
    }
}

/// What a click in the sidebar asked for. The sidebar reports it; the app
/// decides what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarAction {
    Select(Destination),
    DuplicateSelected,
}

/// What a click in the VM bar asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VmBarAction {
    ToggleSidebar,
    PowerOn,
    PowerOff,
    Restart,
    ToggleSerial,
    ToggleMouseCapture,
    SendCtrlAltDel,
    ShowAbout,
    Quit,
}
```

Add `pub(crate) mod destination;` to `shell/mod.rs`.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo test --release -p rusty_box_gui --lib destination`
Expected: PASS, 7 tests.

- [ ] **Step 5: Delete `ShellPage` from `app.rs` and give `ShellChrome` the destination**

Delete the `ShellPage` enum at `app.rs:435-440` and import it instead. Replace
`ShellChrome`'s two fields:

```rust
#[derive(Debug)]
pub(crate) struct ShellChrome {
    destination: Destination,
    selected_hardware: HardwareDevice,
    vm_library: Vec<VmLibraryEntry>,
    library_filter: String,
    show_serial: bool,
    show_library: bool,
    show_about: bool,
}
```

`Default` sets `destination: Destination::default()`. Add three accessors so
call sites stay short and neither shell reaches into the field:

```rust
impl ShellChrome {
    pub(crate) fn page(&self) -> ShellPage {
        self.destination.page()
    }

    pub(crate) fn selected_vm(&self) -> usize {
        self.destination.vm()
    }

    /// Moves to a page of the VM already shown.
    pub(crate) fn go_to(&mut self, page: ShellPage) {
        self.destination = self.destination.select_page(page);
    }
}
```

- [ ] **Step 6: Update every call site in both shells**

Mechanical, and the compiler enumerates them — build, read the error list, fix,
repeat. The shapes:

- `self.chrome.selected_page = ShellPage::X;` → `self.chrome.go_to(ShellPage::X);`
- `self.chrome.selected_page == page` → `self.chrome.page() == page`
- `match self.chrome.selected_page {` → `match self.chrome.page() {`
- `self.chrome.selected_vm` (read) → `self.chrome.selected_vm()`
- `self.chrome.selected_vm = index;` → `self.chrome.destination = self.chrome.destination.select_vm(index);`
  (inside `select_profile`, `app.rs:2071`)
- In `delete_selected_profile` (`app.rs:2118`), the index fix-up becomes
  `self.chrome.destination = self.chrome.destination.clamped_after_removal(removed, self.profiles.len());`
  where `removed` is the index that was deleted and `self.profiles.len()` is
  read **after** the removal.

Do **not** use `replace_all` for these — `selected_vm` appears both as a field
read and as an assignment, and one blind pass would turn an assignment into a
call to a method that does not exist. Edit each site.

- [ ] **Step 7: Update the two chrome tests**

In `app.rs`'s `mod tests`, `shell_starts_on_home_page` becomes:

```rust
    #[test]
    fn shell_starts_on_home_page() {
        let chrome = ShellChrome::default();
        assert_eq!(chrome.page(), ShellPage::Home);
        assert_eq!(chrome.selected_vm(), 0);
    }
```

Leave `library_filter_handles_multiple_vms`,
`shell_hardware_list_starts_on_memory_device`,
`shell_library_sidebar_is_visible_by_default` and
`shell_library_sidebar_respects_visibility_toggle` as they are.

- [ ] **Step 8: Build and test**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean.
Run: `cargo test --release -p rusty_box_gui --lib`
Expected: PASS.

- [ ] **Step 9: Screenshot to prove nothing moved**

Relaunch, click through all four tabs, screenshot each. Identical to Task 2's.

- [ ] **Step 10: Commit**

```bash
git add rusty_box_gui/src/shell/destination.rs rusty_box_gui/src/shell/mod.rs rusty_box_gui/src/app.rs
git commit -m "refactor(gui): the shell points at one destination, not at two fields that can disagree

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: The sidebar tree replaces the library and the tab strip

The visible half of the redesign. After this task the desktop navigates from
one surface.

**Files:**
- Create: `rusty_box_gui/src/shell/sidebar.rs`
- Modify: `rusty_box_gui/src/shell/mod.rs`
- Modify: `rusty_box_gui/src/app.rs` — move `VmLibraryEntry` (:475-507), delete
  `NativeShellApp::draw_library` (:1060) and `draw_tab_strip` (:1198), gate
  `nav_button` (:2014) to android, edit `draw_central` (:1218) and
  `impl eframe::App for NativeShellApp` (:2301)

**Interfaces:**
- Consumes: `theme::*`, `widgets::ShellStateBadge`,
  `destination::{Destination, ShellPage, SidebarAction}`
- Produces: `crate::shell::sidebar::{VmLibraryEntry, draw_sidebar,
  SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MIN_WIDTH}`

- [ ] **Step 1: Move `VmLibraryEntry` into `sidebar.rs`**

Cut the struct and its `impl` (`app.rs:475-507`) into `rusty_box_gui/src/shell/sidebar.rs`
unchanged, making the type and its fields `pub(crate)`. Both shells import it.

- [ ] **Step 2: Write the tree row**

In `sidebar.rs`:

```rust
/// The width the sidebar opens at, and the narrowest a drag may make it.
pub(crate) const SIDEBAR_DEFAULT_WIDTH: f32 = 200.0;
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 170.0;

const ROW_HEIGHT: f32 = 24.0;
const CHILD_INDENT: f32 = 22.0;

/// One row of the tree. Selection is the shell's single idiom — a two-point
/// accent bar on the left edge over a card fill — and the row is the whole
/// click target, so a name and its indent never disagree about what was hit.
fn tree_row(
    ui: &mut egui::Ui,
    label: &str,
    indent: f32,
    selected: bool,
    trailing_dot: Option<Color32>,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 6.0, BG_CARD);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
            0.0,
            ACCENT_CYAN,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, BG_CARD.gamma_multiply(0.5));
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(indent, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(TEXT_BODY),
        if selected { TEXT_PRIMARY } else { TEXT_MUTED },
    );
    if let Some(color) = trailing_dot {
        ui.painter()
            .circle_filled(rect.right_center() - egui::vec2(10.0, 0.0), 3.5, color);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
```

- [ ] **Step 3: Write the sidebar**

```rust
/// The desktop's only navigation: every VM profile, with the selected one
/// expanded into its pages. Draws from borrowed data and reports what was
/// clicked; the caller owns the consequences.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_sidebar(
    ui: &mut egui::Ui,
    entries: &[VmLibraryEntry],
    visible: &[usize],
    destination: Destination,
    filter: &mut String,
    badge: ShellStateBadge,
) -> Option<SidebarAction> {
    let mut action = None;
    egui::Panel::left("vm_sidebar")
        .resizable(true)
        .default_size(SIDEBAR_DEFAULT_WIDTH)
        .min_size(SIDEBAR_MIN_WIDTH)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                .inner_margin(egui::Margin::symmetric(8, 10)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("My computer")
                        .size(TEXT_CAPTION)
                        .color(TEXT_MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("+")
                        .on_hover_text("Duplicate this VM profile")
                        .clicked()
                    {
                        action = Some(SidebarAction::DuplicateSelected);
                    }
                });
            });
            ui.add_space(SPACE_ITEM);
            ui.add(
                egui::TextEdit::singleline(filter)
                    .desired_width(f32::INFINITY)
                    .hint_text("Search VMs"),
            );
            ui.add_space(SPACE_ITEM);

            if entries.is_empty() {
                ui.label(
                    RichText::new("No VM profiles")
                        .size(TEXT_BODY)
                        .color(TEXT_MUTED),
                );
                return;
            }

            for &index in visible {
                let is_selected_vm = destination.vm() == index;
                let dot = is_selected_vm.then_some(badge.color);
                if tree_row(ui, &entries[index].name, 8.0, is_selected_vm, dot).clicked() {
                    action = Some(SidebarAction::Select(destination.select_vm(index)));
                }
                if !is_selected_vm {
                    continue;
                }
                for page in ShellPage::ALL {
                    let on_page = destination.page() == page;
                    if tree_row(ui, page.label(), CHILD_INDENT, on_page, None).clicked() {
                        action = Some(SidebarAction::Select(destination.select_page(page)));
                    }
                }
            }
        });
    action
}
```

- [ ] **Step 4: Delete the desktop tab strip, keep android's**

In `app.rs`, delete `NativeShellApp::draw_tab_strip` entirely. Put
`#[cfg(target_os = "android")]` on `NativeShellApp::nav_button` and give it a
doc comment saying what it is for:

```rust
    /// The phone form factor's navigation. Android hides the sidebar, so on a
    /// phone this strip is the only way off the page it is drawn on.
    #[cfg(target_os = "android")]
    fn nav_button(&mut self, ui: &mut egui::Ui, page: ShellPage, label: &str) {
```

`draw_central` loses its `self.draw_tab_strip(ui);` line. Android's four
`nav_button` calls in `draw_android_console_header` are untouched.

Because android now has no tab strip on non-Console pages, add one inside
`draw_central`, gated, so that build keeps its navigation:

```rust
    fn draw_central(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.take_runtime_error_notice();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_BASE))
            .show(ui, |ui| {
                #[cfg(target_os = "android")]
                self.draw_android_tab_strip(ui);
                match self.chrome.page() {
                    ShellPage::Home => self.draw_home_page(ui),
                    ShellPage::Console => self.draw_console_page(ui, frame),
                    ShellPage::Hardware => self.draw_hardware_page(ui),
                    ShellPage::Images => self.draw_images_page(ui),
                }
            });
    }
```

`draw_android_tab_strip` is the deleted `draw_tab_strip` body, renamed and
gated `#[cfg(target_os = "android")]`. It cannot be compiled here — say so in
the report.

- [ ] **Step 5: Draw the sidebar and interpret its action**

In `impl eframe::App for NativeShellApp`, replace the library call:

```rust
        if shell_should_draw_library(&self.chrome) {
            self.draw_sidebar(ui);
        }
```

and add the method to `NativeShellApp`, replacing `draw_library`:

```rust
    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let badge = shell_state_badge(&self.runtime_status(), self.has_error_notice());
        let visible = self.chrome.visible_vm_indices();
        let action = crate::shell::sidebar::draw_sidebar(
            ui,
            &self.chrome.vm_library,
            &visible,
            self.chrome.destination,
            &mut self.chrome.library_filter,
            badge,
        );
        match action {
            None => {}
            Some(SidebarAction::DuplicateSelected) => self.duplicate_selected_profile(),
            Some(SidebarAction::Select(destination)) => {
                if destination.vm() == self.chrome.destination.vm() {
                    self.chrome.destination = destination;
                } else {
                    self.select_profile(destination.vm());
                }
            }
        }
    }
```

`select_profile` already loads the profile's config and settings, so a VM
change must go through it rather than assigning the destination directly.
Confirm `select_profile` sets the destination via `select_vm` (Task 3, Step 6)
so the page lands on `Summary`.

- [ ] **Step 6: Build**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean, no `dead_code` warning.
Run: `cargo test --release -p rusty_box_gui --lib`
Expected: PASS.

- [ ] **Step 7: Screenshot and drive the tree**

Relaunch. Screenshot: the tab strip is gone and the sidebar shows one VM
expanded into Summary / Console / Hardware / Images. `click` each child and
screenshot — the accent bar must move and the content must change. Click the
`+` and confirm a second profile appears collapsed.

- [ ] **Step 8: Commit**

```bash
git add rusty_box_gui/src/shell/sidebar.rs rusty_box_gui/src/shell/mod.rs rusty_box_gui/src/app.rs
git commit -m "feat(gui): one tree navigates the desktop shell, and selecting a VM selects its page

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: The VM bar, and the menu bar's deletion

**Files:**
- Create: `rusty_box_gui/src/shell/vm_bar.rs`
- Modify: `rusty_box_gui/src/shell/mod.rs`
- Modify: `rusty_box_gui/src/app.rs` — delete `draw_menu_bar` (:902),
  `draw_toolbar` (:992), `shell_menu_style` (:3943),
  `shell_menu_labels` (:558-561), the test
  `shell_menu_labels_omit_redundant_view_and_tabs` (:4310)

**Interfaces:**
- Consumes: `destination::VmBarAction`, `widgets::ShellStateBadge`, `theme::*`
- Produces: `crate::shell::vm_bar::{VmBarState, draw_vm_bar}`

- [ ] **Step 1: Write the bar**

`rusty_box_gui/src/shell/vm_bar.rs`:

```rust
//! The one bar above a page: what VM this is, what state it is in, and the
//! verbs that change that state.

/// What the bar needs to know to draw itself. Borrowed, so the bar cannot
/// change any of it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct VmBarState<'a> {
    pub(crate) name: &'a str,
    pub(crate) badge: ShellStateBadge,
    pub(crate) running: bool,
    pub(crate) start_pending: bool,
    pub(crate) on_console: bool,
    pub(crate) serial_shown: bool,
    pub(crate) mouse_captured: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn draw_vm_bar(ui: &mut egui::Ui, state: VmBarState<'_>) -> Option<VmBarAction> {
    let mut action = None;
    let bar = egui::Panel::top("vm_bar")
        .exact_size(36.0)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .inner_margin(egui::Margin::symmetric(10, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if ui
                    .button("☰")
                    .on_hover_text("Show or hide the VM tree")
                    .clicked()
                {
                    action = Some(VmBarAction::ToggleSidebar);
                }
                ui.label(
                    RichText::new(state.name)
                        .size(TEXT_BODY)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new(state.badge.label)
                        .size(TEXT_CAPTION)
                        .color(state.badge.color),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⋯").on_hover_text("More").clicked() {
                        action = Some(VmBarAction::ShowAbout);
                    }
                    if state.on_console {
                        if ui
                            .add_enabled(state.running, egui::Button::new("Ctrl+Alt+Del"))
                            .clicked()
                        {
                            action = Some(VmBarAction::SendCtrlAltDel);
                        }
                        let capture = if state.mouse_captured {
                            "Release mouse"
                        } else {
                            "Capture mouse"
                        };
                        if ui
                            .add_enabled(state.running, egui::Button::new(capture))
                            .clicked()
                        {
                            action = Some(VmBarAction::ToggleMouseCapture);
                        }
                        let serial = if state.serial_shown {
                            "Hide serial"
                        } else {
                            "Show serial"
                        };
                        if ui.button(serial).clicked() {
                            action = Some(VmBarAction::ToggleSerial);
                        }
                        ui.separator();
                    }
                    if ui
                        .add_enabled(state.running, egui::Button::new("↻ Restart"))
                        .clicked()
                    {
                        action = Some(VmBarAction::Restart);
                    }
                    if ui
                        .add_enabled(state.running, egui::Button::new("■ Power off"))
                        .clicked()
                    {
                        action = Some(VmBarAction::PowerOff);
                    }
                    // The one filled button in the shell.
                    let can_start = !state.running && !state.start_pending;
                    let power_on = egui::Button::new(
                        RichText::new("▶ Power on").strong().color(BG_BASE),
                    )
                    .fill(ACCENT_CYAN)
                    .stroke(Stroke::NONE);
                    if ui.add_enabled(can_start, power_on).clicked() {
                        action = Some(VmBarAction::PowerOn);
                    }
                });
            });
        });
    hairline_below(ui, bar.response.rect);
    action
}
```

`Quit` is reached from the `⋯` menu once the About window is open, so
`VmBarAction::Quit` is raised there; keep the variant and match it in Step 2 so
the set stays exhaustive.

- [ ] **Step 2: Wire it and delete the two old bars**

In `impl eframe::App for NativeShellApp`, replace
`self.draw_menu_bar(ui); self.draw_toolbar(ui);` with `self.draw_vm_bar(ui);`,
and add:

```rust
    fn draw_vm_bar(&mut self, ui: &mut egui::Ui) {
        let status = self.runtime_status();
        let action = crate::shell::vm_bar::draw_vm_bar(
            ui,
            VmBarState {
                name: &self.vm_info.name,
                badge: shell_state_badge(&status, self.has_error_notice()),
                running: status.running,
                start_pending: status.start_pending,
                on_console: self.chrome.page() == ShellPage::Console,
                serial_shown: self.chrome.show_serial,
                mouse_captured: self.emulator.mouse_captured(),
            },
        );
        match action {
            None => {}
            Some(VmBarAction::ToggleSidebar) => {
                self.chrome.show_library = !self.chrome.show_library;
            }
            Some(VmBarAction::PowerOn) => self.start_vm(),
            Some(VmBarAction::PowerOff) => self.request_power_off(),
            Some(VmBarAction::Restart) => self.request_reset(),
            Some(VmBarAction::ToggleSerial) => {
                self.chrome.show_serial = !self.chrome.show_serial;
            }
            Some(VmBarAction::ToggleMouseCapture) => self.emulator.toggle_mouse_capture(),
            Some(VmBarAction::SendCtrlAltDel) => self.emulator.send_ctrl_alt_del(),
            Some(VmBarAction::ShowAbout) => self.chrome.show_about = true,
            Some(VmBarAction::Quit) => {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
```

Then delete `draw_menu_bar`, `draw_toolbar`, `shell_menu_style` and
`shell_menu_labels`.

- [ ] **Step 3: Delete the test that asserts a constant against itself**

Remove `shell_menu_labels_omit_redundant_view_and_tabs` from `app.rs`'s
`mod tests`. It reads a `cfg(test)` list of four labels while the bar it claims
to describe drew five (File, Edit, VM, **Input**, Help), so it could not fail
for any change to the shell. Do not replace it — there is nothing left for it
to assert.

- [ ] **Step 4: Add `Quit` to the About window**

In `draw_about_window`, add a `Quit` button beside the existing close so the
menu item has a home:

```rust
            if ui.button("Quit Rusty Box").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
```

- [ ] **Step 5: Build and test**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean, no `dead_code`.
Run: `cargo test --release -p rusty_box_gui --lib`
Expected: PASS.

- [ ] **Step 6: Screenshot every verb state**

Relaunch. Screenshot the bar **stopped** (only `Power on` enabled, filled) and,
after clicking `Power on` and waiting ~30 s for Alpine, **running** (`Power on`
disabled, the other two live). Switch to Console and screenshot again — the
three console-local controls appear only there. Click `☰` and confirm the tree
hides and returns.

- [ ] **Step 7: Commit**

```bash
git add rusty_box_gui/src/shell/vm_bar.rs rusty_box_gui/src/shell/mod.rs rusty_box_gui/src/app.rs
git commit -m "feat(gui): one bar carries the VM's name, its state, and the verbs that change it

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: Hardware and Images on the grid

The two panes the audit measured as ragged.

**Files:**
- Modify: `rusty_box_gui/src/app.rs` — `draw_hardware_page` (:1350),
  `draw_hardware_detail` (:1391-1946), `DiskCreatorPanel::ui_page` (:2326)

**Interfaces:**
- Consumes: `widgets::{page_header, field_row, ActionTileWeight}`,
  `theme::{SPACE_PAGE, SPACE_GROUP, TEXT_TITLE}`
- Produces: nothing new

- [ ] **Step 1: Give Hardware a header and two filled columns**

```rust
    /// The device list takes a fixed column; the detail card takes the rest.
    /// A card's width is the layout's decision, never the card's own.
    const HARDWARE_LIST_WIDTH: f32 = 150.0;

    fn draw_hardware_page(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .inner_margin(egui::Margin::same(SPACE_PAGE))
            .show(ui, |ui| {
                self.draw_shell_notice(ui);
                page_header(ui, "Hardware", "Settings apply at power-on.");
                let height = ui.available_height();
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(HARDWARE_LIST_WIDTH, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            shell_card_frame().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(height - f32::from(SPACE_CARD) * 2.0);
                                for device in HardwareDevice::ALL {
                                    let selected = self.chrome.selected_hardware == device;
                                    if hardware_row(ui, device.label(), selected).clicked() {
                                        self.chrome.selected_hardware = device;
                                    }
                                }
                            });
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            shell_card_frame().show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.set_min_height(height - f32::from(SPACE_CARD) * 2.0);
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    self.draw_hardware_detail(ui);
                                });
                            });
                        },
                    );
                });
            });
    }
```

`hardware_row` is `tree_row`'s sibling for a flat list; add it to
`shell/widgets.rs` so the selection idiom has exactly one implementation
shape:

```rust
/// A row in a flat selectable list, wearing the shell's one selection idiom.
pub(crate) fn hardware_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 6.0, BG_CARD);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
            0.0,
            ACCENT_CYAN,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, BG_CARD.gamma_multiply(0.5));
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(TEXT_BODY),
        if selected { TEXT_PRIMARY } else { TEXT_MUTED },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
```

The detail card's own `set_min_width(520.0)` and the list card's
`set_min_width(190.0)` are deleted — the layout sets both widths now.

- [ ] **Step 2: Put every Hardware setting on the field grid**

Throughout `draw_hardware_detail`, replace each

```rust
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Guest memory").strong().color(TEXT_PRIMARY));
                        changed |= draw_u32_field(ui, &mut self.settings.memory_mib, 1, 4096, " MB", editable, memory_step, None);
                    });
```

with

```rust
                    changed |= field_row(ui, "Guest memory", |ui| {
                        draw_u32_field(
                            ui,
                            &mut self.settings.memory_mib,
                            1,
                            4096,
                            " MB",
                            editable,
                            memory_step,
                            None,
                        )
                    });
```

Apply the same rewrite to every labelled control in the function — memory, host
memory, memory block, processors, boot order, disk, CD-ROM, serial, VGA mode.
The label text is unchanged; only the wrapper differs.

- [ ] **Step 3: Make `Save settings to config file` the pane's primary**

Wherever that button is added, replace the plain `egui::Button::new(..)` with
the primary treatment already used by the power verb:

```rust
        let save = egui::Button::new(
            RichText::new("Save settings to config file")
                .strong()
                .color(BG_BASE),
        )
        .fill(ACCENT_CYAN)
        .stroke(Stroke::NONE);
        if ui.add_enabled(editable, save).clicked() {
            self.save_settings_to_config_file();
        }
```

- [ ] **Step 4: Replace the Images hero with a page header**

In `DiskCreatorPanel::ui_page`, delete the first `shell_card_frame()` block
(the "Disk Images" hero, `app.rs:2330-2343`) and the `ui.add_space(12.0)` that
follows it. Open with:

```rust
        egui::Frame::new()
            .inner_margin(egui::Margin::same(SPACE_PAGE))
            .show(ui, |ui| {
                page_header(
                    ui,
                    "Disk images",
                    "Create flat hard disks and floppy images with the bximage backend.",
                );
```

and let the remaining form card take `ui.set_width(ui.available_width())`.

- [ ] **Step 5: Put the Images form on the same grid**

`Path`, `Size`, `Floppy format` and `Overwrite existing file` each become a
`field_row`, so all four inputs start on one x:

```rust
                    field_row(ui, "Path", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.path)
                                .desired_width(320.0),
                        );
                        if ui.button("Browse…").clicked() {
                            self.choose_native_image_path();
                        }
                    });
                    field_row(ui, "Size", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.hard_disk_size)
                                .hint_text("20G")
                                .desired_width(120.0),
                        );
                        ui.label(
                            RichText::new("Examples: 10M, 512M, 20G, 512")
                                .size(TEXT_SECONDARY)
                                .color(TEXT_MUTED),
                        );
                    });
```

and the checkbox, which the audit found rendering with no visible glyph and an
odd indent. **Keep its `cfg`** — the `overwrite` field itself is
`#[cfg(not(target_arch = "wasm32"))]` (`app.rs:596`), so dropping the attribute
breaks the browser build:

```rust
                    #[cfg(not(target_arch = "wasm32"))]
                    field_row(ui, "", |ui| {
                        ui.checkbox(&mut self.overwrite, "Overwrite existing file");
                    });
```

- [ ] **Step 6: Make `Create image` the pane's primary**

Same treatment as Step 3, with `ACCENT_CYAN` fill and `BG_BASE` text.

- [ ] **Step 7: Build**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean.
Run: `cargo test --release -p rusty_box_gui --lib`
Expected: PASS.

- [ ] **Step 8: Screenshot and measure**

Relaunch. Screenshot Hardware on **all four** device sections and Images on
both `Hard Disk` and `Floppy`. Then `query_tree` and read the `bounds` of the
first input on each row: **every input in a pane must share one x.** If two
differ, the field row was skipped somewhere — find it before committing. Check
that neither card ends mid-air with space below it.

- [ ] **Step 9: Commit**

```bash
git add rusty_box_gui/src/app.rs rusty_box_gui/src/shell/widgets.rs
git commit -m "style(gui): Hardware and Images share one page header, one grid, and one primary verb

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 7: Summary, the Console's empty state, and the colour rule

**Files:**
- Modify: `rusty_box_gui/src/app.rs` — `draw_home_page` (:1233),
  `draw_home_header` (:1277), `draw_console_page` (:1344)

**Interfaces:**
- Consumes: everything above
- Produces: nothing new

- [ ] **Step 1: Take the duplicated verbs off Summary**

In `draw_home_header`, delete the right-to-left row holding
`Hardware Settings`, `Delete Profile` and `Duplicate VM Profile`, and re-add
only `Delete Profile` below the facts row — the other two now live in the tree
and the sidebar's `+`. Keep its `delete_enabled` guard exactly as it is
(`!running && !start_pending && self.profiles.len() > 1`).

The VM-name editor keeps its `TextStyle::Heading` font; set its size explicitly
to the scale:

```rust
                    name_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut profile.name)
                                .font(egui::FontId::proportional(TEXT_DISPLAY))
                                .desired_width(320.0),
                        )
                        .changed();
```

- [ ] **Step 2: Spend the accent only where the rule allows**

In `draw_home_page`, the three tiles currently take `ACCENT_CYAN`,
`ACCENT_BLUE` and `ACCENT_AMBER`. Cyan means state or the primary verb; blue
means data; amber means pending. Two of those three are neither. Pass
`ACCENT_CYAN` with `ActionTileWeight::Primary` for `Power On VM`, and
`STROKE_HAIRLINE` with `ActionTileWeight::Secondary` for the other two, so only
the primary tile carries an accent at rest:

```rust
                        action_tile(
                            &mut columns[1],
                            "Create Disk Image",
                            "Build bximage-compatible hard disks and floppies.",
                            STROKE_HAIRLINE,
                            ActionTileWeight::Secondary,
                            || self.chrome.go_to(ShellPage::Images),
                        );
```

- [ ] **Step 3: Give the Console an empty state**

```rust
    fn draw_console_page(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.draw_shell_notice(ui);
        let status = self.runtime_status();
        if !status.running && !status.start_pending {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("This VM is powered off")
                            .size(TEXT_TITLE)
                            .color(TEXT_MUTED),
                    );
                    ui.label(
                        RichText::new("Power on in the bar above to start it.")
                            .size(TEXT_CAPTION)
                            .color(TEXT_MUTED),
                    );
                });
            });
            return;
        }
        self.emulator
            .ui_embedded_with_serial(ui, frame, self.chrome.show_serial);
    }
```

- [ ] **Step 4: Sweep for glyphs that are not power verbs**

Search `app.rs` for `▣`, `＋`, `▾` and `□`. The shell keeps glyphs only on the
three power verbs (`▶ ■ ↻`). The sidebar's `+` is a plain ASCII plus. Delete
the rest, leaving the label text alone.

Run: `grep -n "▣\|＋\|▾\|□" rusty_box_gui/src/app.rs`
Expected after the sweep: only matches inside `#[cfg(target_os = "android")]`
blocks or `WebShellApp`, which this change does not touch. Report what remains.

- [ ] **Step 5: Build and test**

Run: `cargo check --release -p rusty_box_gui`
Expected: clean.
Run: `cargo test --release -p rusty_box_gui --lib`
Expected: PASS.

- [ ] **Step 6: Screenshot every pane, powered off and running**

Relaunch. Screenshot Summary, Console, Hardware and Images **stopped**; then
`Power on`, wait for Alpine's `login:`, and screenshot all four **running**.
Eight images. Put them all in the report beside Task 1's originals.

Check against the spec: one page-header shape everywhere, one selection idiom,
one filled button in the whole shell, no card ending mid-air, every input in a
pane on one x, and a Console that says something when it is off.

- [ ] **Step 7: Run the full gate**

```bash
cargo xtask ci > gate.log 2>&1; grep -c FAILED gate.log
```

Expected: `0`. If the log stops mid-step with `FAILED` 0 and exit
`0xffffffff`, another project killed `xtask.exe` by name — retry once.
Delete `gate.log` before committing; do not stage it.

- [ ] **Step 8: Commit**

```bash
git add rusty_box_gui/src/app.rs
git commit -m "style(gui): the accent means one thing, and a powered-off console says so

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Self-review of this plan

**Spec coverage.** §1 navigation → Tasks 3 and 4. §2 chrome → Task 5 (menu bar
disposition table, the stale test). §3 vocabulary → Task 2 (`page_header`,
`field_row`) and Task 6 (card fill, `ActionTileWeight`). §4 scale → Task 1
(constants), Task 2 (`home_fact`), Task 7 (colour rule, glyph sweep). §5 pages
→ Tasks 6 and 7. §6 form factors → Task 4 Step 4 (android), and the global
constraint that `WebShellApp` is untouched. §7 naming → Task 3 Step 3's doc
comment. §8 file structure → each task's Files block. §9 verification → every
task's screenshot step, plus Task 7 Step 7.

**Type consistency.** `Destination`, `ShellPage`, `SidebarAction`,
`VmBarAction`, `VmBarState`, `ShellStateBadge`, `VmLibraryEntry`,
`page_header`, `field_row`, `FIELD_LABEL_WIDTH`, `hardware_row`, `tree_row`,
`draw_sidebar`, `draw_vm_bar` are each defined in exactly one task and spelled
the same at every later use.

**Two things a reviewer should expect to be argued.**

1. `tree_row` (Task 4) and `hardware_row` (Task 6) are the same shape with
   different indent and dot arguments. They are deliberately two functions
   because one lives in the sidebar and one in a pane, and merging them would
   put a sidebar concept in `widgets.rs`. If the reviewer disagrees, merging is
   the easy direction — but do not merge them by copying the body.
2. Task 6 Step 1's `set_min_height(height - SPACE_CARD * 2.0)` assumes the
   card's own margin is the only thing between the frame and its content. If a
   rendered frame shows a scrollbar appearing on a pane that fits, that
   assumption is wrong: drop the `set_min_height` and let the card size to its
   content rather than fighting it.
