# Driving a guest from Rust

Rusty Box runs a PC without a human at the keyboard. A program builds a
machine, boots it, reads what is on the screen, types at it, and watches what
it does — all in process, with no window open.

This guide is task-based: each section answers one "how do I…", shows the code
that does it, and then says where the edge is. Every Rust block here is
compiled by the test suite — the hypervisor ones excepted, since that crate is
not a dependency of `rusty_box` — so the code and the text cannot drift apart.

For the graphical runner, its configuration file and its disk handling, see
[getting-started.md](getting-started.md) and
[rusty_box_gui/README.md](../rusty_box_gui/README.md). For what a guest may
and may not do on the hypervisor engine, see
[whp-guest-capabilities.md](whp-guest-capabilities.md).

```toml
[dependencies]
rusty_box = { git = "https://github.com/r3bb1t/rusty_box" }
```

The API in this guide needs the `std` feature, which is on by default.

---

## 1. Boot a machine headless

`MachineBuilder` assembles a machine: memory, firmware, disks and a boot
order. `build()` hands back a machine sitting at its reset vector.

```rust,no_run
use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, DiskGeometry, EmulatorConfig, Ips,
    MachineBuilder, MemorySize, RunBudget, StopReason,
};
use rusty_box::gui::NoGui;

let config = EmulatorConfig {
    memory: MemorySize::bytes(256 * 1024 * 1024),
    ips: Ips::BOCHS_DEFAULT,
    ..EmulatorConfig::default()
};

let bios = std::fs::read("BIOS-bochs-latest")?;
let vga_bios = std::fs::read("VGABIOS-lgpl-latest.bin")?;

let mut machine = MachineBuilder::new(config)
    .gui(NoGui::new())
    .bios(&bios)
    .vga_bios(&vga_bios)
    .boot_order(BootOrder::just(BootDevice::Disk))
    .disk_file(AtaSlot::PRIMARY_MASTER, "disk.img", DiskGeometry::new(1024, 16, 63))
    .build()?;

// Nothing has run yet. Give it a budget and it boots.
let outcome = machine.step(RunBudget::Instructions(50_000_000))?;
assert_ne!(outcome.stop, StopReason::EngineFault);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`NoGui` is the display for a machine nobody is watching: the VGA model still
renders, so [reading the screen](#2-read-the-screen) works, but no window
opens and no frame is uploaded anywhere.

A machine needs no firmware at all if the goal is to run your own code. See
[section 6](#6-read-and-write-memory-and-registers).

**Limits.** One machine is one guest; machines are `Send`, so a fleet is one
per worker thread. Building never runs anything — a machine that has not been
stepped has retired no instructions and its clock reads zero.

---

## 2. Read the screen

`display()` borrows the adapter. `text()` gives the character grid when the
guest is in a text mode, and `None` when it is not.

```rust,no_run
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder};
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;
// One expression: the view borrows the machine, not a binding.
let at_the_prompt = machine
    .display()
    .text()
    .is_some_and(|screen| screen.contains("login:"));

