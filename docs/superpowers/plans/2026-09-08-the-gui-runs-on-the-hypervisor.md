# The GUI Runs On The Hypervisor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `--engine whp` works in the desktop GUI, so Alpine can be watched booting on the hypervisor; and the shell it is watched in is worth looking at.

**Architecture:** The WHP engine refuses `SliceEngine::run_slice` — a machine on the hypervisor is driven by `FastMachine`, which the examples already do and the GUI never learned. Task 1 gives the GUI a real hypervisor drive path beside its interpreter one. Task 2 is a separate, purely visual pass over the shell, working within the design tokens `app.rs` already defines.

**Tech Stack:** Rust, `rusty_box_gui` (eframe/egui 0.35), `rusty_box_whp_engine`, Windows Hypervisor Platform.

## Global Constraints

- Read `CLAUDE.md` first and follow it. **Edit with the Edit/Write tools, never shell heredocs, `sed -i`, or Python**; release builds only; **never run `cargo fmt`** — it reformats 169 files over your edits and would bury a GUI diff entirely; never `let _ = <Result>`; comments state today's invariant, never history.
- **Never use the LSP tools** (`mcp__lsp__references`/`definition`/`diagnostics`) — they hang indefinitely in this workspace. The compiler is the oracle. rust-analyzer here also emits stale false-positive errors; reproduce every diagnostic with a real cargo command.
- **The gate does NOT build the examples, and builds `rusty_box_gui` only as `cargo build -p rusty_box_gui --release`.** `ci: N steps passed` does not prove a GUI change renders or even that its tests compile. Build and run the GUI yourself.
- `cargo xtask ci` before every commit. **Never pipe it and never append to its line** — a pipeline's exit code masks the gate's. Redirect to a log, read it for `ci: N steps passed`, grep it for `FAILED`.
- **A gate run here can be KILLED, not failed**, by another project's session running `Stop-Process` on `xtask.exe` by name: log stops mid-step, `FAILED` count 0, exit `0xffffffff`. Retry once. Do not wait on foreign `cargo`/`xtask` processes. **Never kill a process you did not start** — except a GUI you launched yourself, which is yours to close.
- **Never stage** `ROADMAP.md`, `docs/superpowers/plans/2026-09-03-whp-vmm-shape.md`, or the three untracked `docs/superpowers/specs/2026-08-22-*.md` files. Explicit `git add <path>` only; never `git add -A`. **Leave `stash@{0}` alone.**
- Commit messages end with: `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- Branch: `wip/atom-execctx`. Do not create branches.

## How to see the GUI you are changing

Launch with inspection and drive it through the `egui` MCP tools — do NOT use desktop screenshots:

```bash
EGUI_INSPECTION=1 cargo run --release -p rusty_box_gui -- --no-config --display egui \
  --engine whp \
  --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest \
  --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin \
  --cdrom alpine-virt-3.24.1-x86_64.iso --boot cdrom \
  --memory-mib 512 --host-memory-mib 512 --ips 300000000
