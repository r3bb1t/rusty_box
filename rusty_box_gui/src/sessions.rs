//! The VMs the shell has shown: a session per VM, with its own display and
//! console, which outlives each of its runs; and what only one running VM
//! may have at a time, held for a power-on's runs and given back after them.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use rusty_box::gui::shared_display::SharedDisplay;
use rusty_box::gui::RustyBoxApp;

use crate::app::{status_snapshot, ShellStatus, VmOrigin};
use crate::config::Engine;
use crate::debugger::control::{DumpOutcome, DumpRequest, RunControl, RunPhase};

/// What one running VM at a time may have, or, for a disk, several VMs that
/// only read it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Exclusive {
    /// The Windows Hypervisor Platform: a process holds one partition at a
    /// time (rusty_box_whp `a_process_holds_one_partition_at_a_time`).
    Hypervisor,
    /// A hard-disk image, by its canonical path: two writers would corrupt
    /// it, and a reader would see it change under it.
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
pub(crate) struct Holdings(Arc<Mutex<Held>>);

#[derive(Default)]
struct Held {
    holders: HashMap<Exclusive, Holders>,
    /// The number the next hold is known by.
    next: u64,
}

/// Who holds one thing.
enum Holders {
    /// One VM, beside which no other may hold it.
    Alone(Holder),
    /// The VMs that only read it, in the order they took it. The list goes
    /// with its last reader, so it is never empty.
    Readers(Vec<Holder>),
}

/// One hold, by the number its [`Hold`] gives it back by.
struct Holder {
    hold: u64,
    by: HeldBy,
}

/// How a hold shares what it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claim {
    Alone,
    /// Beside other readers and no writer: QEMU's image locking, where a
    /// read-only drive shares `BLK_PERM_CONSISTENT_READ` and refuses
    /// `BLK_PERM_WRITE` (hw/block/block.c `blkconf_apply_backend_options`).
    Reading,
}

impl Holdings {
    /// Takes `what` for the VM named `by` alone, or says who has it.
    pub(crate) fn take(&self, what: Exclusive, by: HeldBy) -> Result<Hold, HeldBy> {
        self.claim(what, by, Claim::Alone)
    }

    /// Takes `what` for the VM named `by`, which only reads it, beside the
    /// other VMs that only read it; or says which VM holds it alone.
    pub(crate) fn share(&self, what: Exclusive, by: HeldBy) -> Result<Hold, HeldBy> {
        self.claim(what, by, Claim::Reading)
    }

    /// A lock a panicking thread poisoned still holds a whole map, since
    /// every change to it is made whole under the lock, so it is used as it
    /// is. A refusal names the VM that holds `what` alone, or, to a VM that
    /// would hold it alone, the first of its readers.
    fn claim(&self, what: Exclusive, by: HeldBy, claim: Claim) -> Result<Hold, HeldBy> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let hold = held.next;
        held.next = hold.wrapping_add(1);
        let holder = Holder { hold, by };
        match (held.holders.entry(what.clone()), claim) {
            (Entry::Vacant(free), Claim::Alone) => {
                free.insert(Holders::Alone(holder));
            }
            (Entry::Vacant(free), Claim::Reading) => {
                free.insert(Holders::Readers(vec![holder]));
            }
            (Entry::Occupied(taken), claim) => {
                let holders = taken.into_mut();
                match (&mut *holders, claim) {
                    (Holders::Alone(owner), Claim::Alone | Claim::Reading) => return Err(owner.by.clone()),
                    (Holders::Readers(readers), Claim::Reading) => readers.push(holder),
                    (Holders::Readers(readers), Claim::Alone) => match readers.first() {
                        Some(reader) => return Err(reader.by.clone()),
                        // A list with no reader holds nothing.
                        None => *holders = Holders::Alone(holder),
                    },
                }
            }
        }
        Ok(Hold {
            holdings: self.clone(),
            what,
            hold,
        })
    }
}

/// `what`, held for one VM's runs; given back when dropped.
pub(crate) struct Hold {
    holdings: Holdings,
    what: Exclusive,
    hold: u64,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut held = self
            .holdings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let gone = match held.holders.get_mut(&self.what) {
            None => return,
            Some(Holders::Alone(owner)) => owner.hold == self.hold,
            Some(Holders::Readers(readers)) => {
                readers.retain(|reader| reader.hold != self.hold);
                readers.is_empty()
            }
        };
        if gone {
            drop(held.holders.remove(&self.what));
        }
    }
}

