# The Shell Is One Tree — Design

> Supersedes `docs/superpowers/plans/2026-09-08-the-shell-is-consistent-throughout.md`
> (`fd41c7b`), whose seven audited items are folded in below. Written
> 2026-09-08 at HEAD `af85918` on `wip/atom-execctx`.

## The problem

`rusty_box_gui`'s desktop shell carries **four** chrome layers — a menu bar, a
toolbar, a left library column and a tab strip — to reach **four** destinations.
Navigation and selection are two separate acts (`selected_vm` and
`selected_page` are independent fields that can disagree), the menu bar is a
fifth path to actions that already exist elsewhere, and each pane invents its
own header, its own card widths and its own field alignment.

A live audit of a rendered frame at `f184706` found: two selected-tab
treatments, three different page-header shapes, `ActionTileWeight` applied on
one pane out of four, cards that end mid-air with 350 px of dead space beneath
them, field inputs 6 px out of alignment, a Console tab that is entirely blank
when powered off, and five toolbar glyphs from three families.

None of that is fixable by patching, because nothing in the code stops the next
pane from inventing a sixth idiom. The vocabulary has to exist before
consistency can be enforced.

## Decisions taken

| decision | chosen | rejected, and why |
|---|---|---|
| Scope | Redesign navigation, not only style | A polish pass leaves each pane free to invent its own idiom |
| Shape | **One wide sidebar tree** | An activity rail + context panel was the alternative; the tree makes selecting a VM and selecting its page one act |
| Staging | Future inspection views become **more children of the VM node** | Docking was considered and is not needed: a register view is a page like any other |
| Treatment | **Instrument** — the existing navy palette, compact 8 px rhythm | A warmer graphite palette was mocked and rejected: it re-tokenises every colour and leaves the pure-black VGA framebuffer visibly darker than its own chrome |

## 1. Navigation

The sidebar is the only navigation on the desktop.

```
My computer                      [+]
[ Search VMs                        ]
▾ Alpine 3.24                      ●
      Summary
      Console
      Hardware
      Images
▸ DLX Linux
▸ Windows 7
```

**Rules.**

- Exactly one VM is expanded: the selected one. There is no separate expansion
  state, so no state variable can disagree with the selection.
- Clicking a collapsed VM selects it and lands on its `Summary`.
- Clicking a VM that is already selected leaves the page alone.
- Clicking a child selects that page of that VM.
- The state dot sits on the selected VM's row, in the colour
  `shell_state_badge` already assigns (cyan running, amber starting, red
  faulted, muted stopped).
- The `+` on the header row duplicates the selected profile. `Delete profile`
  stays where it is — inside the selected VM's `Summary` page — because it is
  destructive and does not belong on a hover target in a list.
- The search field filters VM rows through the existing
  `ShellChrome::visible_vm_indices`.

**The destination becomes one value.** `selected_vm: usize` and
`selected_page: ShellPage` are replaced by a single `Destination`, so the pair
cannot drift:

```rust
pub(crate) struct Destination { vm: usize, page: ShellPage }
```

with `select_vm` (lands on `Summary` only when the VM actually changed),
`select_page`, and `clamped_after_removal` for profile deletion. This is the
one part of the redesign that is pure logic, and it is the one part that gets
unit tests.

**The sidebar reports intent; it does not act.** It is a free function that
borrows the data it draws and returns what the user asked for:

```rust
pub(crate) enum SidebarAction { Select(Destination), DuplicateSelected }
pub(crate) fn draw_sidebar(
    ui: &mut egui::Ui,
    entries: &[VmLibraryEntry],
    destination: Destination,
    filter: &mut String,
    badge: ShellStateBadge,
) -> Option<SidebarAction>
```

`NativeShellApp` interprets the action. The sidebar never touches the emulator,
the profiles or the notice state, so it can be read and changed without
understanding either.

**Deleted:** `NativeShellApp::draw_tab_strip`, the desktop call sites of
`nav_button`, and `NativeShellApp::draw_library`.

## 2. Chrome — four bars become two

### The VM bar (36 px, right of the sidebar)

Left: a sidebar-collapse chevron, the VM name, the state pill.
Right: the verbs.

- `Power on` is the bar's **primary verb, and so its one filled button**: as §3
  rules, each pane carries a single filled primary, and every other button
  rests on the hairline. It is disabled while running or starting.
- `Power off` and `Restart` are ghost buttons, enabled only while running.
- **On the Console page only**, the bar also carries `Serial`, `Capture mouse`
  and `Send Ctrl+Alt+Del`. Page-local controls belong to their page; they are
  not global chrome, which is why the `Serial` checkbox leaves the toolbar.
- A `⋯` overflow at the far right holds `About Rusty Box Workstation` and
  `Quit`.

Like the sidebar, the bar returns intent rather than acting:

```rust
pub(crate) enum VmBarAction {
    ToggleSidebar, PowerOn, PowerOff, Restart,
    ToggleSerial, ToggleMouseCapture, SendCtrlAltDel,
    ShowAbout, Quit,
}
```