```

Then `attach` (127.0.0.1:5719), `screenshot` to see the frame, `query_tree` to find widgets, `click` to drive. The shell starts **Stopped** on the launcher — the guest console appears only after clicking the "Power On VM" **Button** (note: three nodes match that text; the Button is the one with `role: Button`). Close the GUI when done; it is your process.

---

### Task 1: The GUI drives a hypervisor machine with `FastMachine`

**Files:**
- Modify: `rusty_box_gui/src/runner.rs`

**Interfaces:**
- Consumes: `rusty_box_whp_engine::{FastMachine, FastMachineFault, StepStop, WhpEngine}`; `rusty_box::emulator::RunBudget`; the existing `ResolvedConfig`, `RunSummary`, `RunError`.
- Produces: a hypervisor drive path in `runner.rs`, reached from the existing `Engine::Whp` branch.

- [ ] **Step 1: Establish exactly what `drive` does, and what refuses**

Read `drive` (`runner.rs`) and `run_interactive` (`rusty_box/src/emulator/interactive.rs`). `drive` is generic over the engine and ends in `emu.run_interactive(config.max_instructions)`. That path reaches `SliceEngine::run_slice`, which `WhpEngine` implements as an unconditional refusal:

```
a machine on the hypervisor is driven by FastMachine, not by slices
```

List every responsibility `drive` and `run_interactive` carry that a hypervisor path must also carry — at minimum the stop flag, the pre-boot VBE mode, the pre-queued boot Enter, `max_instructions`, and returning `RunSummary { instructions_executed }`. **Write that list into your report before writing code.** A migration that drops one of these silently is the failure mode here: the guest would run but the GUI's stop button, video mode or boot key would quietly stop working.

- [ ] **Step 2: Read the working precedent**

`rusty_box_whp_engine/examples/dlx_whp.rs` drives a `FastMachine`: `FastMachine::adopt(assembled)`, then a loop of `machine.step(RunBudget::Ticks(SLICE_TICKS))` matching on `StepStop::{BudgetSpent, GuestPowerOff, Faulted}`. `alpine_bench.rs` does the same. Follow their shape; do not invent a third.

Note `FastMachine::adopt` takes `Box<Emulator<T, WhpEngine>>` and returns `Result<Self, FastMachineFault>`, and `with_machine(|m| …)` is how you reach the `Emulator` inside it afterwards.

- [ ] **Step 3: Write the hypervisor drive path**

In `runner.rs`, add a function beside `drive` — not inside it. `drive` stays generic and unchanged for the interpreter; the hypervisor gets its own, because the two now genuinely differ in how the guest is advanced, and a generic function with an engine-shaped `if` inside would be the patch this task exists to avoid.

Give it the same signature shape as `drive` but concrete in the engine:

```rust
/// Drive a machine that runs its guest on the hypervisor.
///
/// Separate from [`drive`] rather than a branch inside it: a hypervisor
/// machine is advanced by `FastMachine::step`, which owns the vCPU thread and
/// the device wheel, while an interpreter machine is advanced by
/// `run_interactive`'s own loop. The two share their setup — the stop flag, the
/// pre-boot video mode, the queued boot key — and nothing else.
fn drive_on_the_hypervisor(
    emu: Box<rusty_box::emulator::Emulator<(), rusty_box_whp_engine::WhpEngine>>,
    config: &ResolvedConfig,
    stop_flag: Option<Arc<AtomicBool>>,
) -> Result<RunSummary, RunError>
```

It must, in order: apply the stop flag; apply `config.vga_mode` via `emu.display().set_preferred_mode(...)`; pre-queue the boot Enter when `should_prequeue_boot_enter(&config.boot_order)` says so (calling `emu.prepare_run()` first, as `drive` does); adopt into a `FastMachine`; then loop `step` until the guest powers off, the stop flag is set, a fault occurs, or `max_instructions` is reached; and return `RunSummary { instructions_executed }`.

**The stop flag is the part most easily got wrong.** `run_interactive` checks it inside its own loop. Yours must check it between steps, so the GUI's "Power Off" still works — pick a step budget small enough that the button feels responsive and say in a comment why you chose it.

Map `StepStop` to the existing `RunError` variants rather than inventing new ones; if no variant fits a `FastMachineFault`, add one to `error.rs` with a message in that file's voice and say so in your report.

- [ ] **Step 4: Call it from the existing branch**

The `Engine::Whp` branch already exists in `runner.rs` and returns early with a comment explaining that the two machines are different types. Change only its final call from `drive(emu, &config, stop_flag)` to your new function, and update that comment to say what is true now — that the two paths differ in how they advance the guest, not merely in type.

- [ ] **Step 5: Build and check**

```bash
cargo build --release -p rusty_box_gui
cargo check --release -p rusty_box_gui --all-targets
```
Expected: both clean. Note `--all-targets` is NOT what the gate runs and has caught pre-existing breakage in this crate before; if it reports errors in code you did not touch, say so rather than fixing them here.

- [ ] **Step 6: Watch Alpine boot on the hypervisor**

Launch with the command in "How to see the GUI you are changing", attach, click the "Power On VM" Button, then switch to the Console tab and watch.

Report what you see: does the guest reach `login:`, and roughly how long did it take? Take a `screenshot` at the login prompt and save it to the scratchpad. **If it does not boot, say exactly where it stopped and what the shell showed** — do not retry until it works.

Close the GUI when done.

- [ ] **Step 7: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-gui-whp.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-gui-whp.log
grep -c FAILED /tmp/gate-gui-whp.log
```