/// What a power-on holds for as long as its runs last.
pub(crate) struct RunHolds {
    pub(crate) hypervisor: Option<Hold>,
    pub(crate) disk: Option<Hold>,
    /// The app is kept running while the power-on lasts.
    pub(crate) alive: AliveHold,
}

/// What keeping the app running takes on this platform.
#[derive(Clone)]
pub(crate) enum AliveEffect {
    /// A desktop app keeps running whatever its screen does.
    Nothing,
    /// A phone runs `VmService`, a foreground service with a wake lock, while
    /// any VM runs: without one Android puts the app to sleep and closes it
    /// once the screen has been off for a while (src/android.rs).
    #[cfg(target_os = "android")]
    Phone(crate::android::AndroidApp),
    /// Each start and stop, in order, for a test to read.
    #[cfg(test)]
    Record(Arc<Mutex<Vec<AliveChange>>>),
}

/// A change of the platform's keep-alive, as a test records it.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AliveChange {
    Started,
    Stopped,
}

impl AliveEffect {
    fn start(&self) {
        match self {
            Self::Nothing => {}
            #[cfg(target_os = "android")]
            Self::Phone(app) => crate::android::keep_alive(app),
            #[cfg(test)]
            Self::Record(changes) => changes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(AliveChange::Started),
        }
    }

    fn stop(&self) {
        match self {
            Self::Nothing => {}
            #[cfg(target_os = "android")]
            Self::Phone(app) => crate::android::let_go(app),
            #[cfg(test)]
            Self::Record(changes) => changes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(AliveChange::Stopped),
        }
    }
}

/// Keeps the app running while any power-on lasts: the first hold taken
/// starts the platform's keep-alive and the last one given back stops it,
/// the one place either happens (R5). Shared by the shell, which takes a
/// hold for each power-on it sends, and the run threads, which give it back
/// once the power-on's last machine is gone — with the app in the
/// background as much as in front.
#[derive(Clone)]
pub(crate) struct KeepAlive(Arc<Mutex<Keeping>>);

struct Keeping {
    /// The power-ons holding the app alive.
    holds: usize,
    effect: AliveEffect,
}

impl KeepAlive {
    pub(crate) fn new(effect: AliveEffect) -> Self {
        Self(Arc::new(Mutex::new(Keeping { holds: 0, effect })))
    }

    /// Keeps the app running until the hold is dropped. A lock a panicking
    /// thread poisoned still holds a whole count, since every change to it
    /// is one step up or down, so it is used as it is.
    pub(crate) fn take(&self) -> AliveHold {
        let mut keeping = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        keeping.holds += 1;
        if keeping.holds == 1 {
            keeping.effect.start();
        }
        AliveHold {
            keep_alive: self.clone(),
        }
    }
}

/// One power-on keeping the app running; given back when dropped.
pub(crate) struct AliveHold {
    keep_alive: KeepAlive,
}

impl Drop for AliveHold {
    fn drop(&mut self) {
        let mut keeping = self
            .keep_alive
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Every hold was counted once when it was taken.
        keeping.holds = keeping.holds.saturating_sub(1);
        if keeping.holds == 0 {
            keeping.effect.stop();
        }
    }
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
    /// The only writer of the run's stop flag (`RunControl`).
    control: Arc<RunControl>,
    /// What the Debugger page keeps for this VM between frames.
    debugger: crate::shell::debugger_page::DebuggerPane,
}

