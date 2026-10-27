//! The Arm `virt` machine's runs (`[vm] machine = "arm-virt"`): in the shell,
//! on the VM's window under its run control; and without the shell, on this
//! process's standard output and, for `display = "terminal"`, its terminal.
//!
//! Guest time keeps to the host clock, as QEMU's virtual clock does for
//! `virt`: a slice of guest time that ran ahead of the host's is slept off, so
//! a waiting guest leaves the host idle and its timers fire at their real
//! rate, and a guest the interpreter cannot keep up with runs flat out. (The
//! PC keeps Bochs's default, `clock: sync=none`, instead; each machine follows
//! the emulator it is ported from.)

#[cfg(feature = "gui-egui")]
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
#[cfg(feature = "gui-egui")]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusty_box_aarch64::boot::LinuxImage;
#[cfg(feature = "gui-egui")]
use rusty_box_aarch64::{Key, KeyAction, PointerInput, Refreshed};
use rusty_box_aarch64::{
    BatchOutcome, ClockHz, Disk, KeyboardInput, MachineBuilder, OwnedStorage, RamSize, Resolution, RunBudget,
    StopReason, VirtConfig, VirtMachine,
};
#[cfg(feature = "gui-egui")]
use rusty_box::gui::{host_input::AbsolutePointerEvent, shared_display::SharedDisplay};

use crate::config::{ArmBoot, ArmVirtConfig, DiskAccess, VirtDisk};
#[cfg(feature = "gui-egui")]
use crate::debugger::{
    control::{AfterRun, BetweenRuns, RunControl, Task},
    dump::dump_ram,
    search::FindOutcome,
    serve::{MachineAfter, Next},
};
use crate::error::RunError;
use crate::runner::{read_required_file, Console, RunSummary};

/// The machine every run here builds.
type Virt = VirtMachine<OwnedStorage>;

/// How much guest time one slice covers.
///
/// The slice is the latency ceiling for every host action — "Power Off",
/// "Pause" and a keystroke are all seen between slices — and the grain the
/// pacing sleeps in. Ten milliseconds is under what a person notices on a key
/// and a quarter of the frame interval.
const SLICE: Duration = Duration::from_millis(10);

/// How often the window is redrawn: 25 frames a second, the rate the PC's
/// display keeps (Bochs vga.cc `vga_update_interval`).
#[cfg(feature = "gui-egui")]
const FRAME_INTERVAL: Duration = Duration::from_millis(40);

/// How often the status bar's instruction rate is brought up to date.
#[cfg(feature = "gui-egui")]
const IPS_INTERVAL: Duration = Duration::from_secs(1);

/// How far guest time may fall behind the host's before the pacing stops
/// owing it: past this the guest is measured from now, so a guest that was
/// slower than real time for a while does not then race its timers to catch
/// up.
const MAX_LAG: Duration = Duration::from_millis(100);

/// How long host input waits for a guest that takes none of it before it is
/// dropped: a guest that reads no input for that long (its driver not loaded
/// yet) is not then sent a burst of stale keystrokes. A burst the guest is
/// taking — a paste, or the keys one frame of the window delivers at once —
/// waits however long it is.
#[cfg(feature = "gui-egui")]
const STALE_INPUT: Duration = Duration::from_secs(2);

/// The most host input events of each kind that wait at once, which bounds
/// the memory a window can queue in under [`STALE_INPUT`]; past it the
/// oldest is dropped.
#[cfg(feature = "gui-egui")]
const INPUT_BACKLOG: usize = 1 << 16;

/// What a run boots, read from its files.
struct BootFiles {
    boot: BootImage,
}

enum BootImage {
    Firmware {
        code: Vec<u8>,
        vars: Option<VariableStore>,
    },
    Linux {
        kernel: Vec<u8>,
        kernel_path: PathBuf,
        initrd: Option<Vec<u8>>,
        command_line: String,
        device_tree: Option<Vec<u8>>,
    },
}

/// The UEFI variable store a firmware boot reads, and the file it is written
/// back to: QEMU's second flash bank, which keeps what the firmware programs
/// into it.
struct VariableStore {
    path: PathBuf,
    image: Vec<u8>,
}

impl BootFiles {
    fn read(config: &ArmVirtConfig) -> Result<Self, RunError> {
        let boot = match &config.boot {
            ArmBoot::Unset => return Err(RunError::MissingVirtBoot),
            ArmBoot::Firmware { code, vars } => BootImage::Firmware {
                code: read_required_file("firmware", code)?,
                vars: match vars {
                    Some(path) => Some(VariableStore {
                        path: path.clone(),
                        image: read_required_file("UEFI variable store", path)?,
                    }),
                    None => None,
                },
            },
            ArmBoot::Linux {
                kernel,
                initrd,
                command_line,
                device_tree,
            } => BootImage::Linux {
                kernel: read_required_file("kernel", kernel)?,
                kernel_path: kernel.clone(),
                initrd: match initrd {
                    Some(path) => Some(read_required_file("initrd", path)?),
                    None => None,
                },
                command_line: command_line.clone(),
                device_tree: match device_tree {
                    Some(path) => Some(read_required_file("device tree", path)?),
                    None => None,
                },
            },
        };
        Ok(Self { boot })
    }

    /// Writes the variable store back to its file when the firmware changed
    /// it, as QEMU's flash keeps what is programmed into it.
    fn persist(&self, machine: &Virt) -> Result<(), RunError> {
        let BootImage::Firmware {
            vars: Some(store), ..
        } = &self.boot
        else {
            return Ok(());
        };
        let now = machine.variable_store();
        let unchanged = now
            .split_at_checked(store.image.len())
            .is_some_and(|(image, rest)| {
                image == store.image.as_slice() && rest.iter().all(|&byte| byte == ERASED_FLASH)
            });
        if unchanged {
            return Ok(());
        }
        std::fs::write(&store.path, now).map_err(|source| RunError::VariableStoreWrite {
            path: store.path.clone(),
            source,
        })
    }
}

/// What erased flash reads as: the padding past a variable-store image
/// shorter than its bank.
const ERASED_FLASH: u8 = 0xFF;