```bash
git add rusty_box_gui/src/runner.rs
git commit -m "feat(gui): a hypervisor machine is driven by FastMachine, not by slices

The slice engine is gone, and WhpEngine::run_slice is an unconditional refusal
naming the verb it wants. The GUI still drove every machine through
run_interactive, so --engine whp failed at power-on with that refusal and the
desktop shell could not run a guest on hardware at all.

The hypervisor now has its own drive path beside the interpreter's, sharing
their setup — stop flag, pre-boot video mode, queued boot key — and differing
only where they genuinely differ: how the guest is advanced.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: The shell is worth looking at

Purely visual. **No behaviour changes** — if you find yourself editing anything other than layout, spacing, colour, typography or widget chrome, stop and report it instead.

**Files:**
- Modify: `rusty_box_gui/src/app.rs`

**Interfaces:**
- Consumes: the design tokens already defined at the top of `app.rs` — `BG_BASE`, `BG_PANEL`, `BG_CARD`, `STROKE_HAIRLINE`, `TEXT_PRIMARY`, `TEXT_MUTED`, `ACCENT_CYAN`, `ACCENT_BLUE`, `ACCENT_AMBER`, `ACCENT_RED` — and the theme block at `ctx.style_mut_of(Theme::Dark, …)`.
- Produces: nothing consumed elsewhere.

- [ ] **Step 1: Look at it before you change it**

Launch the GUI (command above), attach, and `screenshot` both the Home tab and the Console tab. Save both to the scratchpad. **You cannot polish what you have not looked at**, and the tree alone does not show spacing, weight or rhythm.

- [ ] **Step 2: Work within the existing tokens**

`app.rs` already defines a coherent palette and a dark theme. **Use those constants; do not introduce new literal colours.** If a shade you need genuinely does not exist, add it as a named constant beside the others, in the same style, and say why in your report.

- [ ] **Step 3: Fix these five, which are visible in the current shell**

1. **The menu bar and the tab strip are indistinguishable.** `File Edit VM Input Help` sits in the same band, with the same weight, as `Home Console Hardware Images`. Separate them — a hairline, a different background, or different height — so the eye reads two rows, not one.
2. **"Power On" appears twice**, once as a tab and once as a toolbar button, meaning different things. Resolve the collision: the toolbar action is the verb; the tab is a destination. Rename or restyle so they cannot be confused.
3. **The hero title wastes the fold.** `RUSTY BOX WORKSTATION` plus its subtitle occupies the top third of the Home tab and says nothing a user needs twice. Shrink it, and give the space to the selected VM's actual state.
4. **The three action cards are inert rectangles** — same weight, same border, one button each, loose vertical rhythm. Give them a clear primary (Power On) against two secondaries, tighten the spacing, and make the whole card feel like the target rather than only its button.
5. **The status bar is nearly empty**: `Stopped | --- IPS | Ready`. It is the one place a VM shell should always tell the truth — show at least the engine in use (interpreter or hypervisor), the guest's state, and the IPS, using `ACCENT_CYAN`/`ACCENT_AMBER`/`ACCENT_RED` to distinguish running, paused and faulted.

- [ ] **Step 4: Look again, and compare**

Rebuild, relaunch, `screenshot` the same two tabs, and save them beside the "before" images. **Put the before/after pairs in your report** and say, for each of the five, what changed. If one of them turned out to be a bad idea when you saw it rendered, say that instead of forcing it — a rendered frame is better evidence than a plan.

- [ ] **Step 5: Confirm you changed nothing but appearance**

```bash
git diff --stat rusty_box_gui/src/app.rs
cargo build --release -p rusty_box_gui
```

Read your own diff and confirm every hunk is layout, colour, spacing, typography or widget chrome. Any hunk that touches emulator state, commands, or the bridge is out of scope for this task — revert it and report it.

- [ ] **Step 6: Gate and commit**

```bash
cargo xtask ci > /tmp/gate-gui-style.log 2>&1
grep -E "ci: .* steps passed" /tmp/gate-gui-style.log
grep -c FAILED /tmp/gate-gui-style.log
```

```bash
git add rusty_box_gui/src/app.rs
git commit -m "style(gui): the shell reads as a workstation, not a debug window

Five things the frame showed: the menu bar and tab strip were one undifferentiated
band; 'Power On' meant two different things in two places; the hero title took the
top third of the Home tab to say what the window title already says; the action
cards were three identical rectangles with no primary; and the status bar — the one
place a VM shell must always be honest — said almost nothing.

Appearance only. The palette and dark theme already in this file are the vocabulary;
this uses them rather than adding to them.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage.** There is no separate spec; this plan is the record. Its two halves match the user's request exactly — a proper migration written in the crate's own idiom (Task 1), plus visual polish (Task 2). Task 1's "not a patch" requirement is enforced structurally: Step 3 mandates a separate function and explains why a branch inside `drive` would be the wrong shape.

**2. Placeholder scan.** No TBD/TODO. Task 1 carries the signature and the ordered list of responsibilities. Task 2's five items are each a concrete, named defect visible in the current frame rather than "make it nicer" — the failure mode for a styling task is exactly that vagueness.

**3. Type consistency.** `FastMachine::adopt` takes `Box<Emulator<T, WhpEngine>>` → `Result<Self, FastMachineFault>`; `step` takes `RunBudget` → `Result<StepOutcome, FastMachineFault>` with `StepStop::{BudgetSpent, GuestPowerOff, Faulted}`; `RunSummary { instructions_executed }` is what both drive paths return. Task 2 introduces no types.

**4. Risks this plan does not remove.**

- **The stop flag is the likely defect.** `run_interactive` owns its loop and checks the flag inside it; a `FastMachine` loop must check between steps, and too large a budget makes "Power Off" feel broken without failing any test. Step 3 calls this out, but no test in this plan catches it — only Step 6's manual run will.
- **`instructions_executed` may not be recoverable from `FastMachine` the way `run_interactive` returns it.** If it is not, the honest answer is to report that rather than return a fabricated number, and `RunSummary`'s meaning for a hypervisor run needs a sentence in the report.
- **Task 2 is judgement, and judgement can be wrong.** Step 4 exists to catch that: the instruction is explicitly to abandon any of the five that looks worse rendered than described.
