//! Boot a CD image headless and report what its screen and serial console do.
//!
//! A diagnosis harness for a guest whose screen goes wrong: it logs, against
//! emulated time, every write to the display registers that decides whether
//! anything is shown — the sequencer's screen-off bit (SR01 bit 5), the
//! attribute controller's palette-address source (bit 5 of its address byte),
//! the CRTC start address and mode control, the miscellaneous output register,
//! the graphics controller's memory map and the Bochs DISPI registers — and
//! prints the screen each time its mode or content changes. With `--login`
//! it logs in on the serial console and runs `--cmd` commands there, so the
//! guest can be asked what it thinks it did.
//!
//! ```text
//! cargo run --release --example boot_probe --features std -- slitaz.iso \
//!     --seconds 300 --login root:root --cmd 'dmesg | tail'
//! ```

use std::time::Instant;

use rusty_box::Result;
use rusty_box::cpu::{HookMask, Instrumentation, IoHookEvent};
use rusty_box::emulator::{
    AtaSlot, BootDevice, BootOrder, EmulatorConfig, Ips, MachineBuilder, MemorySize, RunBudget,
};
use rusty_box::gui::NoGui;

const BIOS: &str = "cpp_orig/bochs/bochs/bios/BIOS-bochs-latest";
const VGA_BIOS: &str = "cpp_orig/bochs/bochs/bios/VGABIOS-lgpl/VGABIOS-lgpl-latest.bin";

/// The display registers a guest writes, as a guest writes them: an index
/// port then a data port, the attribute controller's one port alternating
/// between the two.
#[derive(Default)]
struct DisplayWatch {
    seq_index: u8,
    gr_index: u8,
    crtc_index: u8,
    dispi_index: u16,
    /// Whether the next byte to 0x3C0 is data (an address byte was the last).
    attr_expects_data: bool,
    sr01: Option<u8>,
    attr_pas: Option<bool>,
    crtc_mode_control: Option<u8>,
    misc_output: Option<u8>,
    gr06: Option<u8>,
    /// The last value written to each DISPI register (Bochs vga.cc
    /// `VBE_DISPI_INDEX_ID` .. `VBE_DISPI_INDEX_DDC`).
    dispi: [Option<u16>; 16],
    start_address: [u8; 2],
    start_address_writes: u64,
    dac_writes: u64,
    dac_zero_writes: u64,
    /// What changed since the host last asked, in order.
    events: Vec<String>,
}

impl DisplayWatch {
    fn note<V: PartialEq + Copy>(
        events: &mut Vec<String>,
        last: &mut Option<V>,
        value: V,
        say: impl FnOnce(V) -> String,
    ) {
        if *last != Some(value) {
            *last = Some(value);
            events.push(say(value));
        }
    }

    fn write_byte(&mut self, port: u16, value: u8) {
        match port {
            0x3C4 => self.seq_index = value,
            0x3C5 if self.seq_index == 1 => {
                Self::note(&mut self.events, &mut self.sr01, value, |v| {
                    format!(
                        "SR01 = {v:#04x}: screen {}",
                        if v & 0x20 != 0 { "OFF" } else { "on" }
                    )
                });
            }
            0x3C0 if !self.attr_expects_data => {
                self.attr_expects_data = true;
                Self::note(
                    &mut self.events,
                    &mut self.attr_pas,
                    value & 0x20 != 0,
                    |pas| {
                        format!(
                            "attribute address {value:#04x}: palette source {}",
                            if pas {
                                "video (picture shown)"
                            } else {
                                "CPU (picture BLANK)"
                            }
                        )
                    },
                );
            }
            0x3C0 => self.attr_expects_data = false,
            0x3C2 => Self::note(&mut self.events, &mut self.misc_output, value, |v| {
                format!("misc output = {v:#04x}")
            }),
            0x3C8 => {}
            0x3C9 => {
                self.dac_writes += 1;
                if value == 0 {
                    self.dac_zero_writes += 1;
                }
            }
            0x3CE => self.gr_index = value,
            0x3CF if self.gr_index == 6 => {
                Self::note(&mut self.events, &mut self.gr06, value, |v| {
                    format!(
                        "GR06 = {v:#04x}: {} memory map {}",
                        if v & 1 != 0 { "graphics" } else { "text" },
                        (v >> 2) & 3
                    )
                })
            }
            0x3B4 | 0x3D4 => self.crtc_index = value,
            0x3B5 | 0x3D5 => match self.crtc_index {
                0x0C => {
                    self.start_address[0] = value;
                    self.start_address_writes += 1;
                }
                0x0D => {
                    self.start_address[1] = value;
                    self.start_address_writes += 1;
                }
                0x17 => Self::note(&mut self.events, &mut self.crtc_mode_control, value, |v| {
                    format!(
                        "CRTC mode control = {v:#04x}: timing {}",
                        if v & 0x80 != 0 { "running" } else { "STOPPED" }
                    )
                }),
                _ => {}
            },
            _ => {}
        }
    }

