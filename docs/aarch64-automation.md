# Driving an AArch64 guest from Rust

`rusty_box_aarch64` runs QEMU's `virt` board — a Cortex-A57-shaped ARMv8.0
processor, a GICv2, a PL011 UART, flash, an RTC and a power button, and,
when asked, QEMU's virtio display, keyboard and tablet — without a human at
the console. A program builds a machine, boots UEFI firmware or a Linux
kernel on it, reads the serial console or the screen, types at it, points
at it, and watches what it does, all in process.

The verbs are the ones [automation.md](automation.md) teaches for the x86
machine: `step` with a `RunBudget`, `serial(n)` and `keyboard()` handles,
registers and memory by name, an observer type parameter, snapshots. Where
the two machines share a word — `RunBudget`, `Progress`, the key vocabulary
and the `KeyboardInput` trait — it is the same type, from `rusty_box_core`,
so code written against it drives either.

Every Rust block here is compiled by
`cargo test -p rusty_box_aarch64 --features std --doc`.

```toml
[dependencies]
rusty_box_aarch64 = { git = "https://github.com/r3bb1t/rusty_box", features = ["std"] }
```

Without `std` (or `alloc`) the crate still builds everything here except
`build()` and `StopHandle`: a `no_std` host lends the machine its memory with
`build_in` and stops it through a `&'static AtomicBool`.

---

## 1. Build a machine

`MachineBuilder` assembles one. Its type records how the guest starts, so a
builder that has not been told cannot build, and a kernel command line
cannot be handed to firmware.

UEFI firmware built for QEMU's `virt` (EDK2 `ArmVirtQemu`, Debian's AAVMF):

```rust,no_run
use rusty_box_aarch64::{MachineBuilder, RamSize, VirtConfig};

let code = std::fs::read("/usr/share/AAVMF/AAVMF_CODE.no-secboot.fd")?;
let vars = std::fs::read("/usr/share/AAVMF/AAVMF_VARS.fd")?;
let mut machine = MachineBuilder::new(VirtConfig::default())
    .ram(RamSize::mib(512).expect("a size the board maps"))
    .firmware(&code)
    .variables(&vars)
    .build()?;
// Nothing has run: the processor sits at the reset vector.
assert_eq!(machine.ticks(), 0);
# Ok::<(), Box<dyn std::error::Error>>(())
```

A Linux kernel, loaded the way QEMU's `-kernel` loads one without firmware:
the `Image` at its load offset, the initramfs above it, the device tree with
`/chosen` filled in at the 2 MiB boundary after the initramfs, and the
processor at the kernel's entry with `x0` pointing at the tree.

```rust,no_run
use rusty_box_aarch64::boot::{LinuxImage, SERIAL_CONSOLE_COMMAND_LINE};
use rusty_box_aarch64::{MachineBuilder, VirtConfig};

let kernel = std::fs::read("Image")?;
let initrd = std::fs::read("initramfs.cpio.gz")?;
let mut machine = MachineBuilder::new(VirtConfig::default())
    .linux(LinuxImage::parse(&kernel)?)
    .initrd(&initrd)
    .command_line(SERIAL_CONSOLE_COMMAND_LINE)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Or nothing at all: a bare machine starts at the base of RAM, for a host that
writes its own code there (section 5).

The device tree is QEMU's own for this board unless `device_tree(..)` gives
another; either way its memory node is set to the RAM the machine has, and
`machine.device_tree()` returns what the guest was given.
`boot::fdt::property` reads a property back out of it.

**Owned or lent.** `build()` gives a machine that owns its memory
(`VirtMachine<OwnedStorage>`, movable to another thread). `build_in` runs in
memory the caller lends, and borrows the firmware and kernel images where
they are (either way the machine itself, without its RAM, is built and
returned by value: about 170 KiB with `alloc`, whose trace cache is on the
heap, and about half a megabyte without, whose smaller trace cache is in
place — so build and run it on a thread with a few megabytes of stack
rather than on a 1 MiB Windows main thread; the examples use 16 MiB):

```rust
use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};

