//! Where the shell is pointed, and what its two chrome surfaces ask for.

/// One of a VM's pages. The desktop tree labels `Home` "Summary" because it
/// summarises the selected VM; the browser shell's home page is a genuine home
/// — the place an ISO is uploaded — so the variant keeps the name that is true
/// in both shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellPage {
    Home,
    Console,
    Hardware,
    /// A paused VM's processors, code, memory and stack.
    #[cfg(not(target_arch = "wasm32"))]
    Debugger,
}

impl ShellPage {
    /// The pages a VM node lists, in the order the tree draws them.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const ALL: [Self; 4] = [Self::Home, Self::Console, Self::Hardware, Self::Debugger];

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Home => "Summary",
            Self::Console => "Console",
            Self::Hardware => "Hardware",
            Self::Debugger => "Debugger",
        }
    }
}

/// Which VM profile the shell shows, and which of its pages. One value, so the
/// pair cannot drift apart the way two independent fields can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Destination {
    vm: usize,
    page: ShellPage,
}

impl Destination {
    pub(crate) const fn new(vm: usize, page: ShellPage) -> Self {
        Self { vm, page }
    }

    pub(crate) const fn vm(self) -> usize {
        self.vm
    }

    pub(crate) const fn page(self) -> ShellPage {
        self.page
    }

    /// Moving to a different VM lands on its summary; re-selecting the VM
    /// already shown leaves the page where the user put it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn select_vm(self, vm: usize) -> Self {
        if vm == self.vm {
            self
        } else {
            Self {
                vm,
                page: ShellPage::Home,
            }
        }
    }

    pub(crate) fn select_page(self, page: ShellPage) -> Self {
        Self { page, ..self }
    }

    /// The destination after `removed` is deleted from a library that then
    /// holds `remaining` profiles, clamped so it always names a live one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn clamped_after_removal(self, removed: usize, remaining: usize) -> Self {
        let vm = if self.vm > removed { self.vm - 1 } else { self.vm };
        Self {
            vm: vm.min(remaining.saturating_sub(1)),
            page: self.page,
        }
    }
}

impl Default for Destination {
    fn default() -> Self {
        Self::new(0, ShellPage::Home)
    }
}

/// What fills the shell's central area: a page of the shown VM, or the
/// app's own Settings, which belong to no VM.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Surface {
    #[default]
    Vm,
    Settings,
}

/// What a click in the sidebar asked for. The sidebar reports it; the app
/// decides what it means.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarAction {
    Select(Destination),
    /// Add a VM to the library, copied from the selected one.
    NewVm,
    /// Delete the library file at this index of the "Could not load" group.
    DeleteBroken(usize),
    /// Show the app's Settings.
    OpenSettings,
}

/// What a click in the VM bar asked for.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VmBarAction {
    ToggleSidebar,
    PowerOn,
    PowerOff,
    Restart,
    /// Pause a running VM, or let a paused one run on.
    TogglePause,
    /// Save the VM's RAM to a file.
    DumpRam,
    ToggleSerial,
    ToggleMouseCapture,
    SendCtrlAltDel,
    /// Open the floppy maker.
    CreateFloppy,
    /// Show the app's Settings.
    OpenSettings,
    ShowAbout,
    Quit,
    /// A phone's page tab: the bar carries the pages a desktop's tree lists.
    #[cfg(target_os = "android")]
    GoTo(ShellPage),
}

#[cfg(test)]
mod tests {
    use super::{Destination, ShellPage};

    #[test]
    fn selecting_another_vm_lands_on_its_summary() {
        let at = Destination::new(0, ShellPage::Hardware);
        assert_eq!(at.select_vm(2), Destination::new(2, ShellPage::Home));
    }

    #[test]
    fn reselecting_the_shown_vm_keeps_the_page() {
        let at = Destination::new(1, ShellPage::Console);
        assert_eq!(at.select_vm(1), at);
    }

    #[test]
    fn selecting_a_page_keeps_the_vm() {
        let at = Destination::new(2, ShellPage::Home);
        assert_eq!(
            at.select_page(ShellPage::Hardware),
            Destination::new(2, ShellPage::Hardware)
        );
    }

    #[test]
    fn removing_an_earlier_profile_shifts_the_selection_down() {
        let at = Destination::new(2, ShellPage::Console);
        assert_eq!(
            at.clamped_after_removal(0, 2),
            Destination::new(1, ShellPage::Console)
        );
    }

    #[test]
    fn removing_the_selected_profile_clamps_to_the_last_remaining() {
        let at = Destination::new(2, ShellPage::Console);
        assert_eq!(
            at.clamped_after_removal(2, 2),
            Destination::new(1, ShellPage::Console)
        );
    }

    #[test]
    fn removing_the_only_other_profile_leaves_the_first() {
        let at = Destination::new(1, ShellPage::Hardware);
        assert_eq!(
            at.clamped_after_removal(1, 1),
            Destination::new(0, ShellPage::Hardware)
        );
    }

    #[test]
    fn every_page_the_tree_lists_carries_a_label() {
        assert_eq!(
            ShellPage::ALL.map(ShellPage::label),
            ["Summary", "Console", "Hardware", "Debugger"]
        );
    }
}
