//! Which modules of a pause the engine is asked for, and how urgently: the
//! kernel first, since its layouts drive the walks; then the modules the
//! panes show an address in; then the rest. A module whose headers did not
//! read has no identity and is not asked for: its row says why.

use crate::debugger::inspect::{AddressSpace, DebugSnapshot, SymbolIndex};

use super::hub::{Priority, WantedModule};

/// The linear addresses the panes of `snapshot` show: the code pane's first
/// line and the memory window's start, when it reads linear memory.
pub(crate) fn shown_addresses(snapshot: &DebugSnapshot) -> Vec<u64> {
    let mut shown: Vec<u64> = snapshot.code.first().map(|line| line.address).into_iter().collect();
    match snapshot.memory.start.space {
        AddressSpace::Linear | AddressSpace::Process { .. } => shown.push(snapshot.memory.start.value),
        AddressSpace::Physical => {}
    }
    shown
}

/// The modules of `symbols` with an identity, each once, with the priority
/// it is looked up at. `shown` holds the addresses the panes show.
pub(crate) fn wanted_modules(symbols: &SymbolIndex, shown: &[u64]) -> Vec<WantedModule> {
    let mut wanted: Vec<WantedModule> = Vec::new();
    for module in &symbols.modules {
        let Some(identity) = module.identity.known() else {
            continue;
        };
        // The difference before the width test keeps a module near the top
        // of memory from wrapping.
        let contains = |address: u64| address.checked_sub(module.base).is_some_and(|offset| offset < module.size);
        let priority = if symbols.kernel == Some(module.base) {
            Priority::Kernel
        } else if shown.iter().any(|&address| contains(address)) {
            Priority::Shown
        } else {
            Priority::Rest
        };
        match wanted.iter_mut().find(|earlier| earlier.identity == *identity) {
            Some(earlier) => earlier.priority = earlier.priority.min(priority),
            None => wanted.push(WantedModule {
                identity: identity.clone(),
                priority,
            }),
        }
    }
    wanted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::fake_windows::{
        FakeWindows, FakeWindowsSpec, DEBUG_DIRECTORY_RVA, LIVE_CR3,
    };
    use crate::debugger::windows::{read_windows, SystemView, WindowsDiscovery};
    use rusty_box_symbols::identity::ModuleIdentity;

    fn symbols_of(fake: &mut FakeWindows) -> SymbolIndex {
        let view = fake.view;
        read_windows(&mut fake.guest, &view, SystemView::Off, &mut WindowsDiscovery::default(), None)
            .symbols
    }

    #[test]
    fn the_kernel_is_wanted_first_and_a_shown_module_before_the_rest() {
        let spec = FakeWindowsSpec::typical();
        let driver = spec.drivers[0].base;
        let mut fake = FakeWindows::build(spec);
        let symbols = symbols_of(&mut fake);
        let wanted = wanted_modules(&symbols, &[driver + 0x1234]);
        let names: Vec<&str> = wanted.iter().map(|w| w.identity.file_name()).collect();
        assert_eq!(names, ["ntoskrnl.exe", "disk.sys"]);
        assert_eq!(wanted[0].priority, Priority::Kernel);
        assert_eq!(wanted[1].priority, Priority::Shown);
        assert_eq!(wanted_modules(&symbols, &[])[1].priority, Priority::Rest);
    }

    /// Review focus 4: a driver whose CodeView record is paged out is still
    /// asked for, by its image identity, so the engine can learn its PDB
    /// from its binary.
    #[test]
    fn a_module_whose_debug_directory_is_paged_out_is_still_wanted() {
        let spec = FakeWindowsSpec::typical();
        let driver = spec.drivers[0].clone();
        let mut fake = FakeWindows::build(spec);
        fake.guest.page_out(LIVE_CR3, driver.base + DEBUG_DIRECTORY_RVA);
        let wanted = wanted_modules(&symbols_of(&mut fake), &[]);
        let disk = wanted
            .iter()
            .find(|w| w.identity.file_name() == "disk.sys")
            .expect("disk.sys is wanted");
        let ModuleIdentity::Pe { image, pdb, .. } = &disk.identity else {
            panic!("a PE module: {:?}", disk.identity);
        };
        assert_eq!(image.time_date_stamp, driver.time_date_stamp);
        assert_eq!(image.size_of_image, driver.size);
        assert_eq!(*pdb, None);
        assert_eq!(disk.priority, Priority::Rest);
    }

    #[test]
    fn a_module_without_readable_headers_is_not_asked_for() {
        let spec = FakeWindowsSpec::typical();
        let driver = spec.drivers[0].base;
        let mut fake = FakeWindows::build(spec);
        fake.guest.page_out(LIVE_CR3, driver);
        let symbols = symbols_of(&mut fake);
        assert_eq!(symbols.modules.len(), 2, "the driver is still listed");
        let names: Vec<String> = wanted_modules(&symbols, &[])
            .iter()
            .map(|w| w.identity.file_name().to_owned())
            .collect();
        assert_eq!(names, ["ntoskrnl.exe"]);
    }
}