let mut ram = vec![0u8; 16 << 20];
let machine = MachineBuilder::new(VirtConfig::default())
    .ram(RamSize::mib(16).expect("a valid size"))
    .bare()
    .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
# drop(machine);
# Ok::<(), rusty_box_aarch64::BuildError>(())
```

**Limits.** RAM is a whole number of pages from 64 KiB up to the 255 GiB the
board can map. A flash image may be shorter than its 64 MiB bank: the rest
reads as erased flash. `BuildError` says what did not fit.

---

## 2. The serial console

`virt`'s console is its PL011 UART, and `serial(0)` borrows it — the only
port the board has, so `serial(1)` is `None`.

```rust,no_run
# use rusty_box_aarch64::{MachineBuilder, VirtConfig};
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(rusty_box_aarch64::RamSize::mib(16).unwrap()).bare()
#     .build_in(rusty_box_aarch64::MemoryLoan { ram: &mut ram, vars: &mut [] })?;
if let Some(mut console) = machine.serial(0) {
    let bytes: Vec<u8> = console.take_output().collect();
    print!("{}", String::from_utf8_lossy(&bytes));

    // Input waits where a QEMU character backend's would and reaches the
    // UART's FIFO as the guest reads, so a whole line goes in at once.
    let line = b"uname -a\r";
    let sent = console.send(line);
    if sent < line.len() {
        println!("the guest stopped reading; send {:?} later", &line[sent..]);
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Waiting for a prompt is a loop of "step a little, look":

```rust,no_run
use rusty_box_aarch64::{GuestStorage, RunBudget, VirtMachine};

fn wait_for<S: GuestStorage>(machine: &mut VirtMachine<S>, needle: &str, rounds: usize) -> bool {
    let mut seen = Vec::new();
    for _ in 0..rounds {
        let outcome = machine.step(RunBudget::Ticks(5_000_000));
        if let Some(mut console) = machine.serial(0) {
            seen.extend(console.take_output());
        }
        if String::from_utf8_lossy(&seen).contains(needle) {
            return true;
        }
        if outcome.is_terminal() {
            return false;
        }
    }
    false
}
```

**Limits.** The output buffer holds 64 KiB; a host that never drains loses
the oldest bytes, never stalls the guest. Input is bounded too but never
lost: `send` says how much it took. `virt` has no display, so there is no
screen to read — the console is the screen.

---

## 3. Keys and text

`keyboard()` is the machine's keyboard. `virt` has no keyboard device of its
own, so on a machine built without a virtio keyboard (section 8) it is the
keyboard of the terminal on the UART, as `terminal_keyboard()` always is: key
events become the bytes an xterm sends for them — letters as characters,
Ctrl as control codes, cursor and function keys as escape sequences. It
implements `KeyboardInput`, the trait the x86 machine's PS/2 keyboard
implements too:

```rust
use rusty_box_aarch64::{Key, KeyAction, KeyboardInput, Typed};

/// Log in on any machine's keyboard, x86 or AArch64.
fn log_in<K: KeyboardInput>(keyboard: &mut K) -> bool {
    matches!(keyboard.type_text("root\n"), Typed::All { .. })
}

# use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(RamSize::mib(16).unwrap()).bare()
#     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
assert!(log_in(&mut machine.keyboard()));

// Each verb says whether the input took it. Nothing runs on this bare
// machine to read it, but the 4 KiB input buffer has room for all of these.
let mut keyboard = machine.keyboard();
assert!(keyboard.tap(Key::Up)); // ESC [ A
assert!(keyboard.key(Key::CtrlL, KeyAction::Press));
assert!(keyboard.tap(Key::C)); // 0x03
assert!(keyboard.key(Key::CtrlL, KeyAction::Release));
// Bytes no key produces, as a paste would send them.
let pasted = keyboard.bytes("é".as_bytes());
assert_eq!(pasted, 2, "both bytes of the character fit");
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** `type_text` maps a US layout and reports a character with no key
(`Typed::Unmappable`) rather than guessing; send its bytes with `bytes`.
The key verbs are whole or nothing: a key, tap or character whose bytes do
not all fit in the input buffer is refused (`false`, `Typed::Refused`),
never cut in half. `bytes` is a paste: it takes as much as fits and returns
the count, which can end inside a multi-byte character, so resume from that
byte.

---

## 4. Run, step and stop

A run is bounded by a budget and ends with a reason.

```rust
# use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
use rusty_box_aarch64::{RunBudget, StopReason};
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(RamSize::mib(16).unwrap()).bare()
#     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
// Instructions, when counting work; ticks, when waiting for guest time.
let outcome = machine.step(RunBudget::Instructions(1_000));
println!("retired {:?}", outcome.progress.instructions());
let outcome = machine.step(RunBudget::Ticks(1_000));

match outcome.stop {
    StopReason::BudgetExhausted => {}          // keep going
    StopReason::Halted => {}                   // WFI with nothing scheduled
    StopReason::ExitReached => {}              // an exit address (section 5)
    StopReason::StopRequested => {}            // a stop handle fired
    StopReason::PoweredOff => {}               // PSCI SYSTEM_OFF
    StopReason::ResetRequested => {}           // PSCI SYSTEM_RESET, if asked to stop on it
    StopReason::ProcessorOff => {}             // PSCI CPU_OFF
    StopReason::Undefined(_) | StopReason::BusFault { .. } => {} // debug stops
}
if outcome.is_terminal() {
    println!("done");
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

A waiting machine says how far it may skip, and a stop handle ends a run
from another thread:

```rust,no_run
# use rusty_box_aarch64::{MachineBuilder, VirtConfig, RunBudget, StopReason};
# let mut machine = MachineBuilder::new(VirtConfig::default()).bare().build()?;
if let Some(ticks) = machine.ticks_to_next_timer_deadline() {
    let woken = machine.step(RunBudget::Ticks(ticks));
    println!("after the deadline: {:?}", woken.stop);
}

let handle = machine.stop_handle();
std::thread::spawn(move || {
    std::thread::sleep(std::time::Duration::from_secs(5));
    handle.stop();
});
let outcome = machine.step(RunBudget::Instructions(u64::MAX));
if outcome.stop == StopReason::StopRequested {
    println!("stopped after {:?} instructions", outcome.progress.instructions());
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

The power controls:

```rust
# use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
use rusty_box_aarch64::PowerState;
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(RamSize::mib(16).unwrap()).bare()
#     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
assert_eq!(machine.power().state(), PowerState::Running);
// QEMU's gpio-key on GPIO line 3, held for 100 ms of guest time. A guest
// that handles KEY_POWER shuts down; one that does not keeps running.
machine.power().press_power_button();
// A system reset: firmware from its reset vector, or a direct kernel loaded
// afresh with its initrd and device tree.
machine.power().reset();
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** An instruction budget retires exactly that many instructions,
across calls as within one; a tick budget advances guest time exactly. A
processor in `WFI` retires nothing, so give a waiting guest ticks. Time is
the instruction count at `VirtConfig::ips`, never the host clock: the same
inputs give the same run however it is sliced.

---

## 5. Memory and registers

A bare machine is the shortest path to running your own code:

```rust
use rusty_box_aarch64::board::RAM_BASE;
use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, Reg, StopReason, VirtConfig};

let mut ram = vec![0u8; 16 << 20];
let mut machine = MachineBuilder::new(VirtConfig::default())
    .ram(RamSize::mib(16).expect("a valid size"))
    .bare()
    .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;

// mov x0, #42 ; add x0, x0, #1 ; b .
for (i, word) in [0xD280_0540u32, 0x9100_0400, 0x1400_0000].iter().enumerate() {
    machine.mem_write(RAM_BASE + 4 * i as u64, &word.to_le_bytes())?;
}
// Unicorn's uc_emu_start: from `begin` until `until`, or `count` instructions.
let outcome = machine.emu_start(RAM_BASE, Some(RAM_BASE + 8), None);
assert_eq!(outcome.stop, StopReason::ExitReached);
assert_eq!(machine.reg_read(Reg::X0), 43);

// Exit addresses end every run before the instruction there executes.
machine.set_exits(&[RAM_BASE + 4])?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Physical and virtual addresses are different verbs. A virtual access walks
the guest's current page tables as a data access would — with no permission
check, and descriptors read from RAM only. It reads the tables, never the
TLB, so a mapping the guest rewrote without a `TLBI` already shows its new
translation here:

```rust
# use rusty_box_aarch64::board::RAM_BASE;
# use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(RamSize::mib(16).unwrap()).bare()
#     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
let mut buf = [0u8; 4];
machine.mem_read(RAM_BASE, &mut buf)?;           // guest-physical RAM
machine.virt_read(RAM_BASE, &mut buf)?;          // through the page tables
let phys = machine.virt_to_phys(RAM_BASE)?;      // walk without reading
assert_eq!(phys, RAM_BASE, "the MMU is off out of reset");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Registers are named by `Reg`, SIMD&FP registers by `VReg`, and system
registers by encoding (`SysReg`, with constants for the common ones). A host
reads and writes with EL1's privilege and none of the controls that trap a
guest; a write has the effect an `MSR` would (a translation register flushes
the TLB):

```rust
# use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
use rusty_box_aarch64::{Reg, SysReg, SysRegRefusal, VReg};
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(RamSize::mib(16).unwrap()).bare()
#     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
machine.reg_write(Reg::X1, 0x1234);
machine.vreg_write(VReg::new(0).expect("V0"), 1 << 100);
assert_eq!(machine.sysreg_read(SysReg::MIDR_EL1), Ok(0x411F_D070));

match machine.sysreg_write(SysReg::MIDR_EL1, 0) {
    Ok(()) => {}
    Err(SysRegRefusal::Absent) => println!("this processor has no such register"),
    Err(SysRegRefusal::ReadOnly) => println!("read-only"),
    Err(SysRegRefusal::WriteOnly) => println!("write-only"),
}

// The whole architectural state at one moment.
let state = machine.cpu_snapshot();
println!("pc={:#x} at EL{}", state.pc, state.pstate.el.index());
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** `mem_read` and `mem_write` reach RAM only; a device window is
the guest's to touch, since a host read of a UART register would consume the
byte it returned.

---

## 6. Watch what the guest does

An observer is a type, not a callback list: it rides in the machine's own
type, so a machine that observes nothing pays nothing.

```rust
use rusty_box_aarch64::instrumentation::{ExceptionTaken, MemoryAccess};
use rusty_box_aarch64::{Instrumentation, MachineBuilder, MemoryLoan, RamSize, VirtConfig};

#[derive(Default)]
struct Counter {
    executed: u64,
    exceptions: u64,
    accesses: u64,
}

impl Instrumentation for Counter {
    fn before_execution(&mut self, _pc: u64, _insn: u32) {
        self.executed += 1;
    }
    fn exception(&mut self, _taken: ExceptionTaken) {
        self.exceptions += 1;
    }
    fn memory_access(&mut self, _access: MemoryAccess) {
        self.accesses += 1;
    }
}

let mut ram = vec![0u8; 16 << 20];
let mut machine = MachineBuilder::new(VirtConfig::default())
    .ram(RamSize::mib(16).expect("a valid size"))
    .bare()
    .instrumentation(Counter::default())
    .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
let outcome = machine.step(rusty_box_aarch64::RunBudget::Instructions(8));
assert_eq!(outcome.progress.instructions(), Some(8));
println!("{} instructions", machine.instrumentation().executed);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The hooks: `before_execution`, `after_execution`, `branch` and
`wait_for_interrupt` for execution; `exception` for every synchronous
exception, IRQ and FIQ taken (with the vector, the syndrome and the fault
address); `memory_access` for every data access that completes on the
physical bus. Each defaults to nothing.

---

## 7. Save and restore

A snapshot is the whole machine — processor (TLB included), devices, RAM and
the variable store — written to any `SnapWrite` and read back from any
`SnapRead`. A `Vec<u8>` is one, and so is a byte slice.

```rust
# use rusty_box_aarch64::{MachineBuilder, MemoryLoan, RamSize, VirtConfig};
# let mut ram = vec![0u8; 16 << 20];
# let mut machine = MachineBuilder::new(VirtConfig::default())
#     .ram(RamSize::mib(16).unwrap()).bare()
#     .build_in(MemoryLoan { ram: &mut ram, vars: &mut [] })?;
let mut image = Vec::new();
machine.save_snapshot(&mut image)?;
// ... run on, then go back:
machine.restore_snapshot(&mut image.as_slice())?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Limits.** Firmware and kernel images are recorded by hash, not copied, so
an image restores only into a machine built the same way — the same RAM,
images, boot method and clock — and is refused, with nothing changed, by any
other. Host-side state stays with the host: serial input not yet taken is
dropped and the terminal's held modifiers released, so type again after a
restore; the display's next refresh sends the whole picture. A restore that
fails part-way through an image leaves the machine switched off until
`power().reset()`.

---

## 8. A screen, a keyboard and a pointer

QEMU's `virt` takes a display and input devices as `-device` options. The
builder takes the three QEMU's display front ends drive — `virtio-gpu-device`
(2D, with QEMU's EDID), `virtio-keyboard-device` and `virtio-tablet-device`
— and plugs each into the highest free virtio-mmio transport, as QEMU does.
A board with them has non-legacy transports (QEMU's
`-global virtio-mmio.force-legacy=false`): these are virtio 1.0 devices,
which Linux's drivers refuse on a legacy transport.

```rust
use rusty_box_aarch64::{
    MachineBuilder, NullDisplay, PointerInput, PointerPos, Refreshed, Resolution, VirtConfig,
};

let mut machine = MachineBuilder::new(VirtConfig::default())
    .bare()
    .virtio_display(Resolution::new(1024, 768))
    .virtio_keyboard()
    .virtio_tablet()
    .build()?;

let mut display = machine.display().expect("built with a display");
// The size the display asks the guest for, as a QEMU window of that size
// would: in its display information and its EDID.
assert_eq!(display.preferred_resolution(), Some(Resolution::new(1024, 768)));
// No driver has set a scanout yet: the screen is blank at that size.
assert_eq!(display.resolution(), None);
assert_eq!(display.refresh(&mut NullDisplay), Refreshed::Frame);
assert_eq!(display.refresh(&mut NullDisplay), Refreshed::Unchanged);
// A window resized: a display event the guest's driver answers by
// changing mode.
display.request_resolution(Resolution::new(1280, 800));

// The tablet places the pointer anywhere on the picture, whatever its size.
let mut pointer = machine.pointer().expect("built with a tablet");
let middle = PointerPos::from_pixel(512, 384, 1024, 768);
// Nothing has driven the tablet on this bare machine, so it takes nothing.
assert!(!pointer.move_to(middle));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`refresh` pushes what changed since the last refresh into any
`DisplaySink` — the trait the x86 machine's VGA pushes into, so a front end
draws either — a new size first, then the changed rectangle as RGBA, with
the guest's hardware cursor drawn over it. `keyboard()` on such a machine is
the virtio keyboard: keys reach the guest as Linux input events, each
followed by a sync, as QEMU's front ends send them; `terminal_keyboard()`
still types on the serial console. `pointer()` is the tablet: positions on
`PointerPos`'s 0 to `0x7FFF` scale (`from_pixel` scales a pixel of the
picture), buttons, and wheel detents, through `PointerInput`.

**Limits.** An input device takes nothing until its driver is ready, and an
event only when the driver has queued a buffer for each of its parts: a key
or a move it cannot take whole is refused (`false`), never cut, so offer it
again after stepping. UEFI firmware built for `virt` (`ArmVirtQemu`) binds the
display but, as on QEMU, draws its console only on PCI displays, so the
screen stays blank until the operating system's driver takes it. The display
needs `alloc`.

---

## 9. A disk

`virtio_disk` plugs in QEMU's `virtio-blk-device` over a raw image, as
`-drive file=disk.img,format=raw,if=none,id=hd0 -device
virtio-blk-device,drive=hd0` does; it takes the next transport down after
the display and input devices, and Linux sees it as `/dev/vda`. The image is
host memory (`Disk::memory`) or a host file read and written in place
(`Disk::file`, with `std`), and the machine keeps it.

```rust
use rusty_box_aarch64::{Disk, MachineBuilder, VirtConfig};

let mut machine = MachineBuilder::new(VirtConfig::default())
    .bare()
    .virtio_disk(Disk::memory(vec![0; 1 << 20]))
    .build()?;

let disk = machine.disk().expect("built with a disk");
assert_eq!(disk.sectors(), 2048);
// What the guest wrote, for a memory disk.
assert_eq!(disk.memory_bytes().map(<[u8]>::len), Some(1 << 20));
// A file disk: make what the guest wrote durable before the host lets go of
// it, and hear of any host error a guest request met.
let disk = machine.disk_mut().expect("built with a disk");
assert!(disk.take_io_error().is_none());
disk.flush()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The device is the one QEMU makes for a drive given every default: one queue
of 256 entries, a write-back cache the driver may turn off, discard requests
ignored, write-zeroes honoured, and no serial number. **Limits.** A request
completes before the driver's notification returns, where QEMU's block layer
completes it later; the guest sees the same results, sooner. A host error is
an I/O error to the guest, which is QEMU's default except for a write that
finds the host's disk full, where QEMU stops the machine until the user
resumes it. A snapshot holds the disk's size but not its contents: it
restores onto the disk as it is, like a QEMU migration over shared storage.
A disk needs `alloc`.

---

## Code for any machine

`Machine` (with `RunOutcome` and `SerialPort`) is the part every interpreted
rusty_box machine shares — step, the serial console, the keyboard — so a
host can write its boot-and-log-in logic once and run it on an x86 or an
AArch64 guest. Each machine still answers in its own outcome type; the trait
asks only how far it got and whether it is finished. An x86 machine on the
hypervisor engine is deliberately not a `Machine`: it has its own driver, and
the type system keeps it out rather than a `step` that refuses at run time.

```rust
use rusty_box_aarch64::{KeyboardInput, Machine, RunBudget, RunOutcome, SerialPort, Typed};

fn type_after_prompt<M: Machine>(machine: &mut M, prompt: &[u8], text: &str) -> Result<bool, M::Error> {
    let mut seen = Vec::new();
    for _ in 0..1_000 {
        let outcome = machine.step(RunBudget::Ticks(1_000_000))?;
        if let Some(mut console) = machine.serial(0) {
            seen.extend(console.take_output());
        }
        if seen.windows(prompt.len()).any(|w| w == prompt) {
            return Ok(matches!(machine.keyboard().type_text(text), Typed::All { .. }));
        }
        if outcome.is_terminal() {
            break;
        }
    }
    Ok(false)
}
```

An AArch64 machine's runs always end with an outcome, so its `Error` is
`Infallible`; the x86 machine's is `rusty_box::Error`.

## How this compares with the x86 machine

| | x86 (`rusty_box`) | AArch64 (`rusty_box_aarch64`) |
|---|---|---|
| Budget and progress | `RunBudget`, `Progress` | the same types |
| Shared verbs | `Machine`, `RunOutcome`, `SerialPort` | the same traits |
| Keyboard | PS/2 scancodes, `KeyboardInput` | xterm bytes on the UART, or the virtio keyboard; `KeyboardInput` |
| Screen | VGA text and graphics, into a `DisplaySink` | the serial console, or virtio-gpu into a `DisplaySink` |
| Mouse | PS/2 | the virtio tablet, `PointerInput` |
| Disk | ATA and ATAPI on the IDE channels | virtio-blk over a `Disk` |
| Power | ACPI power button, reset | `gpio-key` power button, PSCI, reset |
| Registers | `X86Reg`, MSRs by index | `Reg`, `VReg`, `SysReg` by encoding |
| Observer | `Instrumentation` | `Instrumentation` (its own hooks) |
| Snapshots | `std::io` streams | `SnapWrite` / `SnapRead` (no `std` needed) |

## Where to look next

- `rusty_box_aarch64/examples/` — `uefi_virt` boots EDK2 to the UEFI shell
  and `linux_virt` boots a kernel to a login prompt, each with a console
  script (`--until`, `--type`).
- `rusty_box_aarch64/tests/api.rs` — this API exercised with assertions.
- [rusty_box_aarch64/README.md](../rusty_box_aarch64/README.md) — what the
  processor and board implement, and how they are verified.