    fn write_word(&mut self, port: u16, value: u16) {
        match port {
            0x1CE => self.dispi_index = value,
            0x1CF => {
                let index = self.dispi_index;
                if let Some(last) = self.dispi.get_mut(usize::from(index)) {
                    Self::note(&mut self.events, last, value, |v| {
                        format!("DISPI[{index:#x}] = {v:#06x}")
                    });
                }
            }
            _ => {
                let [low, high] = value.to_le_bytes();
                self.write_byte(port, low);
                self.write_byte(port.wrapping_add(1), high);
            }
        }
    }
}

/// The VGA's ports and the Bochs DISPI index and data ports.
fn is_display_port(port: u16) -> bool {
    matches!(port, 0x3B0..=0x3DF | 0x1CE | 0x1CF)
}

impl Instrumentation for DisplayWatch {
    fn active_hooks(&self) -> HookMask {
        HookMask::IO
    }

    fn inp2(&mut self, ev: &IoHookEvent) {
        // Reading Input Status 1 resets the attribute flip-flop to address.
        if matches!(ev.port.get(), 0x3BA | 0x3DA) {
            self.attr_expects_data = false;
        }
    }

    fn outp(&mut self, ev: &IoHookEvent) {
        let port = ev.port.get();
        match ev.size {
            1 => self.write_byte(port, ev.value as u8),
            2 => self.write_word(port, ev.value as u16),
            _ if is_display_port(port) => self.events.push(format!(
                "{}-byte write {:#x} to port {port:#x}",
                ev.size, ev.value
            )),
            _ => {}
        }
    }
}

/// Where the serial login has got to.
enum Login {
    Off,
    WaitingForLogin { user: String, password: String },
    WaitingForPassword { password: String },
    Commands,
}

struct Options {
    iso: String,
    memory_mib: usize,
    ips: u32,
    seconds: f64,
    /// Put the VGA on the PCI bus as `1234:1111` (Bochs `pci: slot1=pcivga`).
    pci_vga: bool,
    login: Option<(String, String)>,
    commands: Vec<String>,
}

fn parse_options() -> std::result::Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut options = Options {
        iso: String::new(),
        memory_mib: 512,
        ips: 300_000_000,
        seconds: 300.0,
        pci_vga: false,
        login: None,
        commands: Vec::new(),
    };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--memory-mib" => {
                options.memory_mib = value("--memory-mib")?.parse().map_err(|e| format!("{e}"))?
            }
            "--ips" => options.ips = value("--ips")?.parse().map_err(|e| format!("{e}"))?,
            "--seconds" => {
                options.seconds = value("--seconds")?.parse().map_err(|e| format!("{e}"))?
            }
            "--pci-vga" => options.pci_vga = true,
            "--login" => {
                let pair = value("--login")?;
                let (user, password) = pair
                    .split_once(':')
                    .ok_or("--login takes user:password".to_string())?;
                options.login = Some((user.to_string(), password.to_string()));
            }
            "--cmd" => options.commands.push(value("--cmd")?),
            iso if !iso.starts_with("--") && options.iso.is_empty() => options.iso = arg,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if options.iso.is_empty() {
        return Err(
            "usage: boot_probe <image.iso> [--memory-mib N] [--ips N] [--seconds S] \
                    [--pci-vga] [--login user:password] [--cmd 'command']..."
                .to_string(),
        );
    }
    Ok(options)
}

