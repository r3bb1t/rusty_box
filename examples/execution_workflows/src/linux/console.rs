use std::error::Error;

use rusty_box::Emulator;
use rusty_box::emulator::{EmulatorConfig, Ips, MachineBuilder, MemorySize, RunBudget, StopReason};
use rusty_box::gui::NoGui;

pub type ConsoleResult<T> = Result<T, Box<dyn Error>>;

const TICKS: u64 = 2_000_000;

/// A disposable direct-boot guest with bounded COM1 input and output.
pub struct Console {
    machine: Box<Emulator>,
    output: Vec<u8>,
}

impl Console {
    pub fn boot(kernel: &[u8], initramfs: &[u8]) -> ConsoleResult<Self> {
        let config = EmulatorConfig {
            memory: MemorySize::bytes(128 * 1024 * 1024),
            ips: Ips::new(300_000_000),
            ..Default::default()
        };
        let mut machine = MachineBuilder::new(config).gui(NoGui::new()).build()?;
        machine.setup_direct_linux_boot(kernel, Some(initramfs),
            "console=ttyS0,115200 earlycon=uart8250,io,0x3f8,115200n8 nokaslr nomodeset quiet rdinit=/init")?;
        let mut console = Self {
            machine,
            output: Vec::new(),
        };
        let version = console.wait_for("guest ready", 6_000, |line| {
            line.strip_prefix("RB_READY:")
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })?;
        println!("Guest ready on COM1, Linux {version}");
        Ok(console)
    }

    pub fn machine(&mut self) -> &mut Emulator {
        &mut self.machine
    }

    pub fn pump(&mut self) -> ConsoleResult<StopReason> {
        let stop = self.machine.step(RunBudget::Ticks(TICKS))?.stop;
        self.output.extend(
            self.machine
                .serial(0)
                .ok_or("guest has no COM1")?
                .take_output(),
        );
        if self.output.len() > 64 * 1024 {
            return Err("serial transcript exceeded 64 KiB".into());
        }
        Ok(stop)
    }

    fn pump_ordinary(&mut self) -> ConsoleResult<()> {
        match self.pump()? {
            StopReason::BudgetExhausted | StopReason::Halted => Ok(()),
            stop => Err(format!("unexpected console stop: {stop:?}").into()),
        }
    }

    pub fn command(&mut self, command: &str) -> ConsoleResult<()> {
        let line = format!("{command}\n");
        let mut rest = line.as_bytes();
        for _ in 0..80 {
            let sent = self
                .machine
                .serial(0)
                .ok_or("guest has no COM1")?
                .send(rest);
            rest = &rest[sent..];
            if rest.is_empty() {
                return Ok(());
            }
            self.pump_ordinary()?;
        }
        Err("guest console did not accept the complete command".into())
    }

    pub fn clear_output(&mut self) {
        self.output.clear();
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.output
            .split_inclusive(|&byte| byte == b'\n')
            .filter_map(|line| {
                if !line.ends_with(b"\n") {
                    return None;
                }
                std::str::from_utf8(line).ok().map(str::trim_end)
            })
    }

    pub fn tail(&self) -> String {
        let first = self.output.len().saturating_sub(1600);
        String::from_utf8_lossy(&self.output[first..]).into_owned()
    }

    pub fn wait_for<R, F: FnMut(&str) -> Option<R>>(
        &mut self,
        label: &str,
        steps: usize,
        mut parse: F,
    ) -> ConsoleResult<R> {
        for _ in 0..steps {
            if let Some(result) = self.lines().find_map(&mut parse) {
                return Ok(result);
            }
            self.pump_ordinary()?;
        }
        Err(format!(
            "missing {label} at guest IP {:#x}: {:?}",
            self.machine.rip(),
            self.tail()
        )
        .into())
    }

    pub fn wait_line(&mut self, line: &str) -> ConsoleResult<()> {
        self.wait_for(line, 350, |text| (text == line).then_some(()))
    }
}