/// The machine `config` describes, booting `files`.
fn build(config: &ArmVirtConfig, files: &BootFiles) -> Result<Virt, RunError> {
    let virt_config = VirtConfig {
        ips: ClockHz::new(u64::from(config.ips)).ok_or(RunError::ZeroValue { field: "virt.ips" })?,
        cpu: config.cpu,
        rtc_epoch_seconds: host_epoch_seconds(),
        ..VirtConfig::DEFAULT
    };
    let ram = RamSize::mib(u64::from(config.memory_mib)).ok_or(RunError::InvalidVirtMemory {
        memory_mib: config.memory_mib,
    })?;
    let mut builder = MachineBuilder::new(virt_config).ram(ram);
    if let Some(size) = config.display {
        builder = builder.virtio_display(Resolution::new(size.width, size.height));
    }
    if config.keyboard {
        builder = builder.virtio_keyboard();
    }
    if config.tablet {
        builder = builder.virtio_tablet();
    }
    if let Some(disk) = &config.disk {
        builder = builder.virtio_disk(open_disk(disk)?);
    }
    let built = match &files.boot {
        BootImage::Firmware { code, vars } => {
            let mut builder = builder.firmware(code);
            if let Some(store) = vars {
                builder = builder.variables(&store.image);
            }
            builder.build()
        }
        BootImage::Linux {
            kernel,
            kernel_path,
            initrd,
            command_line,
            device_tree,
        } => {
            if let Some(tree) = device_tree {
                builder = builder.device_tree(tree);
            }
            let image = LinuxImage::parse(kernel).map_err(|source| RunError::VirtKernel {
                path: kernel_path.clone(),
                source,
            })?;
            let mut builder = builder.linux(image).command_line(command_line);
            if let Some(initrd) = initrd {
                builder = builder.initrd(initrd);
            }
            builder.build()
        }
    };
    built.map_err(|source| RunError::VirtBuild { source })
}

/// The disk `disk` names, opened as QEMU opens a raw image (block/file-posix.c
/// `raw_open_common`): for reading and writing, or, when the guest may not
/// write it (`readonly=on`), for reading alone, so a file the host lets this
/// process only read can still be one.
fn open_disk(disk: &VirtDisk) -> Result<Disk, RunError> {
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .write(!disk.access.is_read_only())
        .open(&disk.path)
        .and_then(Disk::file)
        .map_err(|source| RunError::MediaAttach {
            kind: "disk",
            path: disk.path.clone(),
            source,
        })?;
    Ok(match disk.access {
        DiskAccess::ReadWrite => opened,
        DiskAccess::ReadOnly => opened.read_only(),
    })
}

/// Logs the host error the guest's disk last met, which the guest was told
/// of as an I/O error (QEMU's `BLOCK_IO_ERROR` event, `rerror=report`).
fn report_disk_error(machine: &mut Virt, config: &ArmVirtConfig) {
    let (Some(disk), Some(named)) = (machine.disk_mut(), &config.disk) else {
        return;
    };
    if let Some(error) = disk.take_io_error() {
        tracing::warn!(
            "the guest was told of an I/O error on its disk {}: {error}",
            named.path.display()
        );
    }
}

/// Makes what the guest wrote to its disk durable, as QEMU does when it
/// closes a drive (block.c `bdrv_close` flushing it).
fn close_disk(machine: &mut Virt, config: &ArmVirtConfig) -> Result<(), RunError> {
    report_disk_error(machine, config);
    let (Some(disk), Some(named)) = (machine.disk_mut(), &config.disk) else {
        return Ok(());
    };
    disk.flush().map_err(|source| RunError::DiskFlush {
        path: named.path.clone(),
        source,
    })
}

/// What the RTC reads at power-on: the host's time, as QEMU's PL031 does
/// with its default `-rtc base=utc`. A host clock set before 1970 gives the
/// board's own epoch instead.
fn host_epoch_seconds() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => since.as_secs(),
        Err(error) => {
            tracing::warn!("the host clock reads before 1970 ({error}); the guest's clock starts at 2026");
            VirtConfig::DEFAULT_RTC_EPOCH
        }
    }
}

/// [`SLICE`] in ticks at `ips`, never none.
fn slice_ticks(ips: u32) -> u64 {
    (u64::from(ips) * SLICE.as_millis() as u64 / 1_000).max(1)
}

/// Whether a slice's end lets the machine run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settled {
    RunOn,
    /// The guest switched itself off, or its only processor.
    Over,
}

/// How `outcome` leaves the machine. Every stop the runs here never ask for
/// is refused rather than run past: there is no exit address, no debugging
/// stop and no stop flag on the machine, and a guest's reset request resets
/// it in place, as QEMU's does.
fn settle(outcome: BatchOutcome) -> Result<Settled, RunError> {
    match outcome.stop {
        StopReason::BudgetExhausted | StopReason::Halted => Ok(Settled::RunOn),
        StopReason::PoweredOff | StopReason::ProcessorOff => Ok(Settled::Over),
        stop @ (StopReason::StopRequested
        | StopReason::ExitReached
        | StopReason::ResetRequested
        | StopReason::Undefined(_)
        | StopReason::BusFault { .. }) => Err(RunError::VirtStopped { stop }),
    }
}

/// Keeps guest time from running ahead of the host's.
struct Pacer {
    since: Instant,
    since_ticks: u64,
    ips: u64,
}

impl Pacer {
    fn new(machine: &Virt, ips: u32) -> Self {
        Self {
            since: Instant::now(),
            since_ticks: machine.ticks(),
            ips: u64::from(ips),
        }
    }

    /// Measures from here: after a pause, the host time the machine stood
    /// still is not owed to it.
    fn restart(&mut self, machine: &Virt) {
        self.since = Instant::now();
        self.since_ticks = machine.ticks();
    }