### The status strip (22 px, full width)

Unchanged in content: state dot and label, engine, `memory · CPUs`, IPS,
`Restart queued`. It keeps its current position below everything.

### The menu bar is deleted

Every item in it except two already exists elsewhere in the shell:

| menu item | where it lives now |
|---|---|
| File ▸ Open Console | the tree's `Console` child |
| File ▸ Duplicate VM Profile | the sidebar's `+` |
| File ▸ Create Disk Image | the tree's `Images` child |
| File ▸ Quit | the VM bar's `⋯` |
| Edit ▸ Clear Library Search | the search field clears itself |
| VM ▸ Power On / Power Off / Restart VM | the VM bar's verbs |
| Input ▸ Send Ctrl+Alt+Del / Capture Mouse | the VM bar, on Console |
| Help ▸ About Rusty Box Workstation | the VM bar's `⋯` |

`draw_menu_bar`, `shell_menu_style` and `shell_menu_labels` go with it.

**A stale test goes too.** `shell_menu_labels()` (`app.rs:559`) is `cfg(test)`
and hardcodes four labels while the bar draws five (File, Edit, VM, **Input**,
Help). The test `shell_menu_labels_omit_redundant_view_and_tabs` asserts that
hardcoded list against itself and never reads the menu bar — it cannot fail for
any change to the real code, which is exactly what doctrine R9 forbids. It is
deleted with the bar rather than repaired.

## 3. The page vocabulary

Four rules, and a module that owns them.

**`page_header(ui, title, subtitle)`** — a 16 px title over an 11 px muted
subtitle, then 12 px of space. Every pane opens with exactly this. It replaces
the Images hero card, Hardware's `Hardware Summary  |  Memory` pipe, and
`hardware_intro`.

**A card fills its column, and the column is set by the layout.** No pane sets
a card's width with `set_min_width`. That call is the actual cause of the
floating cards and the ragged right edge the audit measured: a card sized from
the inside cannot know the column it sits in. Cards are allocated an explicit
width by their parent and then take `ui.available_width()`.

**`field_row(ui, label, add_contents)`** — the label occupies a fixed 104 px
column, so every input in a pane starts on the same x. It replaces every
ad-hoc `ui.horizontal(|ui| { ui.label(..); ui.add(..) })` in Hardware and
Images, which is where the 303-vs-297 px misalignment came from.

**`ActionTileWeight` applies to every pane, not just Home.** Each pane's main
verb carries `Primary`; everything else is `Secondary`. That makes
`Create image` and `Save settings to config file` stop rendering as plain grey
buttons.

## 4. The scale

Naming the scale is what makes "consistent" checkable in review.

**Type** — five sizes, two weights (regular and `.strong()`):

| constant | size | used for |
|---|---|---|
| `TEXT_DISPLAY` | 22.0 | the editable VM name on Summary |
| `TEXT_TITLE` | 16.0 | page headers, card titles |
| `TEXT_BODY` | 14.0 | tree rows, field labels, buttons |
| `TEXT_SECONDARY` | 12.5 | tile bodies, descriptions |
| `TEXT_CAPTION` | 11.0 | subtitles, status strip, fact captions |

The stray 10.5 px and 13.0 px in `home_fact` collapse onto `TEXT_CAPTION` and
`TEXT_BODY`.

**Spacing** — multiples of four: `SPACE_ITEM` 8 within a group, `SPACE_GROUP`
12 between groups, `SPACE_CARD` 16 card padding, `SPACE_PAGE` 20 page margin.

**Colour discipline** — the palette does not change; what changes is what is
allowed to spend it.

- `ACCENT_CYAN` — **state, and the single primary verb. Nothing else.**
- `ACCENT_BLUE` — data and links.
- `ACCENT_AMBER` — pending or queued.
- `ACCENT_RED` — faults.

Under this rule the three Summary action tiles stop being cyan/blue/amber; only
`Power on VM` keeps its accent, and the other two rest on the hairline.

**One selection idiom** — a 2 px `ACCENT_CYAN` bar on the left edge plus a
`BG_CARD` fill, used in the sidebar tree and the Hardware device list and
nowhere else. `selectable_label`'s default frame is not used for selection
anywhere in the desktop shell.

**Glyphs** — the audit found five styles from three families. The shell keeps
glyphs only on the three power verbs (`▶ ■ ↻`), which are a recognised set.
Navigation and object-creation buttons carry text alone.

## 5. The pages

**Summary** (`ShellPage::Home`; see §7 for the naming ruling) keeps its header
card, its facts row and its three action tiles, and loses its duplicated action
row — `Hardware Settings`, `Delete Profile` and `Duplicate VM Profile` were a
third copy of controls now in the bar and the sidebar. `Delete Profile` stays,
because it is the one of the three with nowhere else to live.

**Console** gains an empty state. Powered off, it renders — centred in the VGA
region, in `TEXT_MUTED` — that the VM is powered off and that `Power on` in the
bar above starts it. The `Serial` toggle moves to the VM bar.