if let Some(screen) = machine.display().text() {
    println!("{} rows of {} columns", screen.rows(), screen.cols());
    if let Some(found) = screen.find("Password:") {
        println!("prompt at row {}, column {}", found.row, found.col);
    }
    if let Some(cursor) = screen.cursor() {
        println!("cursor at row {}, column {}", cursor.row, cursor.col);
    }
    // The whole screen, rows separated by newlines and trailing blanks off.
    print!("{}", screen.to_text());
    // Or one row at a time.
    let first: String = screen.row_chars(0).collect();
    println!("{first}");
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Waiting for a prompt is a loop of "step a little, look":

```rust,no_run
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder, RunBudget};
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;
fn wait_for(
    machine: &mut rusty_box::emulator::Emulator,
    needle: &str,
    rounds: usize,
) -> Result<bool, rusty_box::Error> {
    for _ in 0..rounds {
        let outcome = machine.step(RunBudget::Instructions(5_000_000))?;
        if machine.display().text().is_some_and(|s| s.contains(needle)) {
            return Ok(true);
        }
        if outcome.is_terminal() {
            return Ok(false);
        }
    }
    Ok(false)
}

let booted = wait_for(&mut machine, "login:", 200)?;
# let _ = booted;
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** A match never spans a row boundary, because on a real screen it
does not either. In a graphics mode `text()` is `None` — there are no
characters to read, only pixels. Before the video BIOS programs the CRTC the
grid has columns but no rows, so a scrape that starts too early sees an empty
screen rather than a wrong one. `resolution()` answers in both modes.

---

## 3. Read serial, the debug console and POST codes

Three byte streams come out of a machine, and each is drained the same way.

```rust,no_run
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder};
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;
// COM1. `serial(n)` answers only for a port this machine built.
if let Some(mut com1) = machine.serial(0) {
    let bytes: Vec<u8> = com1.take_output().collect();
    print!("{}", String::from_utf8_lossy(&bytes));
}

// Port 0xE9, the debug console firmware and kernels write to directly.
let debug: Vec<u8> = machine.debug_port().take_output().collect();

// BIOS POST codes: ports 0x80 and 0x84, in write order.
let codes: Vec<u8> = machine.post_codes().take_output().collect();
if let Some(last) = codes.last() {
    println!("firmware reached POST code {last:#04x}");
}
# let _ = debug;
# Ok::<(), Box<dyn std::error::Error>>(())
```

POST codes are the first thing to look at when a machine stops before any
console exists: the last code is where the firmware got to.

**Limits.** Every stream is a bounded ring. A host that never drains loses
the oldest bytes rather than stalling the guest, so a long-running scrape
drains on a schedule. Draining is destructive; the second read sees only what
arrived since the first. A standard machine builds COM1 alone, so `serial(1)`
and above are `None`. Sending bytes *into* a UART is not on this surface — the
graphical runner does it, a program cannot yet.

---

## 4. Send keys, text and mouse input

`keyboard()` and `mouse()` borrow the input devices. Typing reports what the
guest actually took.

```rust,no_run
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder, Typed};
use rusty_box::iodev::scancodes::BxKey;
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;

let text = "root\n";
match machine.keyboard().type_text(text) {
    Typed::All { .. } => {}
    // The guest's ring filled. Resume at the character it stopped on.
    Typed::Refused { delivered } => {
        let rest: String = text.chars().skip(delivered).collect();
        println!("retry with {rest:?}");
    }
    // One character has no scancode on this layout. Resume PAST it.
    Typed::Unmappable { delivered, .. } => {
        let rest: String = text.chars().skip(delivered + 1).collect();
        println!("skipped one, retry with {rest:?}");
    }
}

// Individual keys: press and release, or hold.
let tapped = machine.keyboard().tap(BxKey::F1);
let held = machine.keyboard().key(BxKey::CtrlL, true);

// Raw scancode bytes, for a set this layout does not cover. The count is how
// many the guest's ring took.
let accepted = machine.keyboard().scancodes(&[0x3B, 0xBB]);

// Relative mouse motion, and the button mask.
let moved = machine.mouse().motion(4, -2, 0, 0x01);
# let _ = (tapped, held, accepted, moved);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Input is paced: the guest's keyboard ring holds sixteen bytes, and a host that
types faster than the guest reads gets a short delivery rather than a silent
loss. Step the machine between bursts.

**Limits.** `type_text` maps a US layout. A character with no scancode is
reported, not guessed at. `tap` and `key` answer whether the byte reached the
ring, so a `false` means "type it again after stepping", not "the key does not
exist".

---

## 5. Run, step and stop

A run is bounded by a budget and ends with a reason.