    /// Sleeps off guest time that ran ahead of the host's; forgives a lag
    /// past [`MAX_LAG`].
    fn keep_pace(&mut self, machine: &Virt) {
        let ticks = machine.ticks().saturating_sub(self.since_ticks);
        let nanos = u128::from(ticks) * 1_000_000_000 / u128::from(self.ips);
        let guest = Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX));
        let host = self.since.elapsed();
        match guest.checked_sub(host) {
            Some(ahead) => std::thread::sleep(ahead.min(SLICE)),
            None if host - guest > MAX_LAG => self.restart(machine),
            None => {}
        }
    }
}

/// The guest's serial output since the last call.
fn take_serial_output(machine: &mut Virt) -> Vec<u8> {
    match machine.serial(0) {
        Some(mut port) => port.take_output().collect(),
        None => Vec::new(),
    }
}

/// `bytes` on standard output, which every run mirrors the guest's serial
/// console to, as the PC's runs do.
fn mirror_to_stdout(bytes: &[u8]) {
    let mut out = io::stdout();
    if let Err(error) = out.write_all(bytes).and_then(|()| out.flush()) {
        tracing::warn!("serial stdout mirror failed: {error}");
    }
}

// ── without the shell ──

/// Runs the machine `config` describes until the guest switches it off: its
/// serial console on standard output and, for [`Console::Terminal`], this
/// terminal's keys sent to it. The terminal leaves the run as QEMU's
/// `-nographic` console does, on Ctrl-A then X; Ctrl-A twice sends one.
pub(crate) fn run_on_console(config: ArmVirtConfig, console: Console) -> Result<RunSummary, RunError> {
    let files = BootFiles::read(&config)?;
    let mut machine = build(&config, &files)?;
    let mut terminal = match console {
        Console::Headless => None,
        Console::Terminal => Some(RawTerminal::enter()),
    };
    let ran = drive_on_console(&mut machine, &config, terminal.as_mut());
    drop(terminal);
    finish(&config, &files, &mut machine, ran)
}

/// [`run_on_console`]'s loop, until the guest switches the board off, the
/// terminal leaves, or the board stops where no run asked it to.
fn drive_on_console(
    machine: &mut Virt,
    config: &ArmVirtConfig,
    mut terminal: Option<&mut RawTerminal>,
) -> Result<(), RunError> {
    let slice = slice_ticks(config.ips);
    let mut pacer = Pacer::new(machine, config.ips);
    loop {
        let outcome = machine.step(RunBudget::Ticks(slice));
        report_disk_error(machine, config);
        let output = take_serial_output(machine);
        if !output.is_empty() {
            mirror_to_stdout(&output);
        }
        if let Some(terminal) = terminal.as_deref_mut() {
            match terminal.pump(machine) {
                Pumped::RunOn => {}
                Pumped::Quit => return Ok(()),
            }
        }
        match settle(outcome)? {
            Settled::RunOn => pacer.keep_pace(machine),
            Settled::Over => return Ok(()),
        }
    }
}

/// A run's end, however it came: the variable store is written back and
/// the disk made durable whether or not the run failed — QEMU's flash keeps
/// what the firmware programmed, and its drives are flushed as they close,
/// whatever ends the VM. Of the failures, the run's own is the one
/// reported, then the variable store's, then the disk's; the others are
/// logged.
fn finish(
    config: &ArmVirtConfig,
    files: &BootFiles,
    machine: &mut Virt,
    ran: Result<(), RunError>,
) -> Result<RunSummary, RunError> {
    let persisted = files.persist(machine);
    let closed = close_disk(machine, config);
    let mut failures = [ran.err(), persisted.err(), closed.err()].into_iter().flatten();
    match failures.next() {
        None => Ok(RunSummary {
            instructions_executed: Some(machine.instructions()),
        }),
        Some(reported) => {
            for logged in failures {
                tracing::warn!("{logged}");
            }
            Err(reported)
        }
    }
}

/// What the terminal asked of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pumped {
    RunOn,
    Quit,
}

/// The most bytes one key sends: an xterm function key's escape sequence,
/// or a character's UTF-8 with an Alt prefix.
const LONGEST_KEY_BYTES: usize = 8;

/// The process's terminal in raw mode for as long as this lives, so every
/// key reaches the guest rather than the line discipline.
struct RawTerminal {
    raw: bool,
    /// Ctrl-A was pressed and the next key is the escape's.
    escaped: bool,
}

