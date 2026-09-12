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
    Images,
}

impl ShellPage {
    /// The pages a VM node lists, in the order the tree draws them.
    pub(crate) const ALL: [Self; 4] = [Self::Home, Self::Console, Self::Hardware, Self::Images];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Home => "Summary",
            Self::Console => "Console",
            Self::Hardware => "Hardware",
            Self::Images => "Images",
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

/// What a click in the sidebar asked for. The sidebar reports it; the app
/// decides what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarAction {
    Select(Destination),
    /// Add a VM to the library, copied from the selected one.
    NewVm,
    /// Delete the library file at this index of the "Could not load" group.
    DeleteBroken(usize),
}

/// What a click in the VM bar asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VmBarAction {
    ToggleSidebar,
    PowerOn,
    PowerOff,
    Restart,
    ToggleSerial,
    ToggleMouseCapture,
    SendCtrlAltDel,
    ShowAbout,
    Quit,
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
            at.select_page(ShellPage::Images),
            Destination::new(2, ShellPage::Images)
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
            ["Summary", "Console", "Hardware", "Images"]
        );
    }
}