```rust,no_run
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder, RunBudget, StopReason};
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;
// Instructions, when the caller is counting work.
let outcome = machine.step(RunBudget::Instructions(1_000_000))?;
println!("retired {:?}", outcome.progress.instructions());

// Guest time, which is the unit a device deadline is in.
let outcome = machine.step(RunBudget::Ticks(1_000))?;

match outcome.stop {
    StopReason::BudgetExhausted => {}        // keep going
    StopReason::Halted => {}                 // HLT with nothing pending
    StopReason::GuestPowerOff => {}          // the guest asked to be switched off
    StopReason::CpuShutdown => {}            // a triple fault the machine did not reset on
    StopReason::StopRequested => {}          // a stop handle fired
    StopReason::EngineFault => {}            // the execution engine refused
}
// A caller that only needs "should I keep going" asks this instead.
if outcome.is_terminal() {
    println!("done");
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

A halted machine says how far it may skip:

```rust,no_run
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder, RunBudget};
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;
if let Some(ticks) = machine.ticks_to_next_timer_deadline() {
    machine.step(RunBudget::Ticks(ticks))?;
}
println!("clock reads {}", machine.ticks());
# Ok::<(), Box<dyn std::error::Error>>(())
```

Stopping from another thread, and the power buttons:

```rust,no_run
# use rusty_box::cpu::ResetReason;
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder, PowerState, RunBudget};
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).build()?;
let handle = machine.stop_handle();
std::thread::spawn(move || {
    std::thread::sleep(std::time::Duration::from_secs(5));
    handle.stop();
});
machine.step(RunBudget::Instructions(u64::MAX))?;

assert_eq!(machine.power().state(), PowerState::Running);
machine.power().reset(ResetReason::Hardware)?;
// The ACPI power button. A guest with no SCI armed ignores it.
machine.power().press_power_button();
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** An instruction budget retires exactly that many instructions —
including across calls and returns — so it is the unit for stepping. A tick
budget advances the clock exactly, which is what a guest that halts needs,
since a halted processor retires nothing while its timers keep running.

---

## 6. Read and write memory and registers

A machine with no firmware is the shortest path to running your own code: pick
a CPU mode, write bytes, run.

```rust,no_run
use rusty_box::cpu::{CpuSetupMode, X86Reg};
use rusty_box::emulator::{Emulator, EmulatorConfig};

let mut machine = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)?;

// Where the mode setup's own structures end — long mode builds identity page
// tables, and code written over them never runs.
let code = CpuSetupMode::FlatLong64.first_free_physical_address();

machine.mem_write(code, &[0x48, 0xC7, 0xC0, 0x2A, 0x00, 0x00, 0x00, 0xF4])?; // mov rax,42 ; hlt
machine.reg_write(X86Reg::Rsp, 0x0010_0000);
machine.emu_start(code, None, None, Some(16))?;
assert_eq!(machine.reg_read(X86Reg::Rax), 42);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Physical and virtual addresses are different verbs:

```rust,no_run
# use rusty_box::cpu::CpuSetupMode;
# use rusty_box::emulator::{Emulator, EmulatorConfig};
# let mut machine = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)?;
# let addr = CpuSetupMode::FlatLong64.first_free_physical_address();
let mut buf = [0u8; 4];
machine.mem_read(addr, &mut buf)?;          // guest-physical
machine.virt_read(addr, &mut buf)?;         // through the page tables
let byte = machine.mem_read_u8(addr)?;      // one byte, physical
let phys = machine.virt_to_phys(addr)?;     // walk without reading
# let _ = (byte, phys);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Registers are named by one enum, and MSRs by index:

