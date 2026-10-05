//! The VMs the shell has shown: a session per VM, with its own display and
//! console, which outlives each of its runs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusty_box::gui::shared_display::SharedDisplay;
use rusty_box::gui::RustyBoxApp;

use crate::app::{status_snapshot, ShellStatus, VmOrigin};

/// A VM's screen and console: what its runs draw into and read their input
/// from. It outlives each run, so a powered-off console keeps its last frame
/// and its serial log.
pub(crate) struct VmSession {
    display: Arc<Mutex<SharedDisplay>>,
    console: RustyBoxApp,
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
        Self { display, console }
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