impl RawTerminal {
    fn enter() -> Self {
        let raw = match crossterm::terminal::enable_raw_mode() {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!("the terminal could not be put in raw mode ({error}); keys reach the guest a line at a time");
                false
            }
        };
        Self { raw, escaped: false }
    }

    /// Sends the keys pressed since the last call to `machine`'s serial
    /// console. Keys wait in the terminal while the console's input is too
    /// full for the longest one, so none is cut.
    fn pump(&mut self, machine: &mut Virt) -> Pumped {
        use crossterm::event::{self, Event, KeyEventKind};
        loop {
            let room = machine.serial(0).map_or(0, |port| port.input_room());
            if room < LONGEST_KEY_BYTES {
                return Pumped::RunOn;
            }
            match event::poll(Duration::ZERO) {
                Ok(true) => {}
                Ok(false) => return Pumped::RunOn,
                Err(error) => {
                    tracing::warn!("the terminal could not be read: {error}");
                    return Pumped::RunOn;
                }
            }
            let key = match event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => key,
                Ok(_) => continue,
                Err(error) => {
                    tracing::warn!("the terminal could not be read: {error}");
                    return Pumped::RunOn;
                }
            };
            match self.key(machine, key) {
                Pumped::RunOn => {}
                Pumped::Quit => return Pumped::Quit,
            }
        }
    }

    fn key(&mut self, machine: &mut Virt, key: crossterm::event::KeyEvent) -> Pumped {
        use crossterm::event::{KeyCode, KeyModifiers};
        const CTRL_A: u8 = 0x01;
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let control_byte = match key.code {
            KeyCode::Char(c) if control && (c.is_ascii_alphabetic() || "@[\\]^_".contains(c)) => {
                Some((c.to_ascii_uppercase() as u8) & 0x1F)
            }
            _ => None,
        };
        if self.escaped {
            self.escaped = false;
            return match (key.code, control_byte) {
                (_, Some(CTRL_A)) => {
                    send_bytes(machine, &[CTRL_A]);
                    Pumped::RunOn
                }
                (KeyCode::Char('x' | 'X'), None) => Pumped::Quit,
                // QEMU's other escapes drive its monitor, which this has none of.
                _ => Pumped::RunOn,
            };
        }
        if control_byte == Some(CTRL_A) {
            self.escaped = true;
            return Pumped::RunOn;
        }
        let mut bytes = Vec::with_capacity(LONGEST_KEY_BYTES);
        if alt {
            bytes.push(0x1B);
        }
        if let Some(byte) = control_byte {
            bytes.push(byte);
            send_bytes(machine, &bytes);
            return Pumped::RunOn;
        }
        let named = match key.code {
            KeyCode::Char(c) => {
                let mut utf8 = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
                send_bytes(machine, &bytes);
                return Pumped::RunOn;
            }
            KeyCode::Enter => rusty_box_aarch64::Key::Enter,
            KeyCode::Backspace => rusty_box_aarch64::Key::Backspace,
            KeyCode::Tab => rusty_box_aarch64::Key::Tab,
            KeyCode::Esc => rusty_box_aarch64::Key::Esc,
            KeyCode::Up => rusty_box_aarch64::Key::Up,
            KeyCode::Down => rusty_box_aarch64::Key::Down,
            KeyCode::Left => rusty_box_aarch64::Key::Left,
            KeyCode::Right => rusty_box_aarch64::Key::Right,
            KeyCode::Home => rusty_box_aarch64::Key::Home,
            KeyCode::End => rusty_box_aarch64::Key::End,
            KeyCode::PageUp => rusty_box_aarch64::Key::PageUp,
            KeyCode::PageDown => rusty_box_aarch64::Key::PageDown,
            KeyCode::Insert => rusty_box_aarch64::Key::Insert,
            KeyCode::Delete => rusty_box_aarch64::Key::Delete,
            KeyCode::F(number) => match function_key(number) {
                Some(key) => key,
                None => return Pumped::RunOn,
            },
            // Keys a terminal reports that send nothing on an xterm.
            _ => return Pumped::RunOn,
        };
        if !bytes.is_empty() {
            send_bytes(machine, &bytes);
        }
        if !machine.terminal_keyboard().tap(named) {
            tracing::warn!("the serial console refused {named:?}");
        }
        Pumped::RunOn
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        if self.raw {
            // The teardown path has no one to hand an error to, and a
            // terminal left raw is worth saying out loud.
            if let Err(error) = crossterm::terminal::disable_raw_mode() {
                tracing::warn!("terminal left in raw mode: {error}");
            }
        }
    }
}

/// F1 to F12.
fn function_key(number: u8) -> Option<rusty_box_aarch64::Key> {
    use rusty_box_aarch64::Key;
    const KEYS: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    KEYS.get(usize::from(number).checked_sub(1)?).copied()
}

/// Sends `bytes` to the serial console, which [`RawTerminal::pump`] made
/// room for.
fn send_bytes(machine: &mut Virt, bytes: &[u8]) {
    let taken = machine.keyboard().bytes(bytes);
    if taken < bytes.len() {
        tracing::warn!("the serial console took {taken} of a key's {} bytes", bytes.len());
    }
}

// ── in the shell ──

/// One run of the machine `config` describes, on the VM window `display`,
/// under the shell's run `control`: paused, stepped, dumped and ended from
/// there. Returns once the guest switches itself off or the shell ends the
/// run.
#[cfg(feature = "gui-egui")]
pub(crate) fn run_in_shell(
    config: &ArmVirtConfig,
    display: &Arc<Mutex<SharedDisplay>>,
    control: &Arc<RunControl>,
) -> Result<RunSummary, RunError> {
    let files = BootFiles::read(config)?;
    let mut machine = build(config, &files)?;
    let mut window = ShellWindow::open(display, &machine);
    let ran = drive_in_shell(&mut machine, config, &mut window, control);
    finish(config, &files, &mut machine, ran)
}

/// [`run_in_shell`]'s loop, until the guest switches the board off, the
/// shell ends the run, or the board stops where no run asked it to.
#[cfg(feature = "gui-egui")]
fn drive_in_shell(
    machine: &mut Virt,
    config: &ArmVirtConfig,
    window: &mut ShellWindow<'_>,
    control: &RunControl,
) -> Result<(), RunError> {
    let slice = slice_ticks(config.ips);
    let mut pacer = Pacer::new(machine, config.ips);
    let stop_flag = Arc::clone(control.stop_flag());
    loop {
        if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            match control.after_run(machine.power().state().into()) {
                AfterRun::End => return Ok(()),
                AfterRun::Serve => match serve(machine, control, window)? {
                    Next::Resume => {
                        pacer.restart(machine);
                        window.restart_rate(machine);
                        continue;
                    }
                    Next::End => return Ok(()),
                },
            }
        }
        let outcome = machine.step(RunBudget::Ticks(slice));
        report_disk_error(machine, config);
        window.pump_input(machine);
        window.present(machine, Presenting::WhenDue);
        match settle(outcome)? {
            Settled::RunOn => pacer.keep_pace(machine),
            Settled::Over => return Ok(()),
        }
    }
}

/// Serves the shell until it lets the machine run on or ends the run.
#[cfg(feature = "gui-egui")]
fn serve(machine: &mut Virt, control: &RunControl, window: &mut ShellWindow<'_>) -> Result<Next, RunError> {
    loop {
        match control.next_between_runs() {
            BetweenRuns::Perform(task) => match perform(machine, control, window, task)? {
                MachineAfter::Live => {}
                MachineAfter::Over => return Ok(Next::End),
            },
            BetweenRuns::Resume => return Ok(Next::Resume),
            BetweenRuns::End => return Ok(Next::End),
        }
    }
}