```rust,no_run
# use rusty_box::cpu::{CpuSetupMode, X86Reg};
# use rusty_box::emulator::{Emulator, EmulatorConfig};
# let mut machine = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)?;
machine.reg_write(X86Reg::Rcx, 0x1234);
assert_eq!(machine.reg_read(X86Reg::Rcx), 0x1234);

// Sub-registers address the part they name: Spl, Bpl, Sil and Dil are the low
// bytes of RSP, RBP, RSI and RDI, not the high bytes of AX..DX.
machine.reg_write(X86Reg::Sil, 0xAB);
assert_eq!(machine.reg_read(X86Reg::Rsi) & 0xFF, 0xAB);

// The whole architectural state in one struct, for a snapshot of a moment.
let state = machine.cpu_snapshot();
println!("rip={:#x} after {} instructions", state.rip, state.icount);

// MSRs by index. Both directions can refuse.
let efer = machine.msr_read(0xC000_0080)?;
machine.msr_write(0xC000_0080, efer)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** An MSR is refused when this processor's CPU model does not have it
— the same answer the guest gets, which is a fault rather than a zero. A write
the processor would take and then ignore is refused too, rather than answered
`Ok`: `IA32_APERF` and `IA32_MPERF` always, and `IA32_TSC_DEADLINE` while the
local APIC's timer is not in TSC-deadline mode. A host cannot see "ignored" any
other way.

An exit address ends a run before its budget does:

```rust,no_run
# use rusty_box::cpu::{CpuSetupMode, X86Reg};
# use rusty_box::emulator::{Emulator, EmulatorConfig};
# let mut machine = Emulator::new_with_mode(EmulatorConfig::default(), CpuSetupMode::FlatLong64)?;
# let code = CpuSetupMode::FlatLong64.first_free_physical_address();
# machine.mem_write(code, &[0x90, 0x90, 0x90, 0x90, 0xF4])?;
machine.set_exits(&[code + 2]);
machine.emu_start(code, None, None, Some(64))?;
assert_eq!(machine.reg_read(X86Reg::Rip), code + 2);
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 7. Watch what the guest does

An observer is a type, not a callback list: it rides along in the machine's own
type, so a machine that observes nothing pays nothing.

```rust,no_run
use rusty_box::cpu::{CpuSetupMode, Instruction, Instrumentation};
use rusty_box::emulator::{Emulator, EmulatorConfig};

#[derive(Default)]
struct Counter {
    executed: u64,
}

impl Instrumentation for Counter {
    fn before_execution(&mut self, _rip: u64, _instr: &Instruction) {
        self.executed += 1;
    }
}

let mut machine = Emulator::<Counter>::new_with_mode_and_instrumentation(
    EmulatorConfig::default(),
    CpuSetupMode::FlatLong64,
    Counter::default(),
)?;
# let code = CpuSetupMode::FlatLong64.first_free_physical_address();
# machine.mem_write(code, &[0x90, 0x90, 0x90, 0xF4])?;
machine.emu_start(code, None, None, Some(8))?;
println!("{} instructions", machine.instrumentation().executed);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The trait's hooks come in families, and every one has a do-nothing default:

- **execution** — `before_execution`, `after_execution`, `repeat_iteration`,
  `block_start`, `opcode`;
- **control flow** — `branch`, `interrupt`, `exception`, `hwinterrupt`,
  `pre_syscall`, `invalid_instruction`;
- **memory** — `lin_access`, `phy_access`, `mem_unmapped`,
  `mem_perm_violation`, `clflush`, `prefetch_hint`;
- **machine** — `inp`, `outp`, `cpuid`, `wrmsr`, `hlt`, `mwait`, `reset`,
  `tlb_cntrl`, `cache_cntrl`, `vmexit`.

`active_hooks()` declares which families an observer uses, and the processor
skips dispatch for the rest. Leave it at its default to receive everything.

**Limits.** Observation is the interpreter's. A machine running its guest on
the hypervisor engine executes instructions the host never sees, so nothing
here fires for them. One observer instance cannot be shared by several
processors: an SMP machine takes a factory that mints one each
(`MachineBuilder::tracer_factory`).

---

## 8. Save and restore a machine

A snapshot is the whole machine — processors, memory and devices — written to
any `Write` and read back from any `Read`.

```rust,no_run
# use rusty_box::cpu::X86Reg;
# use rusty_box::emulator::{EmulatorConfig, MachineBuilder};
# let bios = vec![0xF4u8; 0x10000];
# let mut machine = MachineBuilder::new(EmulatorConfig::default()).bios(&bios).build()?;
let mut saved = Vec::new();
machine.save_snapshot(&mut saved)?;
std::fs::write("machine.snap", &saved)?;