**Hardware** opens with `page_header("Hardware", "Settings apply at
power-on.")`, then a fixed 150 px device column beside a detail card that takes
the remaining width; both fill the page's height. Every setting is a
`field_row`. `Save settings to config file` becomes the pane's `Primary`.

**Images** loses its hero card to `page_header("Disk images", "Create flat hard
disks and floppy images with the bximage backend.")`, puts `Path`, `Size` and
`Floppy format` on the field grid, gives `Overwrite existing file` a real
checkbox on the grid's input column, and makes `Create image` the pane's
`Primary`.

## 6. Form factors that are not the desktop

**Android keeps tabs.** `draw_android_console_header` navigates with
`nav_button`, and android sets `show_library = false`, so on a phone the tab
strip is the *only* navigation on non-Console pages. Deleting it would strand
that build with no way to leave the Console. So `nav_button` and the tab strip
survive under `#[cfg(target_os = "android")]`, documented as the phone form
factor's navigation, and the sidebar is `#[cfg(not(target_os = "android"))]`.

This is deliberately the minimum: **the android target cannot be compiled in
this environment**, so any change to those blocks is unverifiable. Gating them
unchanged is the only option that cannot silently break them.

**The browser shell is not restructured, but it inherits the vocabulary.**
`WebShellApp` has its own `nav_button` (`app.rs:3618`), its own `draw_library`
(`app.rs:3214`) and its own toolbar, in a separate `impl` block; none of those
change. It shares `ShellPage`, `ShellChrome`, `shell_should_draw_library` and
`VmLibraryEntry`, all of which survive.

Where it calls a helper this design rewrites, it takes the new rendering. Six
of `hardware_intro`'s twelve call sites are inside it, so its hardware headers
become `page_header`s. Freezing them would mean a `cfg`-gated copy of every
rewritten helper — a permanently forked vocabulary, re-litigated on each
change, to hold a surface no ci step renders. The drift is the cheaper side.

## 7. Naming ruling: `ShellPage::Home` keeps its name

The desktop tree labels that page **Summary**, because it summarises the
selected VM. The enum variant stays `Home` because the *browser* shell's home
page is a genuine home — it is where a user uploads an ISO and boots it, and
calling that "Summary" would be false. A label is copy; a variant is a name for
what the code means in both shells. Renaming would touch 12 sites to make one
shell read better and the other read worse.

## 8. File structure

`app.rs` is 5,095 lines. This work would push it past 5,500, so the vocabulary
and the two new chrome surfaces get their own module:

| file | responsibility |
|---|---|
| `rusty_box_gui/src/shell/mod.rs` | declares the module, re-exports its items |
| `rusty_box_gui/src/shell/theme.rs` | palette, type and spacing scale, `configure_shell_style`, `shell_card_frame` |
| `rusty_box_gui/src/shell/widgets.rs` | `page_header`, `field_row`, `action_tile*`, `status_dot`, `status_text`, `hairline_*`, `home_fact`, `metadata_text` |
| `rusty_box_gui/src/shell/destination.rs` | `ShellPage`, `Destination`, `SidebarAction`, `VmBarAction`, and their tests |
| `rusty_box_gui/src/shell/sidebar.rs` | `VmLibraryEntry`, `draw_sidebar` |
| `rusty_box_gui/src/shell/vm_bar.rs` | `draw_vm_bar` |

`app.rs` keeps the application struct, the pages, and the wiring that turns an
action into an effect.

## 9. Verification

**The gate cannot see any of this.** `cargo xtask ci` builds this crate only as
`cargo build -p rusty_box_gui --release`, which proves nothing renders. So:

- Every task screenshots the affected panes **before and after** through the
  `egui` MCP tools against a live GUI on `127.0.0.1:5719` — never desktop
  screenshots.
- `cargo check --release -p rusty_box_gui` after each edit batch, and
  `cargo test --release -p rusty_box_gui --lib` for the destination tests,
  which `xtask ci` does not run for this crate.
- `cargo xtask ci` before the final commit.
- Any item that looks worse rendered than it read on paper is abandoned, and
  the report says so.

The only unit tests worth writing are over `Destination`, because it is the
only pure logic here. Tests that assert a colour constant equals itself, or
that a hardcoded label list contains what it was written to contain, are the
defect this design is deleting — do not add more of them.

## 10. Out of scope, and one known non-defect

- **The status bar's IPS reads `---` while a guest runs on WHP.** That path
  publishes `ips = 0`, and `RunSummary.instructions_executed` is `None` on
  hardware because nothing counts a hardware processor's retired instructions
  (`60f86c3`). It is rendered muted so it cannot pass for a real value.
  **Do not "fix" it by inventing a number** — giving the bridge a rate is a
  separate change to `RunSummary`.
- Keyboard behaviour is not touched. `144c108`'s changes stand.
- A third copy of the egui→key mapping lives in `examples/rusty_box_web` and no
  ci step builds it. Out of scope, noted so it is not discovered as new.