/// Performs `task` on `machine`, which is between runs. The Debugger page
/// reads a PC; this machine has none of its views, so a pause publishes no
/// snapshot and a search is answered as one with no debugger.
#[cfg(feature = "gui-egui")]
fn perform(
    machine: &mut Virt,
    control: &RunControl,
    window: &mut ShellWindow<'_>,
    task: Task,
) -> Result<MachineAfter, RunError> {
    match task {
        Task::Refresh => {
            // A paused machine retires nothing, and the status bar says so.
            window.show_ips(0);
            window.present(machine, Presenting::Now);
            Ok(MachineAfter::Live)
        }
        Task::Step => {
            let outcome = machine.step(RunBudget::Instructions(1));
            window.present(machine, Presenting::Now);
            Ok(match settle(outcome)? {
                Settled::RunOn => MachineAfter::Live,
                Settled::Over => MachineAfter::Over,
            })
        }
        Task::DumpRam(path) => {
            control.record_dump(dump_ram(machine, &path, || control.ending()));
            Ok(MachineAfter::Live)
        }
        Task::Find(_) => {
            control.record_find(FindOutcome::NoDebugger);
            Ok(MachineAfter::Live)
        }
    }
}

/// Whether the frame and the instruction rate are presented now or when
/// their interval is up.
#[cfg(feature = "gui-egui")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presenting {
    WhenDue,
    Now,
}

/// The VM's window as a run of this machine drives it: the host input the
/// shell queued, waiting until the guest takes it; and the frame, the serial
/// console and the instruction rate going the other way.
#[cfg(feature = "gui-egui")]
struct ShellWindow<'d> {
    display: &'d Arc<Mutex<SharedDisplay>>,
    keys: VecDeque<(Key, KeyAction)>,
    pointer: VecDeque<AbsolutePointerEvent>,
    serial: VecDeque<u8>,
    /// The guest's serial output as the log shows it.
    serial_text: Utf8Carry,
    /// When the guest last took input, or had none waiting for it.
    input_taken: Instant,
    last_frame: Instant,
    last_rate: Instant,
    rate_from: u64,
}

#[cfg(feature = "gui-egui")]
impl<'d> ShellWindow<'d> {
    /// Takes over `display` for a run of `machine`: the picture starts
    /// empty until the guest draws one.
    fn open(display: &'d Arc<Mutex<SharedDisplay>>, machine: &Virt) -> Self {
        if let Ok(mut shown) = display.lock() {
            shown.ips = 0;
            shown.framebuffer.fill(0);
            shown.fb_dirty = true;
        }
        Self {
            display,
            keys: VecDeque::new(),
            pointer: VecDeque::new(),
            serial: VecDeque::new(),
            serial_text: Utf8Carry::default(),
            input_taken: Instant::now(),
            last_frame: Instant::now(),
            last_rate: Instant::now(),
            rate_from: machine.instructions(),
        }
    }

    /// Takes the input the shell queued and sends the guest as much as it
    /// will take, in order; the rest waits for the next slice. Input the
    /// guest has taken none of for [`STALE_INPUT`] is dropped.
    fn pump_input(&mut self, machine: &mut Virt) {
        if let Ok(mut shown) = self.display.lock() {
            for (key, pressed) in shown.pending_keys.drain(..) {
                let action = if pressed { KeyAction::Press } else { KeyAction::Release };
                push_bounded(&mut self.keys, (key, action));
            }
            for event in shown.pending_pointer.drain(..) {
                push_bounded(&mut self.pointer, event);
            }
            for byte in shown.drain_serial_input() {
                push_bounded(&mut self.serial, byte);
            }
            // The board has no relative mouse; the window queues its moves
            // only when it shows no tablet, and nothing here can take them.
            shown.pending_mouse.clear();
        }
        let waiting = self.keys.len() + self.pointer.len() + self.serial.len();
        while let Some(&(key, action)) = self.keys.front() {
            if !machine.keyboard().key(key, action) {
                break;
            }
            self.keys.pop_front();
        }
        match machine.pointer() {
            Some(mut tablet) => {
                while let Some(&event) = self.pointer.front() {
                    let taken = match event {
                        AbsolutePointerEvent::MoveTo(pos) => tablet.move_to(pos),
                        AbsolutePointerEvent::Button(button, action) => tablet.button(button, action),
                        AbsolutePointerEvent::Wheel(steps) => tablet.wheel(steps),
                    };
                    if !taken {
                        break;
                    }
                    self.pointer.pop_front();
                }
            }
            None => self.pointer.clear(),
        }
        if !self.serial.is_empty() {
            if let Some(mut port) = machine.serial(0) {
                let waiting = self.serial.make_contiguous();
                let taken = port.send(waiting);
                self.serial.drain(..taken);
            }
        }
        let left = self.keys.len() + self.pointer.len() + self.serial.len();
        if left == 0 || left < waiting {
            self.input_taken = Instant::now();
        } else if self.input_taken.elapsed() >= STALE_INPUT {
            tracing::debug!("the guest took no input for {STALE_INPUT:?}; {left} waiting events were dropped");
            self.keys.clear();
            self.pointer.clear();
            self.serial.clear();
            self.input_taken = Instant::now();
        }
    }

    /// Hands the window the guest's serial output, and the frame and the
    /// instruction rate when `when` says so.
    fn present(&mut self, machine: &mut Virt, when: Presenting) {
        let output = take_serial_output(machine);
        if !output.is_empty() {
            mirror_to_stdout(&output);
            let text = self.serial_text.decode(&output);
            if !text.is_empty() {
                if let Ok(mut shown) = self.display.lock() {
                    shown.append_serial_log(&text);
                }
            }
        }
        if when == Presenting::Now || self.last_frame.elapsed() >= FRAME_INTERVAL {
            self.last_frame = Instant::now();
            if let Some(mut screen) = machine.display() {
                if let Ok(mut shown) = self.display.lock() {
                    match screen.refresh(&mut *shown) {
                        Refreshed::Frame => shown.fb_dirty = true,
                        Refreshed::Unchanged => {}
                    }
                }
            }
        }
        if when == Presenting::WhenDue && self.last_rate.elapsed() >= IPS_INTERVAL {
            let instructions = machine.instructions();
            let per_second = instructions.saturating_sub(self.rate_from) as f64 / self.last_rate.elapsed().as_secs_f64();
            self.show_ips(per_second as u32);
            self.rate_from = instructions;
            self.last_rate = Instant::now();
        }
    }