// Later, into a machine built from the same configuration.
let image = std::fs::read("machine.snap")?;
machine.restore_snapshot(&mut std::io::Cursor::new(&image))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** A snapshot needs a machine that went through hardware
initialisation: a CPU-only machine has no devices to serialise and says so
rather than writing a partial image. A restore replaces everything, so host
input that was in flight when the image was taken is not carried over — type
again after restoring. The image records the machine's shape; restoring it into
a machine configured differently is refused rather than half-applied.
Snapshots are the interpreter's.

---

## 9. What differs on the hypervisor engine

On Windows, `rusty_box_whp_engine` runs the guest on the host processor.
`FastMachine` owns the threads: one per processor plus a device thread.

```rust,ignore
use rusty_box::cpu::X86Reg;
use rusty_box_whp_engine::{FastMachine, WhpEngine};

// A machine built on the hypervisor engine: MachineBuilder::build_on::<WhpEngine>().
let mut fast = FastMachine::adopt(machine)?;   // adopted PAUSED: nothing runs yet

// Arrange the guest before it starts, then let it go.
fast.with_machine(|machine| {
    machine.reg_write(X86Reg::Rip, 0x7C00);
})?;
fast.resume()?;

// Pause to look. Registers read while paused are the guest's; writes reach it
// on resume.
fast.pause()?;
let rip = fast.with_machine(|machine| machine.reg_read(X86Reg::Rip))?;
```

`with_machine` refuses rather than lying:

- `NotPaused` — the machine is running, so its registers are the partition's;
- `RegistersNotInTheMachine` — a processor parked without them;
- `Uncarried` / `UncarriedMsr` — the closure wrote a register this engine does
  not carry into the partition. The write is put back and the answer dropped;
- `InterruptAlreadyPlaced` — a write to `IF` under an interrupt already placed
  for delivery;
- `Wedged` — a processor did not come back;
- `NoInstructionCount` — a budget in instructions, which nothing on hardware
  counts. Size runs in time instead;
- `DeviceClockIsTicks` — the machine keeps device time in ticks and has no
  host clock for these threads.

`with_machine_while_running` is the same verb for a machine that must not be
paused; it is the narrower one and refuses more.

**Interpreter only.** Snapshots, the instrumentation hooks, `mmio_map` and
`mem_protect`. What a guest may do on the hypervisor is listed in
[whp-guest-capabilities.md](whp-guest-capabilities.md).

---

## 10. Drive the graphical runner

Sometimes the thing to automate is the front end, not the guest. The runner
builds with egui's inspection protocol, which reads its widget tree and
synthesizes real input events.

Launch it with an inspection address:

```bash
EGUI_INSPECTION=127.0.0.1:5731 cargo run --release -p rusty_box_gui -- --no-config --display egui --bios cpp_orig/bochs/bochs/bios/BIOS-bochs-latest --vga-bios cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin --cdrom image.iso --boot cdrom --memory-mib 256
```

`EGUI_INSPECTION=1` means `127.0.0.1:5719`; any other value is a `host:port`.
Pick a port of your own if another egui application on the machine already
holds the default.

Then attach with an inspection client and work in a loop: read the tree to find
a widget, click it, screenshot to confirm. The runner starts **Stopped** on its
launcher — power the VM on before a guest console exists.

---

## Where to look next

- [rusty_box/examples/README.md](../rusty_box/examples/README.md) — runnable
  programs, including a full Alpine boot.
- `rusty_box/tests/api_examples.rs` — the public API exercised the way a
  caller writes it, with assertions.
- [bochs-parity-divergences.md](bochs-parity-divergences.md) — where this
  emulator deliberately differs from Bochs, and what a guest can observe.
