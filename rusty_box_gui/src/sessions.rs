//! The VMs the shell has shown: a session per VM, with its own display and
//! console, which outlives each of its runs; and what only one running VM
//! may have at a time, held for a power-on's runs and given back after them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use rusty_box::gui::shared_display::SharedDisplay;
use rusty_box::gui::RustyBoxApp;

use crate::app::{status_snapshot, ShellStatus, VmOrigin};
use crate::config::Engine;

/// What only one running VM may have at a time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Exclusive {
    /// The Windows Hypervisor Platform: a process holds one partition at a
    /// time (rusty_box_whp `a_process_holds_one_partition_at_a_time`).
    Hypervisor,
    /// A hard-disk image, by its canonical path: two writers would corrupt it.
    Disk(PathBuf),
}

/// The VM a hold belongs to, by the name the shell shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeldBy {
    pub(crate) name: String,
}

/// Which VM holds what. Shared by the shell, which takes, and the run
/// threads, which give back.
#[derive(Clone, Default)]
pub(crate) struct Holdings(Arc<Mutex<HashMap<Exclusive, HeldBy>>>);

impl Holdings {
    /// Takes `what` for the VM named `by`, or says who has it. A lock a
    /// panicking thread poisoned still holds a whole map, since every
    /// change to it is one insert or one remove, so it is used as it is.
    pub(crate) fn take(&self, what: Exclusive, by: HeldBy) -> Result<Hold, HeldBy> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(holder) = held.get(&what) {
            return Err(holder.clone());
        }
        // `what` was free a line ago, under the same lock: nothing is replaced.
        drop(held.insert(what.clone(), by));
        Ok(Hold {
            holdings: self.clone(),
            what,
        })
    }
}

/// `what`, held for one VM's runs; given back when dropped.
pub(crate) struct Hold {
    holdings: Holdings,
    what: Exclusive,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut held = self
            .holdings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        drop(held.remove(&self.what));
    }
}

/// What a power-on holds for as long as its runs last.
#[derive(Default)]
pub(crate) struct RunHolds {
    pub(crate) hypervisor: Option<Hold>,
    pub(crate) disk: Option<Hold>,
}

/// A VM's screen and console: what its runs draw into and read their input
/// from. It outlives each run, so a powered-off console keeps its last frame
/// and its serial log.
pub(crate) struct VmSession {
    display: Arc<Mutex<SharedDisplay>>,
    console: RustyBoxApp,
    /// The engine the VM's last power-on runs on: the one it is set to, or
    /// the interpreter when another VM held the hypervisor then. `None`
    /// until the VM is first powered on.
    run_engine: Option<Engine>,
}

impl VmSession {
    pub(crate) fn new() -> Self {
        let display = Arc::new(Mutex::new(SharedDisplay::new()));
        let console = RustyBoxApp::new_embedded(Arc::clone(&display));
        #[cfg(target_os = "android")]
        let console = {
            let mut console = console;
            console.set_display_scale(rusty_box::gui::DisplayScale::Fit);
            // A finger is not a mouse: on a phone the guest's image is a
            // trackpad with its own left and right buttons.
            console.set_pointer_mode(rusty_box::gui::PointerMode::Touchpad);
            console
        };
        Self {
            display,
            console,
            run_engine: None,
        }
    }

    pub(crate) fn display(&self) -> &Arc<Mutex<SharedDisplay>> {
        &self.display
    }

    pub(crate) fn console(&mut self) -> &mut RustyBoxApp {
        &mut self.console
    }

    pub(crate) fn mouse_captured(&self) -> bool {
        self.console.mouse_captured()
    }

    pub(crate) fn status(&self) -> ShellStatus {
        status_snapshot(&self.display)
    }

    /// The engine the VM's live run uses: what its last power-on recorded,
    /// which is the run that is live whenever the VM is (`is_live`).
    pub(crate) fn run_engine(&self) -> Option<Engine> {
        self.run_engine
    }

    /// Records the engine the power-on being sent runs on.
    pub(crate) fn set_run_engine(&mut self, engine: Engine) {
        self.run_engine = Some(engine);
    }

    /// Whether the VM runs or is about to.
    pub(crate) fn is_live(&self) -> bool {
        let status = self.status();
        status.running || status.start_pending
    }
}

/// A run's error, with the VM it belongs to.
pub(crate) struct RuntimeError {
    pub(crate) vm: VmOrigin,
    pub(crate) message: String,
}

/// Every VM's session, by VM.
#[derive(Default)]
pub(crate) struct Sessions {
    by_vm: HashMap<VmOrigin, VmSession>,
}

impl Sessions {
    pub(crate) fn get(&self, vm: &VmOrigin) -> Option<&VmSession> {
        self.by_vm.get(vm)
    }

    /// `vm`'s session, opened on first use.
    pub(crate) fn open(&mut self, vm: &VmOrigin) -> &mut VmSession {
        self.by_vm.entry(vm.clone()).or_insert_with(VmSession::new)
    }

    /// Whether `vm` runs or is about to.
    pub(crate) fn is_live(&self, vm: &VmOrigin) -> bool {
        self.get(vm).is_some_and(VmSession::is_live)
    }

    /// Every session's state, in no particular order.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn statuses(&self) -> Vec<ShellStatus> {
        self.by_vm.values().map(VmSession::status).collect()
    }

    /// Moves `from`'s session to `to`: the launch VM kept in the library.
    pub(crate) fn rekey(&mut self, from: &VmOrigin, to: VmOrigin) {
        if let Some(session) = self.by_vm.remove(from) {
            drop(self.by_vm.insert(to, session));
        }
    }

    /// Forgets `vm`'s session: the VM was deleted.
    pub(crate) fn close(&mut self, vm: &VmOrigin) {
        drop(self.by_vm.remove(vm));
    }

    /// Asks every VM's run to stop; each thread ends on its own.
    pub(crate) fn stop_all(&self) {
        for session in self.by_vm.values() {
            crate::runner::signal_egui_stop(&session.display);
        }
    }

    /// Takes every run error the displays hold.
    pub(crate) fn take_runtime_errors(&self) -> Vec<RuntimeError> {
        let mut errors = Vec::new();
        for (vm, session) in &self.by_vm {
            let message = session
                .display
                .lock()
                .ok()
                .and_then(|mut display| display.runtime_error.take());
            if let Some(message) = message {
                errors.push(RuntimeError {
                    vm: vm.clone(),
                    message,
                });
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_refused_to_a_second_vm_and_given_back_when_dropped() {
        let holdings = Holdings::default();
        let alpine = HeldBy {
            name: "Alpine".to_owned(),
        };
        let xp = HeldBy {
            name: "Windows XP".to_owned(),
        };
        let held = holdings
            .take(Exclusive::Hypervisor, alpine.clone())
            .expect("free");
        assert_eq!(
            holdings.take(Exclusive::Hypervisor, xp.clone()).err(),
            Some(alpine)
        );
        drop(held);
        assert!(holdings.take(Exclusive::Hypervisor, xp).is_ok());
    }

    #[test]
    fn two_disks_are_held_apart() {
        let holdings = Holdings::default();
        let first = holdings.take(
            Exclusive::Disk(PathBuf::from("a.img")),
            HeldBy {
                name: "A".to_owned(),
            },
        );
        let second = holdings.take(
            Exclusive::Disk(PathBuf::from("b.img")),
            HeldBy {
                name: "B".to_owned(),
            },
        );
        assert!(first.is_ok() && second.is_ok());
    }
}