    /// Measures the instruction rate from here: the time a pause stood still
    /// is not the guest's.
    fn restart_rate(&mut self, machine: &Virt) {
        self.rate_from = machine.instructions();
        self.last_rate = Instant::now();
    }

    fn show_ips(&mut self, ips: u32) {
        if let Ok(mut shown) = self.display.lock() {
            shown.ips = ips;
        }
    }
}

/// A byte stream decoded as UTF-8 a piece at a time: a character whose bytes
/// a piece ends inside waits for the rest, rather than showing as two
/// replacement characters; a byte that starts no character shows as one.
#[cfg(feature = "gui-egui")]
#[derive(Debug, Default)]
struct Utf8Carry {
    /// The start of a character the last piece ended inside.
    partial: Vec<u8>,
}

#[cfg(feature = "gui-egui")]
impl Utf8Carry {
    /// The text `bytes` completes, after what the last call held back.
    fn decode(&mut self, bytes: &[u8]) -> String {
        self.partial.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = self.partial.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    text.push_str(&String::from_utf8_lossy(valid));
                    match error.error_len() {
                        // The piece ends inside a character.
                        None => {
                            rest = after;
                            break;
                        }
                        Some(len) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            rest = after.get(len..).unwrap_or_default();
                        }
                    }
                }
            }
        }
        self.partial = rest.to_vec();
        text
    }
}

/// Queues `item`, dropping the oldest past [`INPUT_BACKLOG`].
#[cfg(feature = "gui-egui")]
fn push_bounded<T>(queue: &mut VecDeque<T>, item: T) {
    if queue.len() == INPUT_BACKLOG {
        queue.pop_front();
        tracing::debug!("{INPUT_BACKLOG} input events are waiting; the oldest was dropped");
    }
    queue.push_back(item);
}

/// Guest images the tests boot.
#[cfg(test)]
pub(crate) mod test_images {
    /// An arm64 Linux `Image` whose kernel does one thing: switch the
    /// machine off through PSCI `SYSTEM_OFF` on `virt`'s HVC conduit. Its
    /// header is the boot protocol's (`image_size` zero: loaded at the legacy
    /// offset), and its first word branches past the header to the code.
    pub(crate) fn power_off_kernel() -> Vec<u8> {
        const HEADER_LEN: usize = 64;
        let mut image = vec![0u8; HEADER_LEN];
        image[0..4].copy_from_slice(&0x1400_0010u32.to_le_bytes()); // b #64
        image[56..60].copy_from_slice(b"ARM\x64");
        for word in [
            0xD280_0100u32, // movz x0, #0x0008
            0xF2B0_8000,    // movk x0, #0x8400, lsl #16: PSCI SYSTEM_OFF
            0xD400_0002,    // hvc #0
            0x1400_0000,    // b .
        ] {
            image.extend_from_slice(&word.to_le_bytes());
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VirtioDisplaySize;

    /// A file of `bytes` no other test of this process uses, removed when
    /// dropped.
    struct ScratchFile(PathBuf);

    impl ScratchFile {
        fn new(name: &str, bytes: &[u8]) -> Self {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusty-box-gui-virt-{}-{sequence}-{name}",
                std::process::id()
            ));
            std::fs::write(&path, bytes).expect("write the scratch file");
            Self(path)
        }
    }