impl VmSession {
    pub(crate) fn new() -> Self {
        let shown = SharedDisplay::new();
        let control = Arc::new(RunControl::new(Arc::clone(&shown.stop_flag)));
        let display = Arc::new(Mutex::new(shown));
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
            control,
            debugger: crate::shell::debugger_page::DebuggerPane::default(),
        }
    }

    pub(crate) fn debugger_mut(&mut self) -> &mut crate::shell::debugger_page::DebuggerPane {
        &mut self.debugger
    }

    pub(crate) fn display(&self) -> &Arc<Mutex<SharedDisplay>> {
        &self.display
    }

    pub(crate) fn control(&self) -> &Arc<RunControl> {
        &self.control
    }

    /// Power off: the run ends, paused or not, and does not begin again.
    pub(crate) fn power_off(&self) {
        if let Ok(mut display) = self.display.lock() {
            display.reset_requested = false;
        }
        self.control.end_run();
    }

    /// Restart: the run ends, paused or not, and begins again. The restart is
    /// recorded before the run is told to end, so the run reads it as it ends.
    pub(crate) fn restart(&self) {
        if let Ok(mut display) = self.display.lock() {
            display.reset_requested = true;
        }
        self.control.end_run();
    }

    pub(crate) fn request_dump(&self, path: PathBuf) -> DumpRequest {
        self.control.request_dump(path)
    }

    pub(crate) fn console(&mut self) -> &mut RustyBoxApp {
        &mut self.console
    }

    pub(crate) fn mouse_captured(&self) -> bool {
        self.console.mouse_captured()
    }

    pub(crate) fn status(&self) -> ShellStatus {
        ShellStatus {
            paused: self.control.phase() == RunPhase::Paused,
            ..status_snapshot(&self.display)
        }
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

/// A finished RAM dump, with the VM it belongs to.
pub(crate) struct VmDumpOutcome {
    pub(crate) vm: VmOrigin,
    pub(crate) outcome: DumpOutcome,
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

    /// The VMs that run or are about to, in no particular order.
    pub(crate) fn live_vms(&self) -> Vec<VmOrigin> {
        self.by_vm
            .iter()
            .filter(|(_, session)| session.is_live())
            .map(|(vm, _)| vm.clone())
            .collect()
    }

    /// Whether any VM runs or is about to.
    pub(crate) fn any_live(&self) -> bool {
        self.by_vm.values().any(VmSession::is_live)
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
            crate::runner::signal_egui_stop(&session.display, &session.control);
        }
    }

    /// Takes every finished dump the runs recorded.
    pub(crate) fn take_dump_outcomes(&self) -> Vec<VmDumpOutcome> {
        let mut done = Vec::new();
        for (vm, session) in &self.by_vm {
            for outcome in session.control.take_dump_outcomes() {
                done.push(VmDumpOutcome {
                    vm: vm.clone(),
                    outcome,
                });
            }
        }
        done
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

    /// A phone's foreground service runs from the first VM's power-on until
    /// the last VM is off, and starts again with the next power-on.
    #[test]
    fn the_app_is_kept_alive_from_the_first_power_on_until_the_last_is_over() {
        let changes = Arc::new(Mutex::new(Vec::new()));
        let keep_alive = KeepAlive::new(AliveEffect::Record(Arc::clone(&changes)));
        let recorded = || changes.lock().unwrap().clone();

        let alpine = keep_alive.take();
        let xp = keep_alive.take();
        assert_eq!(recorded(), [AliveChange::Started], "one start for two VMs");
        drop(alpine);
        assert_eq!(recorded(), [AliveChange::Started], "Windows XP still runs");
        drop(xp);
        assert_eq!(recorded(), [AliveChange::Started, AliveChange::Stopped]);

        drop(keep_alive.take());
        assert_eq!(
            recorded(),
            [
                AliveChange::Started,
                AliveChange::Stopped,
                AliveChange::Started,
                AliveChange::Stopped,
            ]
        );
    }

    /// VMs that only read a disk hold it together; one that writes it holds
    /// it alone, refused while any reader has it and refusing them while it
    /// does. Each hold gives back only its own share.
    #[test]
    fn readers_share_a_disk_that_a_writer_holds_alone() {
        let holdings = Holdings::default();
        let disk = || Exclusive::Disk(PathBuf::from("base.img"));
        let vm = |name: &str| HeldBy {
            name: name.to_owned(),
        };

        let first = holdings.share(disk(), vm("First reader")).expect("free");
        let second = holdings.share(disk(), vm("Second reader")).expect("readers share");
        assert_eq!(
            holdings.take(disk(), vm("Writer")).err(),
            Some(vm("First reader")),
            "a writer is refused, naming the first reader"
        );
        drop(first);
        assert_eq!(
            holdings.take(disk(), vm("Writer")).err(),
            Some(vm("Second reader")),
            "one reader's going leaves the other's hold"
        );
        drop(second);

        let writer = holdings.take(disk(), vm("Writer")).expect("free once every reader is gone");
        assert_eq!(holdings.share(disk(), vm("Reader")).err(), Some(vm("Writer")));
        drop(writer);
        assert!(holdings.share(disk(), vm("Reader")).is_ok());
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