fn main() -> Result<()> {
    let options = match parse_options() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let bios = std::fs::read(BIOS)?;
    let vga_bios = std::fs::read(VGA_BIOS)?;
    let config = EmulatorConfig {
        memory: MemorySize::bytes(options.memory_mib * 1024 * 1024),
        ips: Ips::new(options.ips),
        pci_enabled: true,
        pci_vga: options.pci_vga,
        ..Default::default()
    };
    let mut machine = MachineBuilder::new(config)
        .tracer(DisplayWatch::default())
        .gui(NoGui::new())
        .bios(&bios)
        .vga_bios(&vga_bios)
        .boot_order(BootOrder::just(BootDevice::Cdrom))
        .cdrom_file(AtaSlot::SECONDARY_MASTER, &options.iso)
        .build()?;

    let chunk = u64::from(options.ips / 10);
    let started = Instant::now();
    let mut login = match options.login {
        Some((user, password)) => Login::WaitingForLogin { user, password },
        None => Login::Off,
    };
    let mut commands = options.commands.into_iter();
    let mut serial_tail = String::new();
    let mut serial_line = String::new();
    let mut to_send: Vec<u8> = Vec::new();
    let mut last_screen = String::new();

    loop {
        let outcome = machine.step(RunBudget::Instructions(chunk))?;
        let at = machine.ticks() as f64 / f64::from(options.ips);

        let watch = machine.instrumentation_mut();
        for event in watch.events.drain(..) {
            println!("[{at:8.2}s] display: {event}");
        }

        let resolution = machine.display().resolution();
        let text = machine.display().text().map(|screen| screen.to_text());
        let screen = match &text {
            Some(text) => format!(
                "text {}x{}, {} non-blank characters",
                resolution.width,
                resolution.height,
                text.chars().filter(|c| !c.is_whitespace()).count()
            ),
            None => format!("graphics {}x{}", resolution.width, resolution.height),
        };
        if screen != last_screen {
            println!("[{at:8.2}s] screen: {screen}");
            if let Some(text) = &text {
                for row in text.lines().filter(|row| !row.trim().is_empty()) {
                    println!("             | {row}");
                }
            }
            last_screen = screen;
        }

        let output: Vec<u8> = machine
            .serial(0)
            .map(|mut port| port.take_output().collect())
            .unwrap_or_default();
        for character in String::from_utf8_lossy(&output).chars() {
            serial_tail.push(character);
            if character == '\n' {
                println!("[{at:8.2}s] serial: {}", serial_line.trim_end());
                serial_line.clear();
            } else if character != '\r' {
                serial_line.push(character);
            }
        }
        if serial_tail.len() > 4096 {
            serial_tail = serial_tail.split_off(serial_tail.len() - 1024);
        }

        login = match login {
            Login::WaitingForLogin { user, password } if serial_tail.ends_with("login: ") => {
                to_send.extend_from_slice(format!("{user}\n").as_bytes());
                serial_tail.clear();
                Login::WaitingForPassword { password }
            }
            Login::WaitingForPassword { password } if serial_tail.ends_with("assword: ") => {
                to_send.extend_from_slice(format!("{password}\n").as_bytes());
                serial_tail.clear();
                Login::Commands
            }
            Login::Commands
                if to_send.is_empty()
                    && (serial_tail.ends_with("# ") || serial_tail.ends_with("$ ")) =>
            {
                if let Some(command) = commands.next() {
                    println!("[{at:8.2}s] typing: {command}");
                    to_send.extend_from_slice(format!("{command}\n").as_bytes());
                    serial_tail.clear();
                }
                Login::Commands
            }
            unchanged => unchanged,
        };
        if !to_send.is_empty() {
            let taken = machine.serial(0).map_or(0, |mut port| port.send(&to_send));
            to_send.drain(..taken);
        }

        if outcome.is_terminal() {
            println!("[{at:8.2}s] machine stopped: {:?}", outcome.stop);
            break;
        }
        if at >= options.seconds {
            break;
        }
    }

    let watch = machine.instrumentation();
    println!(
        "done after {:.0}s of host time; DAC writes {} ({} zero), start-address writes {}, \
         start address {:#06x}",
        started.elapsed().as_secs_f64(),
        watch.dac_writes,
        watch.dac_zero_writes,
        watch.start_address_writes,
        u16::from_be_bytes(watch.start_address)
    );
    Ok(())
}