    impl Drop for ScratchFile {
        fn drop(&mut self) {
            match std::fs::remove_file(&self.0) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => panic!("failed to remove {}: {error}", self.0.display()),
            }
        }
    }

    fn kernel_board(kernel: &ScratchFile) -> ArmVirtConfig {
        ArmVirtConfig {
            boot: ArmBoot::Linux {
                kernel: kernel.0.clone(),
                initrd: None,
                command_line: String::new(),
                device_tree: None,
            },
            ..ArmVirtConfig::blank()
        }
    }

    #[test]
    fn a_kernel_that_switches_the_board_off_ends_a_console_run() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());

        let summary = run_on_console(kernel_board(&kernel), Console::Headless).expect("the run");

        assert_eq!(summary.instructions_executed, Some(3), "movz, movk, hvc");
    }

    #[test]
    fn a_board_with_nothing_to_boot_is_refused() {
        assert!(matches!(
            run_on_console(ArmVirtConfig::blank(), Console::Headless),
            Err(RunError::MissingVirtBoot)
        ));
    }

    #[test]
    fn a_file_that_is_not_an_arm64_image_is_refused_with_its_path() {
        let kernel = ScratchFile::new("bzImage", &[0x4D, 0x5A, 0, 0, 0, 0, 0, 0]);

        match run_on_console(kernel_board(&kernel), Console::Headless) {
            Err(RunError::VirtKernel { path, .. }) => assert_eq!(path, kernel.0),
            other => panic!("not refused as a kernel: {other:?}"),
        }
    }

    /// Firmware that did not program its variable store leaves the store's
    /// file as it was: not rewritten, and not grown to the bank's size.
    #[test]
    fn an_unprogrammed_variable_store_is_not_written_back() {
        // Firmware boots from flash at address 0; the image is the power-off
        // kernel's code, which needs no header there.
        let code = test_images::power_off_kernel().split_off(64);
        let firmware = ScratchFile::new("QEMU_EFI.fd", &code);
        let vars = ScratchFile::new("vars.fd", &[0x5A; 4096]);
        let board = ArmVirtConfig {
            boot: ArmBoot::Firmware {
                code: firmware.0.clone(),
                vars: Some(vars.0.clone()),
            },
            ..ArmVirtConfig::blank()
        };

        run_on_console(board, Console::Headless).expect("the run");

        assert_eq!(std::fs::read(&vars.0).expect("the store"), vec![0x5A; 4096]);
    }

    #[test]
    fn only_a_switched_off_board_ends_a_run_and_a_stop_nobody_asked_for_is_refused() {
        let outcome = |stop| BatchOutcome {
            progress: rusty_box_aarch64::Progress::Instructions(0),
            stop,
        };
        for stop in [StopReason::BudgetExhausted, StopReason::Halted] {
            assert_eq!(settle(outcome(stop)).expect("runs on"), Settled::RunOn, "{stop:?}");
        }
        for stop in [StopReason::PoweredOff, StopReason::ProcessorOff] {
            assert_eq!(settle(outcome(stop)).expect("ends"), Settled::Over, "{stop:?}");
        }
        for stop in [
            StopReason::StopRequested,
            StopReason::ExitReached,
            StopReason::ResetRequested,
            StopReason::BusFault { pc: 0, addr: 0 },
        ] {
            assert!(
                matches!(settle(outcome(stop)), Err(RunError::VirtStopped { stop: refused }) if refused == stop),
                "{stop:?}"
            );
        }
    }

    #[test]
    fn a_slice_is_ten_milliseconds_of_guest_time_and_never_none() {
        assert_eq!(slice_ticks(50_000_000), 500_000);
        assert_eq!(slice_ticks(1), 1);
    }

    /// The board is built with the processor the VM names: its `MIDR_EL1`,
    /// and the counter frequency it reads at reset.
    #[test]
    fn the_board_runs_the_processor_the_vm_names() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        for model in rusty_box_aarch64::CpuModel::ALL {
            let config = ArmVirtConfig {
                cpu: model,
                ..kernel_board(&kernel)
            };
            let files = BootFiles::read(&config).expect("the kernel");
            let machine = build(&config, &files).expect("the machine");
            let midr = machine.sysreg_read(rusty_box_aarch64::SysReg::MIDR_EL1);
            let cntfrq = machine.sysreg_read(rusty_box_aarch64::SysReg::CNTFRQ_EL0);
            let want = match model {
                rusty_box_aarch64::CpuModel::CortexA57 => (0x411F_D070, 62_500_000),
                rusty_box_aarch64::CpuModel::Armv9p2 => (0x000F_0520, 1_000_000_000),
            };
            assert_eq!((midr, cntfrq), (Ok(want.0), Ok(want.1)), "{model:?}");
        }
    }

    /// The board's virtio-blk is over the file the VM names: the guest sees
    /// its length in whole sectors, the last one padded, and may write it
    /// unless the VM makes it read-only.
    #[test]
    fn the_board_runs_on_the_disk_the_vm_names() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let image = ScratchFile::new("disk.img", &[0xA5; 3 * 512 + 1]);
        for (access, device) in [
            (DiskAccess::ReadWrite, rusty_box_aarch64::Access::ReadWrite),
            (DiskAccess::ReadOnly, rusty_box_aarch64::Access::ReadOnly),
        ] {
            let config = ArmVirtConfig {
                disk: Some(VirtDisk {
                    path: image.0.clone(),
                    access,
                }),
                ..kernel_board(&kernel)
            };
            let files = BootFiles::read(&config).expect("the kernel");
            let machine = build(&config, &files).expect("the machine");
            let disk = machine.disk().expect("a disk");
            assert_eq!((disk.sectors(), disk.access()), (4, device), "{access:?}");
        }
    }

    /// A file the host lets this process only read can be a read-only disk:
    /// the image is opened for reading alone. (A process that permissions do
    /// not bind, root's, could open it for writing too.)
    #[test]
    fn a_file_this_process_may_only_read_is_a_read_only_disk() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let image = ScratchFile::new("readable.img", &[0; 512]);
        let writable = std::fs::metadata(&image.0).expect("the image").permissions();
        let mut readable = writable.clone();
        readable.set_readonly(true);
        std::fs::set_permissions(&image.0, readable).expect("make the image read-only");
        let config = ArmVirtConfig {
            disk: Some(VirtDisk {
                path: image.0.clone(),
                access: DiskAccess::ReadOnly,
            }),
            ..kernel_board(&kernel)
        };

        let files = BootFiles::read(&config).expect("the kernel");
        let built = build(&config, &files).map(|machine| machine.disk().map(|disk| disk.access()));
        std::fs::set_permissions(&image.0, writable).expect("give the image back its permissions");
        assert_eq!(built.expect("the machine"), Some(rusty_box_aarch64::Access::ReadOnly));
    }

    #[test]
    fn a_disk_that_cannot_be_opened_is_refused_with_its_path() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let missing = std::env::temp_dir().join(format!("rusty-box-gui-virt-{}-absent.img", std::process::id()));
        let config = ArmVirtConfig {
            disk: Some(VirtDisk {
                path: missing.clone(),
                access: DiskAccess::ReadOnly,
            }),
            ..kernel_board(&kernel)
        };

        match run_on_console(config, Console::Headless) {
            Err(RunError::MediaAttach { kind: "disk", path, source }) => {
                assert_eq!((path, source.kind()), (missing, io::ErrorKind::NotFound));
            }
            other => panic!("not refused as a disk: {other:?}"),
        }
    }

    /// A built machine whose serial console takes the terminal's keys.
    fn console_machine() -> (Virt, ScratchFile) {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let config = kernel_board(&kernel);
        let files = BootFiles::read(&config).expect("the kernel");
        (build(&config, &files).expect("the machine"), kernel)
    }

    fn key(code: crossterm::event::KeyCode, modifiers: crossterm::event::KeyModifiers) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, modifiers)
    }

    fn room(machine: &mut Virt) -> usize {
        machine.serial(0).expect("the UART").input_room()
    }

    /// Keys reach the serial console as an xterm sends them: an arrow as its
    /// three-byte escape sequence, a control chord as its control byte.
    #[test]
    fn a_terminal_key_reaches_the_console_as_an_xterm_sends_it() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let (mut machine, _kernel) = console_machine();
        let mut terminal = RawTerminal { raw: false, escaped: false };
        let empty = room(&mut machine);

        assert_eq!(terminal.key(&mut machine, key(KeyCode::Up, KeyModifiers::NONE)), Pumped::RunOn);
        assert_eq!(room(&mut machine), empty - 3, "ESC [ A");
        assert_eq!(terminal.key(&mut machine, key(KeyCode::Char('c'), KeyModifiers::CONTROL)), Pumped::RunOn);
        assert_eq!(room(&mut machine), empty - 4, "Ctrl-C is one byte");
        assert_eq!(terminal.key(&mut machine, key(KeyCode::Char('é'), KeyModifiers::NONE)), Pumped::RunOn);
        assert_eq!(room(&mut machine), empty - 6, "é is two bytes of UTF-8");
    }

    /// Ctrl-A starts QEMU's console escape: X then leaves the run, a second
    /// Ctrl-A sends one, and nothing of the escape reaches the guest.
    #[test]
    fn ctrl_a_x_leaves_the_run_and_ctrl_a_twice_sends_one() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let (mut machine, _kernel) = console_machine();
        let mut terminal = RawTerminal { raw: false, escaped: false };
        let empty = room(&mut machine);
        let ctrl_a = key(KeyCode::Char('a'), KeyModifiers::CONTROL);

        assert_eq!(terminal.key(&mut machine, ctrl_a), Pumped::RunOn);
        assert_eq!(terminal.key(&mut machine, ctrl_a), Pumped::RunOn);
        assert_eq!(room(&mut machine), empty - 1, "one Ctrl-A");

        assert_eq!(terminal.key(&mut machine, ctrl_a), Pumped::RunOn);
        assert_eq!(terminal.key(&mut machine, key(KeyCode::Char('x'), KeyModifiers::NONE)), Pumped::Quit);
        assert_eq!(room(&mut machine), empty - 1, "the escape sent nothing");
    }

    #[test]
    fn function_keys_are_f1_to_f12_and_nothing_past() {
        assert_eq!(function_key(1), Some(rusty_box_aarch64::Key::F1));
        assert_eq!(function_key(12), Some(rusty_box_aarch64::Key::F12));
        assert_eq!(function_key(0), None);
        assert_eq!(function_key(13), None);
    }

    /// The shell's run drives the window until the guest switches the board
    /// off.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn a_shell_run_drives_the_window_until_the_guest_switches_off() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let config = ArmVirtConfig {
            display: Some(VirtioDisplaySize {
                width: 640,
                height: 480,
            }),
            tablet: true,
            ..kernel_board(&kernel)
        };
        let display = Arc::new(Mutex::new(SharedDisplay::new()));
        let control = Arc::new(RunControl::new(Arc::new(std::sync::atomic::AtomicBool::new(false))));
        control.begin_run(crate::debugger::control::RunStart::PowerOn);

        let summary = run_in_shell(&config, &display, &control).expect("the run");

        assert_eq!(summary.instructions_executed, Some(3));
    }

    /// A character whose bytes two slices split shows once, whole; a byte
    /// that starts no character shows as one replacement character.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn serial_text_split_inside_a_character_shows_it_whole() {
        let mut text = Utf8Carry::default();
        let box_drawing = "─".as_bytes();

        assert_eq!(text.decode(&[b'a', box_drawing[0]]), "a");
        assert_eq!(text.decode(&box_drawing[1..]), "─");
        assert_eq!(text.decode(&[0xFF, b'b']), "\u{FFFD}b");
        assert_eq!(text.decode(b""), "");
    }

    /// A window and a machine built from `config`, booting the power-off
    /// kernel, which never loads an input driver.
    #[cfg(feature = "gui-egui")]
    fn window_over<'d>(config: &ArmVirtConfig, display: &'d Arc<Mutex<SharedDisplay>>) -> (Virt, ShellWindow<'d>) {
        let files = BootFiles::read(config).expect("the kernel");
        let machine = build(config, &files).expect("the machine");
        let window = ShellWindow::open(display, &machine);
        (machine, window)
    }

    /// `count` taps of A, as one frame of the window queues a typed burst.
    #[cfg(feature = "gui-egui")]
    fn queue_taps(display: &Mutex<SharedDisplay>, count: usize) {
        let mut shown = display.lock().unwrap();
        for _ in 0..count {
            shown.pending_keys.push((Key::A, true));
            shown.pending_keys.push((Key::A, false));
        }
    }

    /// A burst the guest is taking reaches it whole, however long: here the
    /// serial console's terminal, which takes every key at once.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn a_burst_of_keys_the_guest_takes_reaches_it_whole() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let config = kernel_board(&kernel);
        let display = Arc::new(Mutex::new(SharedDisplay::new()));
        let (mut machine, mut window) = window_over(&config, &display);
        let empty = room(&mut machine);

        queue_taps(&display, 200);
        window.pump_input(&mut machine);

        assert!(window.keys.is_empty());
        assert_eq!(room(&mut machine), empty - 200, "every tap typed its letter");
    }

    /// A burst a guest is not taking yet (its virtio keyboard has no driver)
    /// waits whole, and is dropped only once the guest has taken none of it
    /// for `STALE_INPUT`.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn a_burst_of_keys_waits_whole_until_it_goes_stale() {
        let kernel = ScratchFile::new("Image", &test_images::power_off_kernel());
        let config = ArmVirtConfig {
            keyboard: true,
            ..kernel_board(&kernel)
        };
        let display = Arc::new(Mutex::new(SharedDisplay::new()));
        let (mut machine, mut window) = window_over(&config, &display);

        queue_taps(&display, 200);
        window.pump_input(&mut machine);
        assert_eq!(window.keys.len(), 400, "none taken, none dropped");

        window.input_taken = Instant::now()
            .checked_sub(STALE_INPUT)
            .expect("the host has been up longer than the staleness bound");
        window.pump_input(&mut machine);
        assert!(window.keys.is_empty(), "stale input is dropped");
    }

    /// Input a guest has not taken waits, oldest first, and past the backlog
    /// the oldest goes.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn the_input_backlog_keeps_the_newest_events() {
        let mut queue = VecDeque::new();
        for event in 0..INPUT_BACKLOG + 2 {
            push_bounded(&mut queue, event);
        }
        assert_eq!(queue.len(), INPUT_BACKLOG);
        assert_eq!(queue.front(), Some(&2));
        assert_eq!(queue.back(), Some(&(INPUT_BACKLOG + 1)));
    }
}
